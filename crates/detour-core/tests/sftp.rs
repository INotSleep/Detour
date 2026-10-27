//! File browsing and transfers against a real `sftp-server` (OpenSSH's, as
//! Git for Windows and Linux distributions ship it) behind an in-process SSH
//! server. Skipped when there is none.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use detour_core::Error;
use detour_core::model::{Auth, Host, KnownHost};
use detour_core::sftp::{FileKind, Files, join};
use detour_core::ssh::ConnectRequest;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::ChildStdin;

fn sftp_server() -> Option<PathBuf> {
    ["C:/Program Files/Git/usr/lib/ssh/sftp-server.exe", "/usr/lib/openssh/sftp-server", "/usr/libexec/sftp-server", "/usr/lib/ssh/sftp-server", "/usr/libexec/openssh/sftp-server"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

#[derive(Clone)]
struct Sshd {
    program: PathBuf,
    /// The directory sftp-server starts in: the client's home.
    root: PathBuf,
    stdin: Arc<Mutex<HashMap<ChannelId, Arc<tokio::sync::Mutex<ChildStdin>>>>>,
}

impl server::Server for Sshd {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Sshd {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
        Ok(if user == "u" && password == "pw" { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel)?;
            return Ok(());
        }
        let mut child = tokio::process::Command::new(&self.program)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        self.stdin.lock().unwrap().insert(channel, Arc::new(tokio::sync::Mutex::new(child.stdin.take().unwrap())));
        let mut stdout = child.stdout.take().unwrap();
        let handle = session.handle();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = stdout.read(&mut buf).await {
                if n == 0 || handle.data(channel, buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
            let _ = child.wait().await;
            let _ = handle.close(channel).await;
        });
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], _: &mut Session) -> Result<(), Self::Error> {
        let stdin = self.stdin.lock().unwrap().get(&channel).cloned();
        if let Some(stdin) = stdin {
            let _ = stdin.lock().await.write_all(data).await;
        }
        Ok(())
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-sftp-{tag}-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An SSH server whose SFTP home is a fresh local directory, and a request to log in to it.
async fn server() -> Option<(PathBuf, ConnectRequest)> {
    let program = sftp_server()?;
    let root = temp_dir("root");
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(server::Config { keys: vec![key.clone()], ..Default::default() });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut srv = Sshd { program, root: root.clone(), stdin: Default::default() };
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    });
    let pk = key.public_key();
    let known = KnownHost { endpoint: format!("127.0.0.1:{port}"), key: pk.to_openssh().unwrap(), fingerprint: pk.fingerprint(Default::default()).to_string(), added_at: 0 };
    let host = Host { id: "h".into(), group: String::new(), label: "t".into(), address: "127.0.0.1".into(), port, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, password: Some("pw".into()), proxy: None, proxy_password: None };
    Some((root, ConnectRequest { host, key: None, secret: None, known: Some(known), trust_fingerprint: None, route: Default::default() }))
}

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

#[tokio::test]
async fn browse_upload_download_rename_remove() {
    let Some((root, req)) = server().await else {
        eprintln!("no sftp-server: skipped");
        return;
    };
    let files = Files::new();
    let (id, home) = {
        let (id, home, _) = files.open(&req).await.unwrap();
        (id, home)
    };

    let dir = join(&home, "docs");
    files.mkdir(&id, &dir).await.unwrap();
    let local = temp_dir("local");
    let data = sample(3 * 1024 * 1024 + 17);
    std::fs::write(local.join("big.bin"), &data).unwrap();

    // Upload with progress up to the full size.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    files.upload(&id, &local.join("big.bin"), &join(&dir, "big.bin"), false, "t1", move |d, t| s.lock().unwrap().push((d, t))).await.unwrap();
    assert_eq!(seen.lock().unwrap().last().copied(), Some((data.len() as u64, data.len() as u64)));
    assert_eq!(std::fs::read(root.join("docs/big.bin")).unwrap(), data, "the server got every byte");

    let list = files.list(&id, &home).await.unwrap();
    assert_eq!(list[0].name, "docs");
    assert_eq!(list[0].kind, FileKind::Dir);
    let inner = files.list(&id, &dir).await.unwrap();
    assert_eq!((inner[0].name.as_str(), inner[0].size), ("big.bin", data.len() as u64));
    assert!(inner[0].modified.is_some());

    // Download it back.
    let back = local.join("back.bin");
    files.download(&id, &join(&dir, "big.bin"), &back, "t2", |_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), data);

    // Rename refuses to overwrite.
    std::fs::write(root.join("docs/other.txt"), b"x").unwrap();
    assert!(files.rename(&id, &join(&dir, "big.bin"), &join(&dir, "other.txt")).await.is_err());
    files.rename(&id, &join(&dir, "big.bin"), &join(&dir, "renamed.bin")).await.unwrap();
    assert!(root.join("docs/renamed.bin").exists());

    // Removing a folder takes everything in it, and says how far it got.
    std::fs::create_dir_all(root.join("docs/deep/er")).unwrap();
    std::fs::write(root.join("docs/deep/er/f.txt"), b"y").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    files.remove(&id, &dir, "t3", move |d, t| s.lock().unwrap().push((d, t))).await.unwrap();
    // docs, deep, er; f.txt, other.txt, renamed.bin.
    assert_eq!(seen.lock().unwrap().last().copied(), Some((6, 6)));
    assert!(!root.join("docs").exists());
    assert!(files.list(&id, &home).await.unwrap().is_empty());

    files.close(&id);
    assert!(matches!(files.list(&id, &home).await, Err(Error::NotFound { .. })));
}

