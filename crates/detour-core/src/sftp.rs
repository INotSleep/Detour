//! Files on a host over SFTP: listing, transfers with progress and
//! cancelling, and the usual operations. A file browser shares its host's
//! kept SSH connection, on a channel of its own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

use crate::model::KnownHost;
use crate::ssh::{self, ConnectRequest, Session};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    File,
    Dir,
    Link,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    /// Unix ms.
    pub modified: Option<i64>,
    /// Unix mode bits (`0o755`), when the server says.
    pub permissions: Option<u32>,
}

struct Open {
    sftp: SftpSession,
    _session: Session,
}

/// Open file browsers by id, and transfers that can be cancelled.
#[derive(Default, Clone)]
pub struct Files {
    open: Arc<Mutex<HashMap<String, Arc<Open>>>>,
    transfers: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

/// Turns an SFTP error about `path` into ours.
fn err(e: SftpError, path: &str) -> Error {
    match &e {
        SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile => Error::NotFound { what: path.to_string() },
        SftpError::Status(s) if s.status_code == StatusCode::PermissionDenied => Error::Forbidden,
        SftpError::Status(s) => Error::Sftp { message: if s.error_message.is_empty() { s.status_code.to_string() } else { s.error_message.clone() } },
        SftpError::Timeout => Error::Sftp { message: "the server did not answer in time".into() },
        other => Error::Sftp { message: other.to_string() },
    }
}

/// `dir` joined with `name` in the server's syntax.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// Calls `report(done, total)` at most every 100 ms, and always at the end.
struct Progress<F: Fn(u64, u64)> {
    report: F,
    total: u64,
    done: u64,
    last: Instant,
}

impl<F: Fn(u64, u64)> Progress<F> {
    fn new(total: u64, report: F) -> Self {
        report(0, total);
        Self { report, total, done: 0, last: Instant::now() }
    }
    fn add(&mut self, n: usize) {
        self.done += n as u64;
        if self.last.elapsed() >= Duration::from_millis(100) {
            self.last = Instant::now();
            (self.report)(self.done, self.total.max(self.done));
        }
    }
    fn finish(&self) {
        (self.report)(self.done, self.total.max(self.done));
    }
}

const CHUNK: usize = 256 * 1024;

impl Files {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, id: &str) -> Result<Arc<Open>> {
        self.open.lock().unwrap().get(id).cloned().ok_or_else(|| Error::NotFound { what: "file browser".into() })
    }

    /// Opens a file browser on the host `req` logs in to. Returns its id, the
    /// home directory, and a server key to pin if it was new.
    pub async fn open(&self, req: &ConnectRequest) -> Result<(String, String, Option<KnownHost>)> {
        let (mut session, mut known) = ssh::session(req).await?;
        let stream = match session.subsystem("sftp").await {
            Ok(s) => s,
            // A kept connection the server closed in the meantime.
            Err(_) if known.is_none() => {
                ssh::forget_pooled(req);
                (session, known) = ssh::session(req).await?;
                session.subsystem("sftp").await?
            }
            Err(e) => return Err(e),
        };
        let sftp = SftpSession::new(stream).await.map_err(|e| match e {
            SftpError::IO(_) | SftpError::UnexpectedPacket => Error::Sftp { message: "the server has no SFTP".into() },
            other => err(other, ""),
        })?;
        sftp.set_timeout(30);
        let home = sftp.canonicalize(".").await.map_err(|e| err(e, "."))?;
        let id = crate::new_id();
        self.open.lock().unwrap().insert(id.clone(), Arc::new(Open { sftp, _session: session }));
        Ok((id, home, known))
    }

    pub fn close(&self, id: &str) {
        self.open.lock().unwrap().remove(id);
    }

    /// Closes every browser and cancels every transfer (when the vault locks).
    pub fn close_all(&self) {
        self.open.lock().unwrap().clear();
        for (_, t) in self.transfers.lock().unwrap().drain() {
            let _ = t.send(true);
        }
    }

