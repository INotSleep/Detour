//! Team vaults in a Git repository, without a system `git`.
//!
//! A team vault is kept as a working copy (`<dir>/files`), the same layout the
//! folder transport uses; `pull` and `push` mirror it with one branch of a
//! remote repository over SSH. Git history means nothing to Detour: every
//! entry is signed and chained by itself. So the client never deletes or
//! overwrites a file it has: a version the remote changed is kept next to it
//! (and reported as a second version of that entry), a file the remote lost is
//! pushed again.

pub mod object;
pub mod pack;
pub mod proto;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use self::object::{Kind, Oid, TreeEntry};
use self::proto::{Pushed, Stream};
use crate::model::{Auth, KnownHost};
use crate::ssh::{self, ConnectRequest};
use crate::{Error, Result};

pub(crate) fn bad(what: &str) -> Error {
    Error::Git { message: what.to_string() }
}

/// Where a team's repository lives and how to log in to it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemote {
    /// `git@host:path`, `ssh://user@host:port/path` or `https://host/path`
    pub url: String,
    /// SSH: a personal key or ssh-agent. HTTPS: `Password`, meaning `token`.
    pub auth: Auth,
    /// HTTPS access token; kept only in the personal vault.
    #[serde(default)]
    pub token: Option<String>,
}

impl std::fmt::Debug for GitRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRemote").field("url", &self.url).field("auth", &self.auth).field("token", &self.token.as_ref().map(|_| "…")).finish()
    }
}

impl GitRemote {
    pub fn is_https(&self) -> bool {
        self.url.trim().starts_with("https://") || self.url.trim().starts_with("http://")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshUrl {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl SshUrl {
    pub fn parse(url: &str) -> Result<SshUrl> {
        let url = url.trim();
        let invalid = || Error::Git { message: "invalid repository address".into() };
        let (user_host, port, path) = if let Some(rest) = url.strip_prefix("ssh://") {
            let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
            let (uh, port) = match authority.rsplit_once(':') {
                Some((uh, p)) if !uh.contains(']') || uh.ends_with(']') => (uh, p.parse().map_err(|_| invalid())?),
                _ => (authority, 22),
            };
            (uh, port, format!("/{path}"))
        } else if !url.contains("://") {
            let (uh, path) = url.split_once(':').ok_or_else(invalid)?;
            (uh, 22, path.to_string())
        } else {
            return Err(Error::Git { message: "only SSH repository addresses are supported".into() });
        };
        let (user, host) = match user_host.rsplit_once('@') {
            Some((u, h)) => (u.to_string(), h.to_string()),
            None => ("git".to_string(), user_host.to_string()),
        };
        if host.is_empty() || path.trim_matches('/').is_empty() || path.contains('\n') || path.contains('\0') {
            return Err(invalid());
        }
        Ok(SshUrl { user, host, port, path })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Service {
    UploadPack,
    ReceivePack,
}

impl Service {
    fn name(self) -> &'static str {
        match self {
            Service::UploadPack => "upload-pack",
            Service::ReceivePack => "receive-pack",
        }
    }
}

/// An `https://` repository address, with the login name taken out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpUrl {
    /// Without credentials and trailing slash: `https://host/team/vault.git`.
    pub base: String,
    pub host: String,
    pub user: Option<String>,
}

impl HttpUrl {
    pub fn parse(url: &str) -> Result<HttpUrl> {
        let url = url.trim().trim_end_matches('/');
        let invalid = || Error::Git { message: "invalid repository address".into() };
        let (scheme, rest) = url.split_once("://").ok_or_else(invalid)?;
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        let (user, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u.split(':').next().unwrap_or(u).to_string()).filter(|u| !u.is_empty()), h),
            None => (None, authority),
        };
        let host = hostport.split(':').next().unwrap_or_default().to_string();
        let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
        match scheme {
            "https" => {}
            // A token must never travel in the clear, except to this machine (tests).
            "http" if local => {}
            "http" => return Err(Error::Git { message: "use https, not http".into() }),
            _ => return Err(invalid()),
        }
        if host.is_empty() || path.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(invalid());
        }
        Ok(HttpUrl { base: format!("{scheme}://{hostport}/{path}"), host, user })
    }
}

