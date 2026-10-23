//! Team vaults mirrored through a Git repository. The repository is served by
//! an installed `git` (the same programs an SSH server runs); the tests are
//! skipped when there is none. `git fsck` checks that what Detour writes is
//! valid Git.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use detour_core::identity::Identity;
use detour_core::model::{Auth, HostView};
use detour_core::store::VaultUpdate;
use detour_core::team::TeamVault;
use detour_core::team::git::{Mirror, Remote};
use detour_core::team::state::{Level, Problem, Role};

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-git-{tag}-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn bare_repo() -> (PathBuf, Remote) {
    let repo = temp_dir("remote").join("vault.git");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--bare", "-q", "-b", "main"]);
    let remote = Remote::Process { git: "git".into(), repo: repo.clone() };
    (repo, remote)
}

fn host(id: &str, label: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: label.into(), address: "10.0.0.1".into(), port: 22, user: "root".into(), auth: Auth::Agent, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None }
}

fn put(v: &mut TeamVault, hosts: Vec<HostView>) {
    v.put(VaultUpdate { hosts, keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
}

fn labels(v: &TeamVault) -> Vec<String> {
    let mut l: Vec<String> = v.view().data.hosts.iter().map(|h| h.label.clone()).collect();
    l.sort();
    l
}

struct Person {
    id: Identity,
    mirror: Mirror,
}

fn person() -> Person {
    Person { id: Identity::generate(), mirror: Mirror::new(temp_dir("person")) }
}

/// Alice creates a team in the repository; Bob clones it and is added with Edit access.
async fn two_people() -> Option<(PathBuf, Remote, Person, TeamVault, Person, TeamVault)> {
    if !git_available() {
        eprintln!("git not installed: skipped");
        return None;
    }
    let (repo, remote) = bare_repo();
    let alice = person();
    let recovery = Identity::generate();
    let mut a = TeamVault::create(alice.mirror.files(), "Acme", &alice.id, "Alice", "alice-pc", &recovery).unwrap();
    put(&mut a, vec![host("h1", "api-1")]);
    assert!(alice.mirror.push(&remote).await.unwrap().pushed);

    let bob = person();
    bob.mirror.pull(&remote).await.unwrap();
    let info = TeamVault::inspect(bob.mirror.files()).unwrap();
    assert_eq!(info.root, alice.id.public().detour_id());
    a.add_member(&bob.id.public().detour_id(), "Bob", Role::Member, Level::Edit).unwrap();
    alice.mirror.sync(&remote).await.unwrap();
    bob.mirror.pull(&remote).await.unwrap();
    let b = TeamVault::open(bob.mirror.files(), &info.root, &bob.id, "bob-pc", BTreeMap::new()).unwrap();
    assert!(b.view().joined);
    Some((repo, remote, alice, a, bob, b))
}

#[tokio::test]
async fn a_team_travels_through_git_and_stays_valid_git() {
    let Some((repo, remote, alice, mut a, bob, mut b)) = two_people().await else { return };
    assert_eq!(labels(&b), ["api-1"]);

    // Both write without pulling first: the second push is stale, pulls and retries.
    put(&mut a, vec![host("h1", "api-1"), host("h2", "from-alice")]);
    let mut data = b.view().data;
    data.hosts.push(host("h3", "from-bob"));
    b.put(VaultUpdate { hosts: data.hosts, keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    assert!(alice.mirror.push(&remote).await.unwrap().pushed);
    let s = bob.mirror.push(&remote).await.unwrap();
    assert!(s.pushed && s.pulled);
    alice.mirror.pull(&remote).await.unwrap();
    a.reload().unwrap();
    b.reload().unwrap();
    assert_eq!(labels(&a), ["api-1", "from-alice", "from-bob"]);
    assert_eq!(labels(&a), labels(&b));
    assert!(a.view().problems.is_empty(), "{:?}", a.view().problems);
    assert!(!alice.mirror.pending().unwrap() && !bob.mirror.pending().unwrap());

    // Real Git agrees with every object Detour wrote.
    git(&repo, &["fsck", "--full", "--strict"]);
    assert!(git(&repo, &["log", "--format=%an", "main"]).lines().all(|l| l == "Detour"));
}

#[tokio::test]
async fn packs_with_deltas_from_git_are_read() {
    let Some((repo, remote, alice, mut a, bob, _b)) = two_people().await else { return };
    // Many similar files, then a repack that turns them into deltas.
    for i in 0..30 {
        put(&mut a, (0..=i).map(|j| host(&format!("h{j}"), &format!("server-{j}"))).collect());
    }
    assert!(alice.mirror.push(&remote).await.unwrap().pushed);
    git(&repo, &["repack", "-adf", "-q"]);
    let idx = std::fs::read_dir(repo.join("objects/pack")).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "idx")).unwrap();
    assert!(git(&repo, &["verify-pack", "-v", idx.to_str().unwrap()]).contains("chain length"), "the pack must hold deltas for this test to mean anything");
    let fresh = Mirror::new(temp_dir("fresh"));
    fresh.pull(&remote).await.unwrap();
    let carol = Identity::generate();
    let info = TeamVault::inspect(fresh.files()).unwrap();
    let c = TeamVault::open(fresh.files(), &info.root, &carol, "carol-pc", BTreeMap::new()).unwrap();
    assert!(c.view().problems.is_empty(), "{:?}", c.view().problems);
    bob.mirror.pull(&remote).await.unwrap();
}

/// Changes the repository behind the clients' backs, as a hostile host could.
fn tamper(repo: &Path, f: impl FnOnce(&Path)) {
    let work = temp_dir("tamper");
    git(&work, &["clone", "-q", repo.to_str().unwrap(), "w"]);
    let w = work.join("w");
    f(&w);
    git(&w, &["-c", "user.name=x", "-c", "user.email=x@x", "commit", "-qam", "tamper", "--allow-empty"]);
    git(&w, &["push", "-q", "origin", "HEAD:main"]);
}

#[tokio::test]
async fn a_changed_entry_is_kept_beside_the_original_and_reported() {
    let Some((repo, remote, alice, _a, bob, mut b)) = two_people().await else { return };
    let file = "ops/alice-pc/00000002.json";
    let original = std::fs::read(bob.mirror.files().join(file)).unwrap();
    tamper(&repo, |w| std::fs::write(w.join(file), b"{\"body\":\"x\",\"sig\":\"y\"}").unwrap());

    bob.mirror.pull(&remote).await.unwrap();
    assert_eq!(std::fs::read(bob.mirror.files().join(file)).unwrap(), original, "a file we have is never replaced");
    b.reload().unwrap();
    assert!(b.view().problems.iter().any(|p| matches!(p.problem, Problem::UnknownFile { .. } | Problem::Forked { .. })), "{:?}", b.view().problems);

    // Bob's next push puts the original back for everyone.
    assert!(bob.mirror.push(&remote).await.unwrap().pushed);
    alice.mirror.pull(&remote).await.unwrap();
    let fresh = Mirror::new(temp_dir("fresh"));
    fresh.pull(&remote).await.unwrap();
    assert_eq!(std::fs::read(fresh.files().join(file)).unwrap(), original);
}

#[tokio::test]
async fn deleted_entries_come_back() {
    let Some((repo, remote, _alice, _a, bob, _b)) = two_people().await else { return };
    let file = "ops/alice-pc/00000002.json";
    tamper(&repo, |w| {
        std::fs::remove_file(w.join(file)).unwrap();
        git(w, &["rm", "-q", "--cached", file]);
    });
    bob.mirror.pull(&remote).await.unwrap();
    assert!(bob.mirror.files().join(file).exists());
    assert!(bob.mirror.pending().unwrap());
    bob.mirror.push(&remote).await.unwrap();
    let fresh = Mirror::new(temp_dir("fresh"));
    fresh.pull(&remote).await.unwrap();
    assert!(fresh.files().join(file).exists());
}

#[tokio::test]
async fn unsafe_paths_are_refused() {
    let Some((repo, remote, _alice, _a, bob, _b)) = two_people().await else { return };
    tamper(&repo, |w| {
        std::fs::create_dir_all(w.join("ops/x y")).unwrap();
        std::fs::write(w.join("ops/x y/evil.json"), b"x").unwrap();
        git(w, &["add", "."]);
    });
    let err = bob.mirror.pull(&remote).await.unwrap_err();
    assert!(err.to_string().contains("unsafe"), "{err}");
    assert!(!bob.mirror.files().join("ops/x y").exists());
}

#[tokio::test]
async fn an_empty_repository_is_recognised() {
    if !git_available() {
        return;
    }
    let (_repo, remote) = bare_repo();
    assert!(Mirror::remote_is_empty(&remote).await.unwrap().0);
}

/* ---------- over SSH ---------- */

mod over_ssh {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use detour_core::model::{Auth, Host, KnownHost};
    use detour_core::ssh::ConnectRequest;
    use russh::keys::{Algorithm, PrivateKey};
    use russh::server::{self, Msg, Server as _, Session};
    use russh::{Channel, ChannelId};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::process::ChildStdin;

    /// An SSH server that runs `git-upload-pack` / `git-receive-pack` like a Git host does.
    #[derive(Clone, Default)]
    struct GitServer {
        stdin: Arc<Mutex<HashMap<ChannelId, Arc<tokio::sync::Mutex<ChildStdin>>>>>,
        commands: Arc<Mutex<Vec<String>>>,
        connections: Arc<Mutex<usize>>,
    }

    impl server::Server for GitServer {
        type Handler = Self;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
            *self.connections.lock().unwrap() += 1;
            self.clone()
        }
    }

    impl server::Handler for GitServer {
        type Error = russh::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
            Ok(if user == "git" && password == "pw" { server::Auth::Accept } else { server::Auth::reject() })
        }

        async fn auth_publickey(&mut self, user: &str, _: &russh::keys::PublicKey) -> Result<server::Auth, Self::Error> {
            Ok(if user == "git" { server::Auth::Accept } else { server::Auth::reject() })
        }

        async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
            let cmd = String::from_utf8_lossy(data).to_string();
            self.commands.lock().unwrap().push(cmd.clone());
            let (service, path) = cmd.split_once(' ').unwrap();
            let path = path.trim_matches('\'');
            // ssh://host/C:/x arrives as /C:/x
            let path = if path.as_bytes().get(2) == Some(&b':') { &path[1..] } else { path };
            let mut child = tokio::process::Command::new("git")
                .arg(service.strip_prefix("git-").unwrap())
                .arg(path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            self.stdin.lock().unwrap().insert(channel, Arc::new(tokio::sync::Mutex::new(child.stdin.take().unwrap())));
            let mut stdout = child.stdout.take().unwrap();
            let handle = session.handle();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 32 * 1024];
                loop {
                    match stdout.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if handle.data(channel, buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                let code = child.wait().await.map(|s| s.code().unwrap_or(1) as u32).unwrap_or(1);
                let _ = handle.exit_status_request(channel, code).await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
            });
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

    /// Starts a Git SSH server on a free port.
    pub async fn start() -> u16 {
        serve().await.0
    }

    async fn serve() -> (u16, PrivateKey, Arc<Mutex<Vec<String>>>, Arc<Mutex<usize>>) {
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let config = Arc::new(server::Config { keys: vec![host_key.clone()], auth_rejection_time: Duration::from_millis(10), ..Default::default() });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut srv = GitServer::default();
        let (commands, connections) = (srv.commands.clone(), srv.connections.clone());
        tokio::spawn(async move {
            let _ = srv.run_on_socket(config, &listener).await;
        });
        (port, host_key, commands, connections)
    }

    #[tokio::test]
    async fn the_mirror_works_over_ssh() {
        if !super::git_available() {
            return;
        }
        let (port, host_key, commands, connections) = serve().await;

        let (repo, _) = super::bare_repo();
        let pk = host_key.public_key();
        let known = KnownHost { endpoint: format!("127.0.0.1:{port}"), key: pk.to_openssh().unwrap(), fingerprint: pk.fingerprint(Default::default()).to_string(), added_at: 0 };
        let host = Host { id: "git".into(), group: String::new(), label: "git".into(), address: "127.0.0.1".into(), port, user: "git".into(), auth: Auth::Password, tags: vec![], last_connected: None, password: Some("pw".into()), proxy: None, proxy_password: None };
        let remote = super::Remote::Ssh { request: ConnectRequest { host, key: None, secret: None, known: Some(known), trust_fingerprint: None, route: Default::default() }, path: repo.display().to_string() };

        let alice = super::person();
        let mut a = super::TeamVault::create(alice.mirror.files(), "Acme", &alice.id, "Alice", "alice-pc", &super::Identity::generate()).unwrap();
        super::put(&mut a, vec![super::host("h1", "over-ssh")]);
        assert!(alice.mirror.push(&remote).await.unwrap().pushed);

        let fresh = super::Mirror::new(super::temp_dir("ssh-fresh"));
        fresh.pull(&remote).await.unwrap();
        assert!(fresh.files().join("vault.json").exists());
        super::git(&repo, &["fsck", "--full", "--strict"]);
        let cmds = commands.lock().unwrap().clone();
        assert!(cmds.iter().any(|c| c.starts_with("git-receive-pack '")) && cmds.iter().any(|c| c.starts_with("git-upload-pack '")), "{cmds:?}");

        // Every command ran over one kept connection.
        for _ in 0..3 {
            fresh.pull(&remote).await.unwrap();
        }
        assert!(commands.lock().unwrap().len() >= 5);
        assert_eq!(*connections.lock().unwrap(), 1);
        // Locking the vault closes it; the next command connects again.
        detour_core::ssh::close_pooled();
        fresh.pull(&remote).await.unwrap();
        assert_eq!(*connections.lock().unwrap(), 2);
    }
}

/* ---------- over HTTP ---------- */

mod over_http {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use base64::Engine;
    use detour_core::team::git::{HttpRemote, HttpUrl, Remote};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// A Git host over HTTP: `git http-backend` behind a tiny CGI server that wants a token.
    pub async fn start(root: PathBuf, token: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let (root, seen) = (root.clone(), seen.clone());
                tokio::spawn(async move {
                    let _ = serve(sock, &root, token, &seen).await;
                });
            }
        });
        (port, log)
    }

    async fn serve(mut sock: TcpStream, root: &Path, token: &str, log: &Mutex<Vec<String>>) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let head_end = loop {
            let n = sock.read(&mut chunk).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.lines();
        let mut first = lines.next().unwrap_or_default().split(' ');
        let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
        let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
        let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = buf[head_end..].to_vec();
        while body.len() < length {
            let n = sock.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        log.lock().unwrap().push(format!("{method} {target}"));

        let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("detour:{token}")));
        if header("authorization").as_deref() != Some(expected.as_str()) {
            sock.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"git\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
            return Ok(());
        }
        let (path, query) = target.split_once('?').unwrap_or((&target, ""));
        let mut child = tokio::process::Command::new("git")
            .arg("http-backend")
            .env("GIT_PROJECT_ROOT", root)
            .env("GIT_HTTP_EXPORT_ALL", "1")
            .env("REQUEST_METHOD", &method)
            .env("PATH_INFO", path)
            .env("QUERY_STRING", query)
            .env("CONTENT_TYPE", header("content-type").unwrap_or_default())
            .env("CONTENT_LENGTH", body.len().to_string())
            .env("REMOTE_USER", "tester")
            .env("REMOTE_ADDR", "127.0.0.1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&body).await?;
        drop(stdin);
        let out = child.wait_with_output().await?.stdout;
        // CGI output: headers, a blank line, the body.
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4)).or_else(|| out.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2))).unwrap_or((out.len(), 0));
        let cgi_head = String::from_utf8_lossy(&out[..split.0]).to_string();
        let cgi_body = &out[(split.0 + split.1).min(out.len())..];
        let mut status = "200 OK".to_string();
        let mut resp = String::new();
        for l in cgi_head.lines() {
            match l.split_once(':') {
                Some((k, v)) if k.eq_ignore_ascii_case("status") => status = v.trim().to_string(),
                Some((k, v)) => resp.push_str(&format!("{k}: {}\r\n", v.trim())),
                None => {}
            }
        }
        let head = format!("HTTP/1.1 {status}\r\n{resp}Content-Length: {}\r\nConnection: close\r\n\r\n", cgi_body.len());
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(cgi_body).await?;
        Ok(())
    }

    fn remote(port: u16, token: &str) -> Remote {
        Remote::Http(HttpRemote { url: HttpUrl::parse(&format!("http://127.0.0.1:{port}/vault.git")).unwrap(), token: Some(token.into()) })
    }

    #[tokio::test]
    async fn the_mirror_works_over_http_with_a_token() {
        if !super::git_available() {
            return;
        }
        let (repo, _) = super::bare_repo();
        let (port, log) = start(repo.parent().unwrap().to_path_buf(), "s3cret").await;

        let alice = super::person();
        let mut a = super::TeamVault::create(alice.mirror.files(), "Acme", &alice.id, "Alice", "alice-pc", &super::Identity::generate()).unwrap();
        super::put(&mut a, vec![super::host("h1", "over-http")]);
        assert!(alice.mirror.push(&remote(port, "s3cret")).await.unwrap().pushed);

        let fresh = super::Mirror::new(super::temp_dir("http-fresh"));
        fresh.pull(&remote(port, "s3cret")).await.unwrap();
        let info = super::TeamVault::inspect(fresh.files()).unwrap();
        assert_eq!(info.root, alice.id.public().detour_id());
        super::git(&repo, &["fsck", "--full", "--strict"]);
        let seen = log.lock().unwrap().clone();
        assert!(seen.iter().any(|l| l == "POST /vault.git/git-receive-pack") && seen.iter().any(|l| l == "POST /vault.git/git-upload-pack"), "{seen:?}");

        // A wrong token is refused with a clear error, and nothing is written.
        let err = super::Mirror::new(super::temp_dir("http-bad")).pull(&remote(port, "wrong")).await.unwrap_err();
        assert!(err.to_string().contains("token rejected"), "{err}");
    }

    #[test]
    fn addresses() {
        let u = HttpUrl::parse("https://me@github.com/acme/vault.git/").unwrap();
        assert_eq!((u.base.as_str(), u.host.as_str(), u.user.as_deref()), ("https://github.com/acme/vault.git", "github.com", Some("me")));
        assert!(HttpUrl::parse("http://github.com/acme/vault.git").is_err(), "no tokens in the clear");
        assert!(HttpUrl::parse("http://127.0.0.1:8080/vault.git").is_ok());
        assert!(HttpUrl::parse("ftp://x/y").is_err());
    }
}

