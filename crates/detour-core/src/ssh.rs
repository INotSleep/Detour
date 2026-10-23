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
    /// Fingerprint the user agreed to trust for a host seen for the first time.
    pub trust_fingerprint: Option<String>,
}

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
    /// Set when the server key was trusted for the first time and should be pinned.
    pub new_known_host: Option<KnownHost>,
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
            None => self.trust_fingerprint.as_deref() == Some(fingerprint(&key).as_str()),
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

/// Connects, checks the host key and authenticates.
async fn open(req: &ConnectRequest) -> Result<(client::Handle<Client>, Option<KnownHost>, Forwards)> {
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

    let addr = (host.address.as_str(), host.port);
    let connecting = tokio::time::timeout(Duration::from_secs(15), client::connect(config, addr, handler));
    let mut session = match connecting.await {
        Err(_) => return Err(Error::Unreachable { endpoint: endpoint(host) }),
        Ok(Ok(s)) => s,
        Ok(Err(russh::Error::UnknownKey)) => {
            let actual = seen.lock().unwrap().clone().ok_or_else(|| connect_error("server sent no host key"))?;
            return Err(match pinned {
                Some(p) => Error::HostKeyChanged { expected: fingerprint(&p), actual: fingerprint(&actual) },
                None => Error::HostKeyUnknown { fingerprint: fingerprint(&actual) },
            });
        }
        Ok(Err(e)) => return Err(connect_error(e)),
    };

    let new_known_host = match (&req.known, seen.lock().unwrap().clone()) {
        (None, Some(key)) => Some(KnownHost {
            endpoint: endpoint(host),
            key: key.to_openssh().map_err(connect_error)?,
            fingerprint: fingerprint(&key),
            added_at: now_ms(),
        }),
        _ => None,
    };

    if let Err(e) = authenticate(&mut session, req).await {
        let _ = session.disconnect(Disconnect::ByApplication, "", "en").await;
        return Err(e);
    }
    Ok((session, new_known_host, forwards))
}

/// A connection kept open for more commands (Git sync runs every few seconds, tunnels share it).
#[derive(Clone)]
pub struct Session(Arc<SessionInner>);

struct SessionInner {
    handle: client::Handle<Client>,
    forwards: Forwards,
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
    format!("{}@{}|{login}|{pinned}", h.user, endpoint(h))
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
pub async fn session(req: &ConnectRequest) -> Result<(Session, Option<KnownHost>)> {
    let key = pool_key(req);
    if let Some(session) = pooled(&key) {
        return Ok((session, None));
    }
    let (handle, new_known_host, forwards) = open(req).await?;
    let session = Session(Arc::new(SessionInner { handle, forwards }));
    POOL.lock().unwrap_or_else(|e| e.into_inner()).insert(key, Pooled { session: session.clone(), used: Instant::now() });
    Ok((session, new_known_host))
}

/// Runs a command, over a kept connection when there is one.
pub async fn exec(req: &ConnectRequest, command: &str) -> Result<Exec> {
    let (session, new_known_host) = session(req).await?;
    match run(&session, command).await {
        Ok(stream) => Ok(Exec { stream, new_known_host, session }),
        // A kept connection the server dropped in the meantime: connect again.
        Err(_) if new_known_host.is_none() => {
            POOL.lock().unwrap_or_else(|e| e.into_inner()).remove(&pool_key(req));
            let (session, new_known_host) = self::session(req).await?;
            let stream = run(&session, command).await?;
            Ok(Exec { stream, new_known_host, session })
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
    let (session, new_known_host, _) = open(&req).await?;
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
    });

    Ok(Connected { handle: SessionHandle { tx: cmd_tx }, events: ev_rx, new_known_host })
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