/// A repository over HTTPS, logged in with a token.
#[derive(Clone)]
pub struct HttpRemote {
    pub url: HttpUrl,
    pub token: Option<String>,
}

/// How to reach a repository.
#[derive(Clone)]
pub enum Remote {
    Ssh { request: ConnectRequest, path: String },
    Http(HttpRemote),
    /// A repository on this machine through an installed `git` (used by tests).
    #[doc(hidden)]
    Process { git: PathBuf, repo: PathBuf },
}

struct Conn {
    stream: Box<dyn Stream>,
    new_known_host: Option<KnownHost>,
    _keep: Box<dyn std::any::Any + Send>,
}

/// A two-way stream to `git-upload-pack` / `git-receive-pack` (SSH or a local process).
async fn connect(remote: &Remote, service: Service) -> Result<Conn> {
    match remote {
        Remote::Ssh { request, path } => {
            let quoted = path.replace('\'', "'\\''");
            let e = ssh::exec(request, &format!("git-{} '{quoted}'", service.name())).await?;
            Ok(Conn { stream: Box::new(e.stream), new_known_host: e.new_known_host, _keep: Box::new(e.session) })
        }
        Remote::Process { git, repo } => {
            let mut child = tokio::process::Command::new(git)
                .arg(service.name())
                .arg(repo)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()?;
            let (stdin, stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
            Ok(Conn { stream: Box::new(tokio::io::join(stdout, stdin)), new_known_host: None, _keep: Box::new(child) })
        }
        Remote::Http(_) => unreachable!("HTTP has no stream"),
    }
}

/// The server's refs.
async fn list_refs(remote: &Remote, service: Service) -> Result<(proto::Advert, Option<KnownHost>)> {
    match remote {
        Remote::Http(h) => Ok((http::refs(h, service).await?, None)),
        _ => {
            let mut c = connect(remote, service).await?;
            Ok((proto::list_refs(&mut c.stream).await?, c.new_known_host.take()))
        }
    }
}

async fn fetch_pack(remote: &Remote, haves: &[Oid]) -> Result<(proto::Fetched, Option<KnownHost>)> {
    match remote {
        Remote::Http(h) => {
            let advert = http::refs(h, Service::UploadPack).await?;
            let plan = proto::plan_fetch(&advert, haves)?;
            let objects = match plan.request {
                None => Default::default(),
                Some(req) => {
                    let body = http::post(h, Service::UploadPack, req).await?;
                    proto::read_fetch_response(&mut &body[..]).await?
                }
            };
            Ok((proto::Fetched { branch: plan.branch, head: plan.head, objects }, None))
        }
        _ => {
            let mut c = connect(remote, Service::UploadPack).await?;
            Ok((proto::fetch(&mut c.stream, haves).await?, c.new_known_host.take()))
        }
    }
}

async fn send_pack(remote: &Remote, branch: &str, old: Oid, new: Oid, pack: &[u8]) -> Result<(Pushed, Option<KnownHost>)> {
    match remote {
        Remote::Http(h) => {
            let advert = http::refs(h, Service::ReceivePack).await?;
            match proto::plan_push(&advert, branch, old, new, pack)? {
                proto::PushPlan::Stale => Ok((Pushed::Stale, None)),
                proto::PushPlan::Send(req, sideband) => {
                    let body = http::post(h, Service::ReceivePack, req).await?;
                    Ok((proto::read_push_response(&mut &body[..], sideband).await?, None))
                }
            }
        }
        _ => {
            let mut c = connect(remote, Service::ReceivePack).await?;
            Ok((proto::push(&mut c.stream, branch, old, new, pack).await?, c.new_known_host.take()))
        }
    }
}

/// Smart HTTP: `GET info/refs`, `POST git-upload-pack` / `git-receive-pack`.
mod http {
    use std::sync::LazyLock;
    use std::time::Duration;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;

    use super::{HttpRemote, Service, proto};
    use crate::{Error, Result};

    /// A pushed or fetched body bigger than this is refused.
    const MAX_BODY: u64 = 512 << 20;

    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            // The system's certificate store, so company proxies with their own CA work.
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .proxy(ureq::Proxy::try_from_env())
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .user_agent(concat!("git/2.0 (detour/", env!("CARGO_PKG_VERSION"), ")"))
            .build()
            .into()
    });

    fn auth(r: &HttpRemote) -> Option<String> {
        // The name hardly matters with a token; some hosts want a real one, given in the address.
        let user = r.url.user.clone().unwrap_or_else(|| "detour".into());
        r.token.as_ref().map(|t| format!("Basic {}", B64.encode(format!("{user}:{t}"))))
    }

    fn status_error(status: u16, host: &str) -> Error {
        match status {
            401 | 403 => Error::Git { message: "token rejected".into() },
            404 => Error::Git { message: "repository not found".into() },
            _ => Error::Git { message: format!("{host} answered {status}") },
        }
    }

    fn net_error(e: ureq::Error, host: &str) -> Error {
        match e {
            ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => Error::Unreachable { endpoint: host.to_string() },
            other => Error::Connect { message: other.to_string() },
        }
    }

    async fn run(f: impl FnOnce() -> Result<Vec<u8>> + Send + 'static) -> Result<Vec<u8>> {
        tokio::task::spawn_blocking(f).await.map_err(|e| Error::Internal { message: e.to_string() })?
    }

    pub async fn refs(r: &HttpRemote, service: Service) -> Result<proto::Advert> {
        let name = format!("git-{}", service.name());
        let url = format!("{}/info/refs?service={name}", r.url.base);
        let (host, header) = (r.url.host.clone(), auth(r));
        let want = format!("application/x-{name}-advertisement");
        let body = run(move || {
            let mut req = AGENT.get(&url);
            if let Some(h) = &header {
                req = req.header("Authorization", h);
            }
            let mut resp = req.call().map_err(|e| net_error(e, &host))?;
            if resp.status() != 200 {
                return Err(status_error(resp.status().as_u16(), &host));
            }
            let smart = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with(&want));
            if !smart {
                return Err(Error::Git { message: "not a smart Git server".into() });
            }
            resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| net_error(e, &host))
        })
        .await?;
        proto::read_http_advert(&body, &name).await
    }

    pub async fn post(r: &HttpRemote, service: Service, request: Vec<u8>) -> Result<Vec<u8>> {
        let name = format!("git-{}", service.name());
        let url = format!("{}/{name}", r.url.base);
        let (host, header) = (r.url.host.clone(), auth(r));
        run(move || {
            let mut req = AGENT
                .post(&url)
                .header("Content-Type", format!("application/x-{name}-request"))
                .header("Accept", format!("application/x-{name}-result"));
            if let Some(h) = &header {
                req = req.header("Authorization", h);
            }
            let mut resp = req.send(&request[..]).map_err(|e| net_error(e, &host))?;
            if resp.status() != 200 {
                return Err(status_error(resp.status().as_u16(), &host));
            }
            resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| net_error(e, &host))
        })
        .await
    }
}