#[tokio::test]
async fn a_cancelled_download_leaves_nothing_behind() {
    let Some((root, req)) = server().await else { return };
    std::fs::write(root.join("huge.bin"), sample(64 * 1024 * 1024)).unwrap();
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    let local = temp_dir("local").join("huge.bin");

    let f2 = files.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f2.cancel("big");
    });
    let res = files.download(&id, &join(&home, "huge.bin"), &local, "big", |_, _| {}).await;
    canceller.await.unwrap();
    assert!(matches!(res, Err(Error::Cancelled)), "{res:?}");
    assert!(!local.exists());
    assert!(!local.with_file_name("huge.bin.part").exists());
}

#[tokio::test]
async fn an_upload_replaces_a_file_only_when_it_completes() {
    let Some((root, req)) = server().await else { return };
    std::fs::write(root.join("notes.txt"), b"old").unwrap();
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    let local = temp_dir("local");
    std::fs::write(local.join("big.bin"), sample(64 * 1024 * 1024)).unwrap();
    std::fs::write(local.join("notes.txt"), b"new").unwrap();

    let f2 = files.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f2.cancel("up");
    });
    let res = files.upload(&id, &local.join("big.bin"), &join(&home, "notes.txt"), true, "up", |_, _| {}).await;
    canceller.await.unwrap();
    assert!(matches!(res, Err(Error::Cancelled)), "{res:?}");
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"old", "the old file is untouched");
    assert!(!root.join("notes.txt.part").exists());

    files.upload(&id, &local.join("notes.txt"), &join(&home, "notes.txt"), true, "up2", |_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "nothing left beside it");
}

#[tokio::test]
async fn missing_paths_are_not_found() {
    let Some((_root, req)) = server().await else { return };
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    assert!(matches!(files.list(&id, &join(&home, "nope")).await, Err(Error::NotFound { .. })));
    let up = files.resolve(&id, &join(&home, "..")).await.unwrap();
    assert!(home.starts_with(&up) && up.len() < home.len(), "{up} vs {home}");
}


/// A small tree: `site/index.html`, `site/css/a.css` and an empty `site/empty/`.
fn local_site(dir: &std::path::Path) -> std::path::PathBuf {
    let site = dir.join("site");
    std::fs::create_dir_all(site.join("css")).unwrap();
    std::fs::create_dir_all(site.join("empty")).unwrap();
    std::fs::write(site.join("index.html"), b"<h1>hi</h1>").unwrap();
    std::fs::write(site.join("css/a.css"), sample(300 * 1024)).unwrap();
    site
}

