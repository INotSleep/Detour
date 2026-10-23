//! Interactive SSH sessions.
//!
//! `connect` verifies the server key against the pinned one (trust on first
//! use), authenticates, opens a PTY shell and returns a handle plus a stream of
//! output events. When something needs the user — an unknown host key, a
//! missing password or passphrase — it fails with a dedicated `Error` and the
//! UI calls `connect` again with the answer.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::ssh_key::HashAlg;
use russh::keys::{PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc;

use crate::model::{Auth, Host, KeyEntry, KnownHost, now_ms};
use crate::store::endpoint;
use crate::{Error, Result, SecretKind, keys};

#[derive(Clone)]
pub struct ConnectRequest {
    pub host: Host,
    pub key: Option<KeyEntry>,
    /// Password or passphrase the user just typed, if any.
    pub secret: Option<String>,
    pub known: Option<KnownHost>,
    /// Fingerprints the user agreed to trust for hosts seen for the first time,
    /// comma separated: this host's and those of hosts on the way.
    pub trust_fingerprint: Option<String>,
    pub route: Route,
}

/// How the connection to a host is made.
#[derive(Clone, Default)]
pub enum Route {
    #[default]
    Direct,
    /// A channel of an SSH connection to another host (`ssh -J`).
    Jump(Box<ConnectRequest>),
    Socks5 { address: String, port: u16, user: String, password: Option<String> },
    Http { address: String, port: u16, user: String, password: Option<String> },
}

impl Route {
    /// Tells kept connections apart by the way they were made.
    fn key(&self) -> String {
        match self {
            Route::Direct => String::new(),
            Route::Jump(j) => format!("|via {}", pool_key(j)),
            Route::Socks5 { address, port, user, .. } => format!("|socks5 {user}@{address}:{port}"),
            Route::Http { address, port, user, .. } => format!("|http {user}@{address}:{port}"),
        }
    }
}

/// Server keys trusted for the first time during a login, to pin.
pub type NewKeys = Vec<KnownHost>;

#[derive(Debug)]
pub enum SessionEvent {
    Data(Vec<u8>),
    Closed { exit_status: Option<u32> },
}

enum Command {
    Input(Vec<u8>),
    Resize(u32, u32),
    Close,
}

/// Cheap to clone; dropping every clone closes the session.
#[derive(Clone)]
pub struct SessionHandle {
    tx: mpsc::UnboundedSender<Command>,
}

impl SessionHandle {
    pub fn input(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Command::Input(bytes));
    }
    pub fn resize(&self, cols: u32, rows: u32) {
        let _ = self.tx.send(Command::Resize(cols, rows));
    }
    pub fn close(&self) {
        let _ = self.tx.send(Command::Close);
    }
}

pub struct Connected {
    pub handle: SessionHandle,
    pub events: mpsc::UnboundedReceiver<SessionEvent>,
    /// Server keys trusted for the first time, to pin.
    pub new_keys: NewKeys,
    /// The fingerprint of this host's own key, if it is among them.
    pub pinned_fingerprint: Option<String>,
}

struct Client {
    pinned: Option<PublicKey>,
    trust_fingerprint: Option<String>,
    seen: Arc<Mutex<Option<PublicKey>>>,
    forwards: Forwards,
}

/// Where connections to a port the server listens on for us (`ssh -R`) go.
#[derive(Clone)]
pub struct RemoteForward {
    pub host: String,
    pub port: u16,
    /// Told +1 when a connection opens and -1 when it ends.
    pub on_connection: Arc<dyn Fn(i32) + Send + Sync>,
}

/// Server port -> where its connections go.
type Forwards = Arc<Mutex<HashMap<u32, RemoteForward>>>;

fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        *self.seen.lock().unwrap() = Some(key.clone());
        Ok(match &self.pinned {
            Some(pinned) => pinned.key_data() == key.key_data(),
            None => self.trust_fingerprint.as_deref().is_some_and(|t| t.split(',').any(|f| f == fingerprint(&key))),
        })
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> std::result::Result<(), Self::Error> {
        // A port we did not ask for: dropping `reply` refuses the channel.
        let Some(fwd) = self.forwards.lock().unwrap().get(&connected_port).cloned() else { return Ok(()) };
        reply.accept().await;
        tokio::spawn(async move {
            if let Ok(mut tcp) = tokio::net::TcpStream::connect((fwd.host.as_str(), fwd.port)).await {
                (fwd.on_connection)(1);
                let mut ch = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut ch, &mut tcp).await;
                (fwd.on_connection)(-1);
            }
        });
        Ok(())
    }
}

fn connect_error(e: impl std::fmt::Display) -> Error {
    Error::Connect { message: e.to_string() }
}