/* ---------- working copy ---------- */

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitState {
    branch: Option<String>,
    /// The remote commit the working copy was last mirrored with.
    head: Option<Oid>,
    /// The files of that commit.
    files: BTreeMap<String, Oid>,
}

const DEFAULT_BRANCH: &str = "refs/heads/main";
/// Path components a team vault uses; anything else in a repository is refused.
const MAX_DEPTH: usize = 4;

fn safe_component(c: &str) -> bool {
    !c.is_empty() && c != "." && c != ".." && c.len() <= 120 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// A team vault's local copy of a Git repository.
pub struct Mirror {
    dir: PathBuf,
}

/// The result of mirroring with the remote.
#[derive(Debug, Default)]
pub struct Synced {
    /// New files arrived from the remote.
    pub pulled: bool,
    /// Local files were pushed.
    pub pushed: bool,
    /// A server key trusted for the first time, to pin.
    pub new_known_host: Option<KnownHost>,
}

impl Mirror {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }

    /// The working copy: what a team vault reads and writes.
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("git.json")
    }

    fn state(&self) -> Result<GitState> {
        match std::fs::read(self.state_path()) {
            Ok(b) => Ok(serde_json::from_slice(&b).map_err(|_| bad("git.json"))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GitState::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_state(&self, st: &GitState) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_file(&self.state_path(), &serde_json::to_vec_pretty(st)?)
    }

    /// Every file in the working copy with its blob id. Our own temp files are skipped.
    fn local_files(&self) -> Result<BTreeMap<String, Oid>> {
        fn walk(root: &Path, dir: &Path, prefix: &str, out: &mut BTreeMap<String, Oid>) -> Result<()> {
            for e in std::fs::read_dir(dir)? {
                let e = e?;
                let name = e.file_name().to_string_lossy().to_string();
                let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                if e.file_type()?.is_dir() {
                    walk(root, &e.path(), &rel, out)?;
                } else if !name.ends_with(".tmp") {
                    out.insert(rel, object::hash(Kind::Blob, &std::fs::read(e.path())?));
                }
            }
            Ok(())
        }
        let mut out = BTreeMap::new();
        if self.files().is_dir() {
            walk(&self.files(), &self.files(), "", &mut out)?;
        }
        Ok(out)
    }

    /// Local files the remote doesn't have yet.
    pub fn pending(&self) -> Result<bool> {
        Ok(self.local_files()? != self.state()?.files)
    }

    /// Is there anything at the remote? Used before creating a vault there.
    pub async fn remote_is_empty(remote: &Remote) -> Result<(bool, Option<KnownHost>)> {
        let (advert, known) = list_refs(remote, Service::UploadPack).await?;
        Ok((!advert.refs.iter().any(|(n, _)| n.starts_with("refs/heads/")), known))
    }

    /// Brings remote changes into the working copy.
    pub async fn pull(&self, remote: &Remote) -> Result<Synced> {
        match self.pull_with(remote, true).await {
            // The server left out something we were supposed to have: fetch everything.
            Err(Error::Git { message }) if message == "incomplete fetch" => self.pull_with(remote, false).await,
            other => other,
        }
    }

    async fn pull_with(&self, remote: &Remote, incremental: bool) -> Result<Synced> {
        let mut st = self.state()?;
        let haves: Vec<Oid> = if incremental { st.head.into_iter().collect() } else { vec![] };
        let (f, known) = fetch_pack(remote, &haves).await?;
        let mut synced = Synced { new_known_host: known, ..Default::default() };
        if f.head == st.head && (f.head.is_none() || f.branch == st.branch) {
            return Ok(synced);
        }
        let files = match f.head {
            None => BTreeMap::new(),
            Some(head) => {
                let (kind, data) = f.objects.get(&head).ok_or_else(|| bad("incomplete fetch"))?;
                if *kind != Kind::Commit {
                    return Err(bad("branch does not point to a commit"));
                }
                let mut out = BTreeMap::new();
                let known = trees_of(&st.files);
                flatten(object::commit_tree(data)?, "", 0, &f.objects, &known, &st.files, &mut out)?;
                out
            }
        };

        // Write what is new; never replace or delete a file we have.
        let local = self.local_files()?;
        let by_id: HashMap<Oid, &String> = local.iter().map(|(p, id)| (*id, p)).collect();
        for (path, id) in &files {
            if local.get(path) == Some(id) {
                continue;
            }
            let content = match f.objects.get(id) {
                Some((Kind::Blob, data)) => data.clone(),
                Some(_) => return Err(bad("expected a file")),
                None => {
                    let from = by_id.get(id).ok_or_else(|| bad("incomplete fetch"))?;
                    std::fs::read(self.files().join(from))?
                }
            };
            let target = if local.contains_key(path) { conflict_name(path, id) } else { path.clone() };
            write_file(&self.files().join(&target), &content)?;
            synced.pulled = true;
        }
        st.branch = f.branch.or(st.branch);
        st.head = f.head;
        st.files = files;
        self.save_state(&st)?;
        Ok(synced)
    }

    /// Sends local files the remote lacks, pulling first when the remote moved on.
    pub async fn push(&self, remote: &Remote) -> Result<Synced> {
        let mut synced = Synced::default();
        for _ in 0..5 {
            let st = self.state()?;
            let local = self.local_files()?;
            if local == st.files {
                return Ok(synced);
            }
            let previous_blobs: BTreeSet<Oid> = st.files.values().copied().collect();
            let previous_trees = trees_of(&st.files);
            let mut objects = Vec::new();
            for (path, id) in &local {
                if !previous_blobs.contains(id) {
                    objects.push((Kind::Blob, std::fs::read(self.files().join(path))?));
                }
            }
            let (root, trees) = build_trees(&local);
            objects.extend(trees.into_iter().filter(|(id, _)| !previous_trees.contains_key(id)).map(|(_, data)| (Kind::Tree, data)));
            let commit = object::encode_commit(root, st.head, crate::model::now_ms() / 1000, "Detour");
            let commit_id = object::hash(Kind::Commit, &commit);
            objects.push((Kind::Commit, commit));

            let branch = st.branch.clone().unwrap_or_else(|| DEFAULT_BRANCH.to_string());
            let (result, known) = send_pack(remote, &branch, st.head.unwrap_or(Oid::ZERO), commit_id, &pack::write(&objects)).await?;
            synced.new_known_host = synced.new_known_host.or(known);
            match result {
                Pushed::Done => {
                    self.save_state(&GitState { branch: Some(branch), head: Some(commit_id), files: local })?;
                    synced.pushed = true;
                    return Ok(synced);
                }
                Pushed::Stale => {
                    let p = self.pull(remote).await?;
                    synced.pulled |= p.pulled;
                }
            }
        }
        Err(Error::Git { message: "the repository keeps changing, try again".into() })
    }

    /// Pull, then push what is pending.
    pub async fn sync(&self, remote: &Remote) -> Result<Synced> {
        let mut s = self.pull(remote).await?;
        let p = self.push(remote).await?;
        s.pushed = p.pushed;
        s.pulled |= p.pulled;
        s.new_known_host = s.new_known_host.or(p.new_known_host);
        Ok(s)
    }
}

/// A second version of `path` that the remote holds, kept beside ours.
fn conflict_name(path: &str, id: &Oid) -> String {
    let short = &id.hex()[..12];
    let (dir, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
    let renamed = match file.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.git-conflict-{short}.{ext}"),
        None => format!("{file}.git-conflict-{short}"),
    };
    if dir.is_empty() { renamed } else { format!("{dir}/{renamed}") }
}

/// Tree objects of a set of files: id -> (directory prefix, encoded tree), root under "".
fn build_trees(files: &BTreeMap<String, Oid>) -> (Oid, Vec<(Oid, Vec<u8>)>) {
    fn build(files: &[(&str, Oid)], out: &mut Vec<(Oid, Vec<u8>)>) -> Oid {
        let mut entries = Vec::new();
        let mut i = 0;
        while i < files.len() {
            let (path, id) = files[i];
            match path.split_once('/') {
                None => {
                    entries.push(TreeEntry { name: path.to_string(), dir: false, id });
                    i += 1;
                }
                Some((dir, _)) => {
                    let start = i;
                    while i < files.len() && files[i].0.split_once('/').is_some_and(|(d, _)| d == dir) {
                        i += 1;
                    }
                    let sub: Vec<(&str, Oid)> = files[start..i].iter().map(|(p, id)| (&p[dir.len() + 1..], *id)).collect();
                    entries.push(TreeEntry { name: dir.to_string(), dir: true, id: build(&sub, out) });
                }
            }
        }
        let data = object::encode_tree(&entries);
        let id = object::hash(Kind::Tree, &data);
        out.push((id, data));
        id
    }
    let list: Vec<(&str, Oid)> = files.iter().map(|(p, id)| (p.as_str(), *id)).collect();
    let mut out = Vec::new();
    let root = build(&list, &mut out);
    (root, out)
}

/// Tree id -> directory prefix, for the trees of a set of files.
fn trees_of(files: &BTreeMap<String, Oid>) -> HashMap<Oid, String> {
    let mut out = HashMap::new();
    let mut dirs: BTreeSet<String> = BTreeSet::from([String::new()]);
    for p in files.keys() {
        let mut acc = String::new();
        for part in p.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            acc = if acc.is_empty() { part.to_string() } else { format!("{acc}/{part}") };
            dirs.insert(acc.clone());
        }
    }
    for d in dirs {
        let sub: BTreeMap<String, Oid> = files
            .iter()
            .filter_map(|(p, id)| if d.is_empty() { Some((p.clone(), *id)) } else { p.strip_prefix(&format!("{d}/")).map(|r| (r.to_string(), *id)) })
            .collect();
        if !sub.is_empty() || d.is_empty() {
            out.insert(build_trees(&sub).0, d);
        }
    }
    out
}