mod stores {
    use std::sync::Mutex;

    use detour_core::Error;
    use detour_core::model::{Auth, KeyAlgorithm};
    use detour_core::store::{Store, VaultUpdate};
    use detour_core::sync;
    use detour_core::team::git::GitRemote;
    use detour_core::team::state::{Level, Role};

    fn store(tag: &str) -> (Mutex<Store>, GitRemote) {
        let mut s = Store::open(super::temp_dir(tag));
        s.create("m").unwrap();
        let key = s.generate_key("repo-key", KeyAlgorithm::Ed25519, None, false).unwrap();
        (Mutex::new(s), GitRemote { url: String::new(), auth: Auth::Key { key_id: key.id }, token: None })
    }

    fn put(s: &Mutex<Store>, id: &str, hosts: Vec<detour_core::model::HostView>) {
        s.lock().unwrap().team_put(id, VaultUpdate { hosts, keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    }

    #[tokio::test]
    async fn a_personal_vault_syncs_through_a_repository() {
        if !super::git_available() {
            return;
        }
        let (repo, _) = super::bare_repo();
        let (port, _) = super::over_http::start(repo.parent().unwrap().to_path_buf(), "tok").await;
        let remote = GitRemote { url: format!("http://127.0.0.1:{port}/vault.git"), auth: Auth::Agent, token: Some("tok".into()) };

        let (laptop, _) = store("laptop");
        laptop.lock().unwrap().put(VaultUpdate { hosts: vec![super::host("h1", "in-git")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
        sync::enable_git_sync(&laptop, remote.clone(), None).await.unwrap();
        let st = laptop.lock().unwrap().sync_status().unwrap();
        assert!(st.enabled && st.git && !st.pending && st.error.is_none(), "{st:?}");
        assert!(matches!(sync::enable_git_sync(&store("other").0, remote.clone(), None).await, Err(Error::FolderNotEmpty)));

        let code = sync::add_device(&laptop).await.unwrap();
        let (desktop, _) = store("desktop");
        sync::join_git_sync(&desktop, remote.clone(), code.code, None).await.unwrap();
        let hosts = desktop.lock().unwrap().view().unwrap().hosts;
        assert!(hosts.iter().any(|h| h.label == "in-git"), "{hosts:?}");

        // An edit on the desktop goes out after it, and comes in on the laptop's next sync.
        let mut v = desktop.lock().unwrap().view().unwrap();
        v.hosts.push(super::host("h2", "second"));
        desktop.lock().unwrap().put(VaultUpdate { hosts: v.hosts, keys: v.keys, snippets: v.snippets, tunnels: v.tunnels }).unwrap();
        assert!(desktop.lock().unwrap().sync_status().unwrap().pending);
        sync::push_personal(&desktop).await.unwrap();
        assert!(sync::sync_personal(&laptop, None).await.unwrap());
        assert_eq!(laptop.lock().unwrap().view().unwrap().hosts.len(), 2);
        // The desktop's key travelled too (the laptop's was deleted before syncing).
        assert_eq!(laptop.lock().unwrap().view().unwrap().keys.len(), 1);
        super::git(&repo, &["fsck", "--full", "--strict"]);
    }

    #[tokio::test]
    async fn a_team_in_git_joins_on_another_device_by_itself() {
        if !super::git_available() {
            return;
        }
        let (repo, _) = super::bare_repo();
        let (port, _) = super::over_http::start(repo.parent().unwrap().to_path_buf(), "tok").await;
        let remote = GitRemote { url: format!("http://127.0.0.1:{port}/vault.git"), auth: Auth::Password, token: Some("tok".into()) };

        let (laptop, _) = store("laptop");
        let team = sync::create_git_team(&laptop, remote, "Acme", "Me", &Store::new_recovery_key(), None).await.unwrap();
        let sync_dir = super::temp_dir("sync").join("vault").display().to_string();
        laptop.lock().unwrap().sync_enable(&sync_dir, None).unwrap();
        let code = sync::add_device(&laptop).await.unwrap();

        let (desktop, _) = store("desktop");
        // A fresh device has its own identity but no teams yet.
        sync::join_folder_sync(&desktop, sync_dir, code.code).await.unwrap();
        assert!(desktop.lock().unwrap().pending_teams().unwrap()[0].url.is_some());
        assert!(sync::join_linked_teams(&desktop).await, "the repository, token and creator came with the link");
        let teams = desktop.lock().unwrap().teams();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].id, team.id);
        assert!(teams[0].view.as_ref().is_some_and(|v| v.joined && v.remote.is_some()));
        assert!(desktop.lock().unwrap().pending_teams().unwrap().is_empty());
    }

    #[tokio::test]
    async fn two_stores_share_a_team_through_a_repository() {
        if !super::git_available() {
            return;
        }
        let port = super::over_ssh::start().await;
        let (repo, _) = super::bare_repo();
        let url = format!("ssh://git@127.0.0.1:{port}/{}", repo.display().to_string().replace('\\', "/"));

        let (alice, mut ra) = store("alice");
        ra.url = url.clone();
        let recovery = Store::new_recovery_key();
        // First contact: the server key must be confirmed.
        let fp = match sync::create_git_team(&alice, ra.clone(), "Acme", "Alice", &recovery, None).await {
            Err(Error::HostKeyUnknown { fingerprint, .. }) => fingerprint,
            other => panic!("{:?}", other.map(|v| v.id)),
        };
        let team = sync::create_git_team(&alice, ra.clone(), "Acme", "Alice", &recovery, Some(fp.clone())).await.unwrap();
        assert!(team.remote.as_ref().is_some_and(|r| r.url == url && !r.pending));
        // A second team can't go into the same, now non-empty, repository.
        assert!(matches!(sync::create_git_team(&alice, ra.clone(), "Other", "Alice", &recovery, None).await, Err(Error::FolderNotEmpty)));

        put(&alice, &team.id, vec![super::host("h1", "in-git")]);
        assert!(alice.lock().unwrap().team_view(&team.id).unwrap().remote.unwrap().pending);
        sync::push_team(&alice, &team.id).await.unwrap();

        // Bob clones, confirms the creator, joins, and gets added.
        let (bob, mut rb) = store("bob");
        rb.url = url.clone();
        let (info, token) = sync::inspect_git_team(&bob, &rb, Some(fp.clone())).await.unwrap();
        assert_eq!(info.root_fingerprint, alice.lock().unwrap().my_id().unwrap().fingerprint);
        let view = sync::join_git_team(&bob, &token, &info.root, rb.clone()).unwrap();
        assert!(!view.joined);
        let bob_id = bob.lock().unwrap().my_id().unwrap().detour_id;
        alice.lock().unwrap().team_add_member(&team.id, &bob_id, "Bob", Role::Member, Level::Edit).unwrap();
        sync::sync_team(&alice, &team.id, None).await.unwrap();
        let view = sync::sync_team(&bob, &team.id, None).await.unwrap();
        assert!(view.joined, "{:?}", view.remote);
        assert_eq!(view.data.hosts[0].label, "in-git");

        // Bob's edit reaches Alice.
        let mut data = view.data.clone();
        data.hosts[0].label = "renamed-by-bob".into();
        put(&bob, &team.id, data.hosts);
        sync::push_team(&bob, &team.id).await.unwrap();
        let view = sync::sync_team(&alice, &team.id, None).await.unwrap();
        assert_eq!(view.data.hosts[0].label, "renamed-by-bob");
        let remote = view.remote.unwrap();
        assert!(remote.error.is_none() && !remote.pending && remote.synced_at.is_some(), "{remote:?}");
        super::git(&repo, &["fsck", "--full", "--strict"]);

        // Leaving removes the local copy; the repository stays.
        bob.lock().unwrap().leave_team(&team.id).unwrap();
        assert!(bob.lock().unwrap().teams().is_empty());
    }
}