/// Marks an error from the host a connection goes through as about that host.
fn via(e: Error, id: &str) -> Error {
    let id = Some(id.to_string());
    match e {
        Error::HostKeyUnknown { fingerprint, via: None } => Error::HostKeyUnknown { fingerprint, via: id },
        Error::HostKeyChanged { expected, actual, via: None } => Error::HostKeyChanged { expected, actual, via: id },
        Error::NeedSecret { .. } | Error::AuthFailed { secret: Some(_) } => Error::JumpNeedsLogin { via: id.unwrap_or_default() },
        other => other,
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Connects (through a jump host or proxy if the route says so), checks the host key and authenticates.
/// Returns the connection, keys to pin, remote forwards, and a jump host's connection to keep open.
async fn open(req: &ConnectRequest) -> Result<(client::Handle<Client>, NewKeys, Forwards, Option<Session>)> {
    let host = &req.host;
    let pinned = match &req.known {
        Some(k) => Some(PublicKey::from_openssh(&k.key).map_err(|e| Error::Internal { message: format!("pinned key: {e}") })?),
        None => None,
    };
    let seen = Arc::new(Mutex::new(None));
    let forwards = Forwards::default();
    let handler = Client { pinned: pinned.clone(), trust_fingerprint: req.trust_fingerprint.clone(), seen: seen.clone(), forwards: forwards.clone() };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let mut new_keys = NewKeys::new();
    let mut keep = None;
    let unreachable = || Error::Unreachable { endpoint: endpoint(host) };
    let connected = match &req.route {
        Route::Direct => {
            let addr = (host.address.as_str(), host.port);
            tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, addr, handler)).await.map_err(|_| unreachable())?
        }
        Route::Jump(jump) => {
            // The same answers to "trust this key?" hold on the way.
            let mut jump = (**jump).clone();
            jump.trust_fingerprint = req.trust_fingerprint.clone();
            let (via_session, keys) = Box::pin(session(&jump)).await.map_err(|e| via(e, &jump.host.id))?;
            // Its new key is pinned only if this login succeeds: until then, don't keep the
            // connection for others, or the next attempt would reuse it and never report the key.
            if !keys.is_empty() {
                forget_pooled(&jump);
            }
            new_keys.extend(keys);
            let from = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
            let stream = via_session.connect_tcp(&host.address, host.port, from).await.map_err(|_| unreachable())?;
            keep = Some(via_session);
            tokio::time::timeout(CONNECT_TIMEOUT, client::connect_stream(config, stream, handler)).await.map_err(|_| unreachable())?
        }
        Route::Socks5 { address, port, user, password } | Route::Http { address, port, user, password } => {
            let socks = matches!(req.route, Route::Socks5 { .. });
            let tcp = async {
                if socks {
                    crate::proxy::socks5(address, *port, user, password.as_deref(), &host.address, host.port).await
                } else {
                    crate::proxy::http_connect(address, *port, user, password.as_deref(), &host.address, host.port).await
                }
            };
            let tcp = tokio::time::timeout(CONNECT_TIMEOUT, tcp).await.map_err(|_| Error::Proxy { reason: "unreachable".into() })??;
            tokio::time::timeout(CONNECT_TIMEOUT, client::connect_stream(config, tcp, handler)).await.map_err(|_| unreachable())?
        }
    };
    let mut session = match connected {
        Ok(s) => s,
        Err(russh::Error::UnknownKey) => {
            let actual = seen.lock().unwrap().clone().ok_or_else(|| connect_error("server sent no host key"))?;
            return Err(match pinned {
                Some(p) => Error::HostKeyChanged { expected: fingerprint(&p), actual: fingerprint(&actual), via: None },
                None => Error::HostKeyUnknown { fingerprint: fingerprint(&actual), via: None },
            });
        }
        Err(e) => return Err(connect_error(e)),
    };

    if let (None, Some(key)) = (&req.known, seen.lock().unwrap().clone()) {
        new_keys.push(KnownHost { endpoint: endpoint(host), key: key.to_openssh().map_err(connect_error)?, fingerprint: fingerprint(&key), added_at: now_ms() });
    }

    if let Err(e) = authenticate(&mut session, req).await {
        let _ = session.disconnect(Disconnect::ByApplication, "", "en").await;
        return Err(e);
    }
    Ok((session, new_keys, forwards, keep))
}

/// A connection kept open for more commands (Git sync runs every few seconds, tunnels share it).
#[derive(Clone)]
pub struct Session(Arc<SessionInner>);

struct SessionInner {
    handle: client::Handle<Client>,
    forwards: Forwards,
    /// The connection to a jump host this one goes through.
    _via: Option<Session>,
}

impl Session {
    pub fn is_closed(&self) -> bool {
        self.0.handle.is_closed()
    }