#[tokio::test]
async fn folders_go_up_and_come_back_whole() {
    let Some((root, req)) = server().await else { return };
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    let local = temp_dir("local");
    let site = local_site(&local);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    files.upload(&id, &site, &join(&home, "site"), false, "up", move |d, t| s.lock().unwrap().push((d, t))).await.unwrap();
    let total = 300 * 1024 + 11;
    assert_eq!(seen.lock().unwrap().last().copied(), Some((total, total)), "one progress for the whole folder");
    assert_eq!(std::fs::read(root.join("site/css/a.css")).unwrap(), sample(300 * 1024));
    assert!(root.join("site/empty").is_dir());
    assert!(!root.join("site.part").exists());
    // Without replace, an existing folder is not touched.
    assert!(files.upload(&id, &site, &join(&home, "site"), false, "up2", |_, _| {}).await.is_err());

    // Into a local folder: under its own name, then under a free one.
    let back = temp_dir("back");
    let first = files.download_into(&id, &join(&home, "site"), &back, "d1", |_, _| {}).await.unwrap();
    let second = files.download_into(&id, &join(&home, "site"), &back, "d2", |_, _| {}).await.unwrap();
    assert_eq!(first, back.join("site"));
    assert_eq!(second, back.join("site (2)"));
    assert_eq!(std::fs::read(back.join("site (2)/css/a.css")).unwrap(), sample(300 * 1024));
    assert_eq!(std::fs::read(back.join("site/index.html")).unwrap(), b"<h1>hi</h1>");
    assert!(back.join("site/empty").is_dir());
    // A single file goes in the same way.
    let one = files.download_into(&id, &join(&home, "site/index.html"), &back, "d3", |_, _| {}).await.unwrap();
    assert_eq!(one, back.join("index.html"));
}

#[tokio::test]
async fn replacing_a_folder_happens_only_when_the_upload_is_complete() {
    let Some((root, req)) = server().await else { return };
    std::fs::create_dir_all(root.join("site")).unwrap();
    std::fs::write(root.join("site/old.txt"), b"old").unwrap();
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    let local = temp_dir("local");
    let site = local_site(&local);
    std::fs::write(site.join("big.bin"), sample(64 * 1024 * 1024)).unwrap();

    let f2 = files.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f2.cancel("up");
    });
    let res = files.upload(&id, &site, &join(&home, "site"), true, "up", |_, _| {}).await;
    canceller.await.unwrap();
    assert!(matches!(res, Err(Error::Cancelled)), "{res:?}");
    assert_eq!(std::fs::read(root.join("site/old.txt")).unwrap(), b"old", "the old folder is untouched");
    assert!(!root.join("site.part").exists());

    std::fs::remove_file(site.join("big.bin")).unwrap();
    files.upload(&id, &site, &join(&home, "site"), true, "up2", |_, _| {}).await.unwrap();
    assert!(!root.join("site/old.txt").exists(), "replaced, not merged");
    assert!(root.join("site/index.html").exists());
}

#[tokio::test]
async fn permissions_can_be_changed() {
    let Some((_root, req)) = server().await else { return };
    let files = Files::new();
    let (id, home, _) = files.open(&req).await.unwrap();
    let local = temp_dir("local");
    std::fs::write(local.join("run.sh"), b"#!/bin/sh\n").unwrap();
    files.upload(&id, &local.join("run.sh"), &join(&home, "run.sh"), false, "u", |_, _| {}).await.unwrap();
    files.chmod(&id, &join(&home, "run.sh"), 0o750).await.unwrap();
    // Windows' sftp-server only approximates Unix bits.
    if cfg!(unix) {
        let list = files.list(&id, &home).await.unwrap();
        assert_eq!(list.iter().find(|e| e.name == "run.sh").unwrap().permissions, Some(0o750));
    }
    assert!(matches!(files.chmod(&id, &join(&home, "nope"), 0o600).await, Err(Error::NotFound { .. })));
}
