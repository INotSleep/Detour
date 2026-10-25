//! Files on a host over SFTP: listing, transfers with progress and
//! cancelling, and the usual operations. A file browser shares its host's
//! kept SSH connection, on a channel of its own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, StatusCode};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

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
    pub async fn open(&self, req: &ConnectRequest) -> Result<(String, String, ssh::NewKeys)> {
        let (mut session, mut known) = ssh::session(req).await?;
        let stream = match session.subsystem("sftp").await {
            Ok(s) => s,
            // A kept connection the server closed in the meantime.
            Err(_) if known.is_empty() => {
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
        remove_tree(&self.get(id)?.sftp, path).await
    }

    /// Sets Unix permission bits (`0o755`).
    pub async fn chmod(&self, id: &str, path: &str, mode: u32) -> Result<()> {
        let mut attrs = FileAttributes::empty();
        attrs.permissions = Some(mode & 0o7777);
        self.get(id)?.sftp.set_metadata(path, attrs).await.map_err(|e| err(e, path))
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
            let total = o.sftp.metadata(remote).await.map_err(|e| err(e, remote))?.size.unwrap_or(0);
            let mut progress = Progress::new(total, report);
            copy_down(&o.sftp, remote, &part, &cancelled, &mut progress).await?;
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

    /// Downloads a file or a whole folder into `local_dir`, under its own name
    /// or the first free one like it. Links inside a folder are not followed.
    /// Returns where it went.
    pub async fn download_into(&self, id: &str, remote: &str, local_dir: &Path, transfer: &str, report: impl Fn(u64, u64)) -> Result<PathBuf> {
        let o = self.get(id)?;
        let name = remote.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("download");
        let target = free_name(local_dir, name);
        let meta = o.sftp.metadata(remote).await.map_err(|e| err(e, remote))?;
        if !meta.is_dir() {
            self.download(id, remote, &target, transfer, report).await?;
            return Ok(target);
        }
        let cancelled = self.begin(transfer);
        let part = part_path(&target);
        let result = async {
            let tree = remote_tree(&o.sftp, remote).await?;
            let mut progress = Progress::new(tree.size(), report);
            tokio::fs::create_dir_all(&part).await?;
            for d in &tree.dirs {
                tokio::fs::create_dir_all(part.join(d)).await?;
            }
            for (f, _) in &tree.files {
                copy_down(&o.sftp, &join(remote, f), &part.join(f), &cancelled, &mut progress).await?;
            }
            tokio::fs::rename(&part, &target).await?;
            progress.finish();
            Ok(())
        }
        .await;
        self.end(transfer);
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&part).await;
        }
        result.map(|_| target)
    }

    /// Uploads a local file or folder to `remote`. Without `replace`, something
    /// already there is an error. Everything is written beside it first: a failed
    /// or cancelled upload leaves what was there as it was.
    pub async fn upload(&self, id: &str, local: &Path, remote: &str, replace: bool, transfer: &str, report: impl Fn(u64, u64)) -> Result<()> {
        let o = self.get(id)?;
        if !replace && o.sftp.try_exists(remote).await.unwrap_or(false) {
            return Err(Error::Sftp { message: format!("{remote} already exists") });
        }
        let cancelled = self.begin(transfer);
        let part = format!("{remote}.part");
        let mut wrote = false;
        let result = async {
            if tokio::fs::metadata(local).await?.is_dir() {
                let tree = local_tree(local)?;
                let mut progress = Progress::new(tree.size(), report);
                // Left over from an upload that was cut off.
                if o.sftp.try_exists(&part).await.unwrap_or(false) {
                    remove_tree(&o.sftp, &part).await?;
                }
                o.sftp.create_dir(&part).await.map_err(|e| err(e, remote))?;
                wrote = true;
                for d in &tree.dirs {
                    let p = join(&part, d);
                    o.sftp.create_dir(&p).await.map_err(|e| err(e, &p))?;
                }
                for (f, _) in &tree.files {
                    copy_up(&o.sftp, &local.join(f), &join(&part, f), &cancelled, &mut progress).await?;
                }
                progress.finish();
            } else {
                let total = tokio::fs::metadata(local).await?.len();
                let mut progress = Progress::new(total, report);
                wrote = true;
                copy_up(&o.sftp, local, &part, &cancelled, &mut progress).await?;
                progress.finish();
            }
            // SFTP's rename does not replace anything.
            if o.sftp.try_exists(remote).await.unwrap_or(false) {
                remove_tree(&o.sftp, remote).await?;
            }
            o.sftp.rename(&part, remote).await.map_err(|e| err(e, remote))
        }
        .await;
        self.end(transfer);
        if result.is_err() && wrote {
            let _ = remove_tree(&o.sftp, &part).await;
        }
        result
    }
}

/// Removes a file, or a folder with everything in it, depth first and without following links.
async fn remove_tree(sftp: &SftpSession, path: &str) -> Result<()> {
    let meta = sftp.symlink_metadata(path).await.map_err(|e| err(e, path))?;
    if !meta.is_dir() || meta.is_symlink() {
        return sftp.remove_file(path).await.map_err(|e| err(e, path));
    }
    let mut stack = vec![(path.to_string(), false)];
    while let Some((dir, emptied)) = stack.pop() {
        if emptied {
            sftp.remove_dir(&dir).await.map_err(|e| err(e, &dir))?;
            continue;
        }
        stack.push((dir.clone(), true));
        for e in sftp.read_dir(&dir).await.map_err(|e| err(e, &dir))? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = join(&dir, &name);
            let m = e.metadata();
            if m.is_dir() && !m.is_symlink() {
                stack.push((child, false));
            } else {
                sftp.remove_file(&child).await.map_err(|e| err(e, &child))?;
            }
        }
    }
    Ok(())
}