    /// A TCP connection made by the server to `host:port` (`ssh -L`, `ssh -D`).
    pub async fn connect_tcp(&self, host: &str, port: u16, from: std::net::SocketAddr) -> Result<russh::ChannelStream<client::Msg>> {
        let channel = self.0.handle.channel_open_direct_tcpip(host, port as u32, from.ip().to_string(), from.port() as u32).await.map_err(connect_error)?;
        Ok(channel.into_stream())
    }

    /// Asks the server to listen on its loopback `port` and send connections to `fwd` (`ssh -R`).
    pub async fn listen_remote(&self, port: u16, fwd: RemoteForward) -> Result<()> {
        self.0.forwards.lock().unwrap().insert(port as u32, fwd);
        if let Err(e) = self.0.handle.tcpip_forward("localhost", port as u32).await {
            self.0.forwards.lock().unwrap().remove(&(port as u32));
            return Err(match e {
                russh::Error::RequestDenied => Error::ForwardRefused { port },
                other => connect_error(other),
            });
        }
        Ok(())
    }

    /// A subsystem such as `sftp` on a new channel.
    pub async fn subsystem(&self, name: &str) -> Result<russh::ChannelStream<client::Msg>> {
        let channel = self.0.handle.channel_open_session().await.map_err(connect_error)?;
        channel.request_subsystem(true, name).await.map_err(connect_error)?;
        Ok(channel.into_stream())
    }

    pub async fn stop_remote(&self, port: u16) {
        self.0.forwards.lock().unwrap().remove(&(port as u32));
        let _ = self.0.handle.cancel_tcpip_forward("localhost", port as u32).await;
    }
}

struct Pooled {
    session: Session,
    used: Instant,
}

/// Open connections by target and login; dropped after a while unused, and on lock.
static POOL: LazyLock<Mutex<HashMap<String, Pooled>>> = LazyLock::new(Default::default);
const IDLE: Duration = Duration::from_secs(300);

fn pool_key(req: &ConnectRequest) -> String {
    let h = &req.host;
    let login = match &h.auth {
        Auth::Key { key_id } => format!("key:{key_id}:{}", req.key.as_ref().map(|k| k.fingerprint.as_str()).unwrap_or_default()),
        Auth::Agent => "agent".into(),
        Auth::Password => "password".into(),
    };
    // A different pinned server key means a different server: never reuse.
    let pinned = req.known.as_ref().map(|k| k.fingerprint.as_str()).unwrap_or_default();
    format!("{}@{}|{login}|{pinned}{}", h.user, endpoint(h), req.route.key())
}

fn pooled(key: &str) -> Option<Session> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.retain(|_, p| p.used.elapsed() < IDLE && !p.session.is_closed());
    let p = pool.get_mut(key)?;
    p.used = Instant::now();
    Some(p.session.clone())
}

/// Drops the kept connection for this target, e.g. after the server closed it.
pub fn forget_pooled(req: &ConnectRequest) {
    POOL.lock().unwrap_or_else(|e| e.into_inner()).remove(&pool_key(req));
}