    /// A directory's entries: folders first, then by name; without `.` and `..`.
    pub async fn list(&self, id: &str, path: &str) -> Result<Vec<FileEntry>> {
        let o = self.get(id)?;
        let dir = o.sftp.read_dir(path).await.map_err(|e| err(e, path))?;
        let mut out: Vec<FileEntry> = dir
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let m = e.metadata();
                let kind = if m.is_symlink() { FileKind::Link } else if m.is_dir() { FileKind::Dir } else { FileKind::File };
                FileEntry {
                    name: e.file_name(),
                    kind,
                    size: m.size.unwrap_or(0),
                    modified: m.mtime.map(|t| t as i64 * 1000),
                    permissions: m.permissions.map(|p| p & 0o7777),
                }
            })
            .collect();
        out.sort_by(|a, b| (a.kind != FileKind::Dir, a.name.to_lowercase()).cmp(&(b.kind != FileKind::Dir, b.name.to_lowercase())));
        Ok(out)
    }

    /// The absolute form of a path (`..`, `~`-less), e.g. to go up.
    pub async fn resolve(&self, id: &str, path: &str) -> Result<String> {
        let o = self.get(id)?;
        o.sftp.canonicalize(path).await.map_err(|e| err(e, path))
    }

    pub async fn mkdir(&self, id: &str, path: &str) -> Result<()> {
        self.get(id)?.sftp.create_dir(path).await.map_err(|e| err(e, path))
    }

    pub async fn rename(&self, id: &str, from: &str, to: &str) -> Result<()> {
        let o = self.get(id)?;
        if o.sftp.try_exists(to).await.unwrap_or(false) {
            return Err(Error::Sftp { message: format!("{to} already exists") });
        }
        o.sftp.rename(from, to).await.map_err(|e| err(e, from))
    }

    /// Removes a file, or a folder with everything in it.
    pub async fn remove(&self, id: &str, path: &str) -> Result<()> {
        let o = self.get(id)?;
        let meta = o.sftp.symlink_metadata(path).await.map_err(|e| err(e, path))?;
        if !meta.is_dir() || meta.is_symlink() {
            return o.sftp.remove_file(path).await.map_err(|e| err(e, path));
        }
        // Depth first, without following links.
        let mut stack = vec![(path.to_string(), false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                o.sftp.remove_dir(&dir).await.map_err(|e| err(e, &dir))?;
                continue;
            }
            stack.push((dir.clone(), true));
            for e in o.sftp.read_dir(&dir).await.map_err(|e| err(e, &dir))? {
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join(&dir, &name);
                let m = e.metadata();
                if m.is_dir() && !m.is_symlink() {
                    stack.push((child, false));
                } else {
                    o.sftp.remove_file(&child).await.map_err(|e| err(e, &child))?;
                }
            }
        }
        Ok(())
    }

    fn begin(&self, transfer: &str) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.transfers.lock().unwrap().insert(transfer.to_string(), tx);
        rx
    }

    fn end(&self, transfer: &str) {
        self.transfers.lock().unwrap().remove(transfer);
    }

    pub fn cancel(&self, transfer: &str) {
        if let Some(t) = self.transfers.lock().unwrap().remove(transfer) {
            let _ = t.send(true);
        }
    }

    /// Copies a remote file to `local`, written beside it first so a failed or
    /// cancelled download never leaves half a file under the real name.
    pub async fn download(&self, id: &str, remote: &str, local: &Path, transfer: &str, report: impl Fn(u64, u64)) -> Result<()> {
        let o = self.get(id)?;
        let cancelled = self.begin(transfer);
        let part = part_path(local);
        let result = async {
            let mut src = o.sftp.open(remote).await.map_err(|e| err(e, remote))?;
            let total = src.metadata().await.ok().and_then(|m| m.size).unwrap_or(0);
            let mut dst = tokio::fs::File::create(&part).await?;
            let mut progress = Progress::new(total, report);
            let mut buf = vec![0u8; CHUNK];
            loop {
                if *cancelled.borrow() {
                    return Err(Error::Cancelled);
                }
                let n = src.read(&mut buf).await.map_err(|e| Error::Sftp { message: e.to_string() })?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).await?;
                progress.add(n);
            }
            dst.flush().await?;
            dst.sync_all().await?;
            drop(dst);
            tokio::fs::rename(&part, local).await?;
            progress.finish();
            Ok(())
        }
        .await;
        self.end(transfer);
        if result.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result
    }

    /// Copies a local file to `remote`, replacing what is there. It is written
    /// beside it first: a failed or cancelled upload leaves the old file as it was.
    pub async fn upload(&self, id: &str, local: &Path, remote: &str, transfer: &str, report: impl Fn(u64, u64)) -> Result<()> {
        let o = self.get(id)?;
        let cancelled = self.begin(transfer);
        let part = format!("{remote}.part");
        let mut wrote = false;
        let result = async {
            let mut src = tokio::fs::File::open(local).await?;
            let total = src.metadata().await?.len();
            let mut dst = o.sftp.create(&part).await.map_err(|e| err(e, remote))?;
            wrote = true;
            let mut progress = Progress::new(total, report);
            let mut buf = vec![0u8; CHUNK];
            loop {
                if *cancelled.borrow() {
                    return Err(Error::Cancelled);
                }
                let n = src.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).await.map_err(|e| Error::Sftp { message: e.to_string() })?;
                progress.add(n);
            }
            dst.shutdown().await.map_err(|e| Error::Sftp { message: e.to_string() })?;
            drop(dst);
            // SFTP's rename does not replace a file.
            if o.sftp.try_exists(remote).await.unwrap_or(false) {
                o.sftp.remove_file(remote).await.map_err(|e| err(e, remote))?;
            }
            o.sftp.rename(&part, remote).await.map_err(|e| err(e, remote))?;
            progress.finish();
            Ok(())
        }
        .await;
        self.end(transfer);
        if result.is_err() && wrote {
            let _ = o.sftp.remove_file(&part).await;
        }
        result
    }
}

fn part_path(local: &Path) -> PathBuf {
    let mut name = local.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    local.with_file_name(name)
}

/// Opens a file browser on a host of a vault, logging in as a terminal would.
pub async fn open_in(store: &std::sync::Mutex<crate::store::Store>, files: &Files, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<(String, String)> {
    let poisoned = || Error::Internal { message: "state lock poisoned".into() };
    let req = store.lock().map_err(|_| poisoned())?.connect_request_in(vault, host_id, secret, trust_fingerprint)?;
    let (id, home, known) = files.open(&req).await?;
    store.lock().map_err(|_| poisoned())?.record_connection_in(vault, host_id, known)?;
    Ok((id, home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(part_path(Path::new("C:/x/report.pdf")), PathBuf::from("C:/x/report.pdf.part"));
    }
}