/// Lists the files of tree `id` under `prefix`, taking trees the server left out from what we already had.
fn flatten(id: Oid, prefix: &str, depth: usize, objects: &pack::Objects, known: &HashMap<Oid, String>, known_files: &BTreeMap<String, Oid>, out: &mut BTreeMap<String, Oid>) -> Result<()> {
    if depth >= MAX_DEPTH {
        return Err(bad("unexpected folders in the repository"));
    }
    let entries = match objects.get(&id) {
        Some((Kind::Tree, data)) => object::parse_tree(data)?,
        Some(_) => return Err(bad("expected a folder")),
        None => {
            let old = known.get(&id).ok_or_else(|| bad("incomplete fetch"))?;
            for (p, blob) in known_files {
                let rest = if old.is_empty() { Some(p.as_str()) } else { p.strip_prefix(&format!("{old}/")) };
                if let Some(rest) = rest {
                    let path = if prefix.is_empty() { rest.to_string() } else { format!("{prefix}/{rest}") };
                    out.insert(path, *blob);
                }
            }
            return Ok(());
        }
    };
    for e in entries {
        if !safe_component(&e.name) {
            return Err(bad("unsafe file name in the repository"));
        }
        let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
        if e.dir {
            flatten(e.id, &path, depth + 1, objects, known, known_files, out)?;
        } else {
            out.insert(path, e.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_urls() {
        assert_eq!(SshUrl::parse("git@github.com:acme/vault.git").unwrap(), SshUrl { user: "git".into(), host: "github.com".into(), port: 22, path: "acme/vault.git".into() });
        assert_eq!(SshUrl::parse("ssh://me@git.example.com:2222/srv/vault").unwrap(), SshUrl { user: "me".into(), host: "git.example.com".into(), port: 2222, path: "/srv/vault".into() });
        assert_eq!(SshUrl::parse("github.com:acme/v.git").unwrap().user, "git");
        assert!(SshUrl::parse("https://github.com/acme/v.git").is_err());
        assert!(SshUrl::parse("host:").is_err());
    }

    #[test]
    fn known_trees_cover_every_directory() {
        let files = BTreeMap::from([("vault.json".to_string(), Oid([1; 20])), ("ops/a/00000001.json".to_string(), Oid([2; 20]))]);
        let t = trees_of(&files);
        let dirs: BTreeSet<&str> = t.values().map(String::as_str).collect();
        assert_eq!(dirs, BTreeSet::from(["", "ops", "ops/a"]));
        assert!(t.contains_key(&build_trees(&files).0));
    }

    #[test]
    fn conflict_copies_keep_the_extension() {
        assert_eq!(conflict_name("ops/d/00000002.json", &Oid([0xab; 20])), "ops/d/00000002.git-conflict-abababababab.json");
        assert!(!safe_component("..") && !safe_component("a\\b") && safe_component("00000001.json"));
    }
}