/// What a folder holds, relative to it with `/`: folders (parents first) and files with their sizes.
#[derive(Default)]
struct Tree {
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
}

impl Tree {
    fn size(&self) -> u64 {
        self.files.iter().map(|(_, s)| s).sum()
    }
}

fn under(rel: &str, name: &str) -> String {
    if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") }
}

/// A remote folder's contents. Links are left out: following them could loop or leave the folder.
async fn remote_tree(sftp: &SftpSession, root: &str) -> Result<Tree> {
    let mut tree = Tree::default();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir = if rel.is_empty() { root.to_string() } else { join(root, &rel) };
        for e in sftp.read_dir(&dir).await.map_err(|e| err(e, &dir))? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let m = e.metadata();
            let child = under(&rel, &name);
            if m.is_symlink() {
                continue;
            }
            if m.is_dir() {
                tree.dirs.push(child.clone());
                stack.push(child);
            } else {
                tree.files.push((child, m.size.unwrap_or(0)));
            }
        }
    }
    Ok(tree)
}

/// A local folder's contents, links left out.
fn local_tree(root: &Path) -> Result<Tree> {
    let mut tree = Tree::default();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        for e in std::fs::read_dir(root.join(&rel))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            let m = std::fs::symlink_metadata(e.path())?;
            let child = under(&rel, &name);
            if m.file_type().is_symlink() {
                continue;
            }
            if m.is_dir() {
                tree.dirs.push(child.clone());
                stack.push(child);
            } else {
                tree.files.push((child, m.len()));
            }
        }
    }
    Ok(tree)
}

/// Copies one remote file to a local path, reporting into `progress`.
async fn copy_down(sftp: &SftpSession, remote: &str, local: &Path, cancelled: &watch::Receiver<bool>, progress: &mut Progress<impl Fn(u64, u64)>) -> Result<()> {
    let mut src = sftp.open(remote).await.map_err(|e| err(e, remote))?;
    let mut dst = tokio::fs::File::create(local).await?;
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
    Ok(())
}

/// Copies one local file to a remote path, reporting into `progress`.
async fn copy_up(sftp: &SftpSession, local: &Path, remote: &str, cancelled: &watch::Receiver<bool>, progress: &mut Progress<impl Fn(u64, u64)>) -> Result<()> {
    let mut src = tokio::fs::File::open(local).await?;
    let mut dst = sftp.create(remote).await.map_err(|e| err(e, remote))?;
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
    dst.shutdown().await.map_err(|e| Error::Sftp { message: e.to_string() })
}

fn part_path(local: &Path) -> PathBuf {
    let mut name = local.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    local.with_file_name(name)
}

/// `dir/name`, or `dir/name (2).ext` and so on if that is taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let taken = |p: &Path| p.exists() || part_path(p).exists();
    let mut candidate = dir.join(name);
    let mut n = 2;
    while taken(&candidate) {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

/* ---------- this computer's files, shown beside a server's ---------- */

/// Where browsing this computer starts from: the drives on Windows, `/` elsewhere.
pub fn local_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z').map(|c| format!("{}:\\", c as char)).filter(|d| Path::new(d).exists()).collect()
    }
    #[cfg(not(windows))]
    {
        vec!["/".into()]
    }
}

/// This person's home folder.
pub fn local_home() -> String {
    directories::UserDirs::new()
        .map(|d| d.home_dir().display().to_string())
        .or_else(|| local_roots().into_iter().next())
        .unwrap_or_else(|| "/".into())
}

/// A folder on this computer; `""` lists the roots. Hidden and system files
/// on Windows stay out, as Explorer does; a link counts as what it points to.
pub fn local_list(path: &str) -> Result<Vec<FileEntry>> {
    if path.is_empty() {
        return Ok(local_roots().into_iter().map(|name| FileEntry { name, kind: FileKind::Dir, size: 0, modified: None, permissions: None }).collect());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path)?.flatten() {
        let Ok(link) = entry.metadata() else { continue };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
            if link.file_attributes() & 0x6 != 0 {
                continue;
            }
        }
        let meta = std::fs::metadata(entry.path()).unwrap_or(link);
        let kind = if meta.is_dir() { FileKind::Dir } else if meta.is_file() { FileKind::File } else { FileKind::Link };
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Some(meta.permissions().mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let permissions = None;
        out.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            size: if kind == FileKind::Dir { 0 } else { meta.len() },
            modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64),
            kind,
            permissions,
        });
    }
    Ok(out)
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

    #[test]
    fn a_local_folder_lists_files_and_folders() {
        let dir = std::env::temp_dir().join(format!("detour-local-{}", crate::new_id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let mut list = local_list(&dir.display().to_string()).unwrap();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(list.iter().map(|e| (e.name.as_str(), e.kind, e.size)).collect::<Vec<_>>(), [("a.txt", FileKind::File, 5), ("sub", FileKind::Dir, 0)]);
        assert!(list[0].modified.is_some());
        assert!(!local_list("").unwrap().is_empty(), "the roots");
        assert!(local_list(&dir.join("missing").display().to_string()).is_err());
    }

    #[test]
    fn free_names() {
        let dir = std::env::temp_dir().join(format!("detour-free-{}", crate::new_id()));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::write(dir.join("a (2).txt"), b"").unwrap();
        assert_eq!(free_name(&dir, "a.txt"), dir.join("a (3).txt"));
        assert_eq!(free_name(&dir, "logs"), dir.join("logs (2)"));
        assert_eq!(free_name(&dir, ".bashrc"), dir.join(".bashrc"));
    }
}