/// Closes every kept connection (when the vault locks).
pub fn close_pooled() {
    POOL.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// A command's stdin and stdout as one stream (Git over SSH).
pub struct Exec {
    pub stream: russh::ChannelStream<client::Msg>,
    pub new_known_host: Option<KnownHost>,
    /// Keeps the connection open while the stream is used.
    pub session: Session,
}

async fn run(session: &Session, command: &str) -> Result<russh::ChannelStream<client::Msg>> {
    let channel = session.0.handle.channel_open_session().await.map_err(connect_error)?;
    channel.exec(true, command).await.map_err(connect_error)?;
    Ok(channel.into_stream())
}

/// A kept connection for this target and login, or a new one (then also kept).
pub async fn session(req: &ConnectRequest) -> Result<(Session, NewKeys)> {
    let key = pool_key(req);
    if let Some(session) = pooled(&key) {
        return Ok((session, NewKeys::new()));
    }
    let (handle, new_keys, forwards, via) = open(req).await?;
    let session = Session(Arc::new(SessionInner { handle, forwards, _via: via }));
    POOL.lock().unwrap_or_else(|e| e.into_inner()).insert(key, Pooled { session: session.clone(), used: Instant::now() });
    Ok((session, new_keys))
}

/// Runs a command, over a kept connection when there is one.
pub async fn exec(req: &ConnectRequest, command: &str) -> Result<Exec> {
    let (session, new_keys) = session(req).await?;
    match run(&session, command).await {
        Ok(stream) => Ok(Exec { stream, new_known_host: new_keys.into_iter().last(), session }),
        // A kept connection the server dropped in the meantime: connect again.
        Err(_) if new_keys.is_empty() => {
            POOL.lock().unwrap_or_else(|e| e.into_inner()).remove(&pool_key(req));
            let (session, new_keys) = self::session(req).await?;
            let stream = run(&session, command).await?;
            Ok(Exec { stream, new_known_host: new_keys.into_iter().last(), session })
        }
        Err(e) => Err(e),
    }
}

/// Is a connection kept for this target and login (for tests)?
#[doc(hidden)]
pub fn pooled_count_for(req: &ConnectRequest) -> usize {
    POOL.lock().unwrap_or_else(|e| e.into_inner()).keys().filter(|k| **k == pool_key(req)).count()
}

pub async fn connect(req: ConnectRequest, cols: u32, rows: u32) -> Result<Connected> {
    let (session, new_keys, _, via) = open(&req).await?;
    let channel = session.channel_open_session().await.map_err(connect_error)?;
    channel.request_pty(false, "xterm-256color", cols, rows, 0, 0, &[]).await.map_err(connect_error)?;
    channel.request_shell(false).await.map_err(connect_error)?;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut channel = channel;
        let mut exit_status = None;
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(Command::Input(bytes)) => { if channel.data(&bytes[..]).await.is_err() { break; } }
                    Some(Command::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                    Some(Command::Close) | None => { let _ = channel.close().await; break; }
                },
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) => { let _ = ev_tx.send(SessionEvent::Data(data.to_vec())); }
                    Some(ChannelMsg::ExtendedData { data, .. }) => { let _ = ev_tx.send(SessionEvent::Data(data.to_vec())); }
                    Some(ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                    Some(_) => {}
                },
            }
        }
        let _ = ev_tx.send(SessionEvent::Closed { exit_status });
        let _ = session.disconnect(Disconnect::ByApplication, "", "en").await;
        drop(via);
    });

    let pinned_fingerprint = new_keys.iter().find(|k| k.endpoint == endpoint(&req.host)).map(|k| k.fingerprint.clone());
    Ok(Connected { handle: SessionHandle { tx: cmd_tx }, events: ev_rx, new_keys, pinned_fingerprint })
}

async fn authenticate(session: &mut client::Handle<Client>, req: &ConnectRequest) -> Result<()> {
    let host = &req.host;
    match &host.auth {
        Auth::Password => {
            let Some(password) = req.secret.clone().or_else(|| host.password.clone()) else {
                return Err(Error::NeedSecret { secret: SecretKind::Password, prompt: format!("{}@{}'s password: ", host.user, host.address) });
            };
            let res = session.authenticate_password(&host.user, password).await.map_err(connect_error)?;
            if !res.success() {
                return Err(Error::AuthFailed { secret: Some(SecretKind::Password) });
            }
        }
        Auth::Key { .. } => {
            let entry = req.key.as_ref().ok_or_else(|| Error::NotFound { what: "key".into() })?;
            let key = match keys::unlock(entry, req.secret.as_deref()) {
                Ok(k) => k,
                Err(Error::PassphraseRequired) => {
                    return Err(Error::NeedSecret { secret: SecretKind::Passphrase, prompt: format!("Enter passphrase for key '{}': ", entry.name) });
                }
                Err(Error::WrongPassphrase) => return Err(Error::AuthFailed { secret: Some(SecretKind::Passphrase) }),
                Err(e) => return Err(e),
            };
            let hash = session.best_supported_rsa_hash().await.map_err(connect_error)?.flatten();
            let res = session.authenticate_publickey(&host.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await.map_err(connect_error)?;
            if !res.success() {
                return Err(Error::AuthFailed { secret: None });
            }
        }
        Auth::Agent => authenticate_with_agent(session, &host.user).await?,
    }
    Ok(())
}

async fn authenticate_with_agent(session: &mut client::Handle<Client>, user: &str) -> Result<()> {
    #[cfg(windows)]
    let agent = match russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(a) => a.dynamic(),
        Err(_) => russh::keys::agent::client::AgentClient::connect_pageant().await.map_err(|_| no_agent())?.dynamic(),
    };
    #[cfg(not(windows))]
    let agent = russh::keys::agent::client::AgentClient::connect_env().await.map_err(|_| no_agent())?.dynamic();

    let mut agent = agent;
    let identities = agent.request_identities().await.map_err(connect_error)?;
    let hash = session.best_supported_rsa_hash().await.map_err(connect_error)?.flatten();
    for id in identities {
        let key = match id {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => key,
            russh::keys::agent::AgentIdentity::Certificate { .. } => continue,
        };
        let res = session.authenticate_publickey_with(user, key, hash, &mut agent).await.map_err(connect_error)?;
        if res.success() {
            return Ok(());
        }
    }
    Err(Error::AuthFailed { secret: None })
}

fn no_agent() -> Error {
    Error::AgentUnavailable
}
