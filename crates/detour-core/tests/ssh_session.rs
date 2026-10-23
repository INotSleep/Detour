//! End-to-end SSH tests against an in-process russh server: host key pinning,
//! every auth method's prompts and failures, PTY shell I/O and resize.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use detour_core::model::{Auth, Host, KeyAlgorithm, KnownHost};
use detour_core::ssh::{self, ConnectRequest, SessionEvent};
use detour_core::{Error, SecretKind, keys};
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;

#[derive(Default)]
struct Seen {
    pty: Option<(u32, u32)>,
    resized: Option<(u32, u32)>,
}

#[derive(Clone)]
struct TestServer {
    allowed_key: Option<PublicKey>,
    seen: Arc<Mutex<Seen>>,
}

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
        Ok(if user == "u" && password == "pw" { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn auth_publickey(&mut self, _: &str, key: &PublicKey) -> Result<server::Auth, Self::Error> {
        Ok(match &self.allowed_key {
            Some(k) if k.key_data() == key.key_data() => server::Auth::Accept,
            _ => server::Auth::reject(),
        })
    }

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        _: ChannelId,
        _: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.seen.lock().unwrap().pty = Some((cols, rows));
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, b"ready$ ".to_vec())?;
        Ok(())
    }

    async fn window_change_request(&mut self, _: ChannelId, cols: u32, rows: u32, _: u32, _: u32, _: &mut Session) -> Result<(), Self::Error> {
        self.seen.lock().unwrap().resized = Some((cols, rows));
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if data == b"exit\r" {
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
            return Ok(());
        }
        session.data(channel, [b"echo:".as_slice(), data].concat())?;
        Ok(())
    }
}

struct Fixture {
    port: u16,
    host_key: PrivateKey,
    seen: Arc<Mutex<Seen>>,
}

async fn start_server(allowed_key: Option<PublicKey>) -> Fixture {
    let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![host_key.clone()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut srv = TestServer { allowed_key, seen: seen.clone() };
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    });
    Fixture { port, host_key, seen }
}

fn host(port: u16, auth: Auth) -> Host {
    Host {
        id: "h".into(),
        group: "g".into(),
        label: "test".into(),
        address: "127.0.0.1".into(),
        port,
        user: "u".into(),
        auth,
        tags: vec![],
        last_connected: None,
        password: None,
        proxy: None,
        proxy_password: None,
    }
}

fn pinned(f: &Fixture) -> KnownHost {
    let pk = f.host_key.public_key();
    KnownHost {
        endpoint: format!("127.0.0.1:{}", f.port),
        key: pk.to_openssh().unwrap(),
        fingerprint: pk.fingerprint(Default::default()).to_string(),
        added_at: 0,
    }
}

fn req(host: Host, known: Option<KnownHost>) -> ConnectRequest {
    ConnectRequest { host, key: None, secret: None, known, trust_fingerprint: None, route: Default::default() }
}

async fn read_until(events: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>, needle: &str) -> String {
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !out.contains(needle) {
            match events.recv().await {
                Some(SessionEvent::Data(d)) => out.push_str(&String::from_utf8_lossy(&d)),
                other => panic!("unexpected {other:?} while waiting for {needle:?}, got {out:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}, got {out:?}"));
    out
}

#[tokio::test]
async fn first_connect_asks_to_trust_then_pins_and_runs_shell() {
    let f = start_server(None).await;
    let mut h = host(f.port, Auth::Password);
    h.password = Some("pw".into());

    let fp = match ssh::connect(req(h.clone(), None), 80, 24).await {
        Err(Error::HostKeyUnknown { fingerprint, .. }) => fingerprint,
        other => panic!("expected HostKeyUnknown, got {:?}", other.err()),
    };
    assert_eq!(fp, pinned(&f).fingerprint);

    let mut r = req(h, None);
    r.trust_fingerprint = Some(fp.clone());
    let mut c = ssh::connect(r, 100, 30).await.unwrap();
    assert_eq!(c.new_keys[0].fingerprint, fp);
    read_until(&mut c.events, "ready$ ").await;
    assert_eq!(f.seen.lock().unwrap().pty, Some((100, 30)));

    c.handle.input(b"ls\r".to_vec());
    read_until(&mut c.events, "echo:ls").await;

    c.handle.resize(120, 40);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(f.seen.lock().unwrap().resized, Some((120, 40)));

    c.handle.input(b"exit\r".to_vec());
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(SessionEvent::Closed { exit_status }) = c.events.recv().await {
                return exit_status;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(closed, Some(0));
}

#[tokio::test]
async fn changed_host_key_is_refused() {
    let f = start_server(None).await;
    let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let mut known = pinned(&f);
    known.key = other.public_key().to_openssh().unwrap();
    known.fingerprint = other.public_key().fingerprint(Default::default()).to_string();

    match ssh::connect(req(host(f.port, Auth::Password), Some(known.clone())), 80, 24).await {
        Err(Error::HostKeyChanged { expected, actual, .. }) => {
            assert_eq!(expected, known.fingerprint);
            assert_eq!(actual, pinned(&f).fingerprint);
        }
        other => panic!("expected HostKeyChanged, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn password_prompt_wrong_and_right() {
    let f = start_server(None).await;
    let known = Some(pinned(&f));
    let h = host(f.port, Auth::Password);

    match ssh::connect(req(h.clone(), known.clone()), 80, 24).await {
        Err(Error::NeedSecret { secret: SecretKind::Password, prompt }) => assert_eq!(prompt, format!("u@127.0.0.1's password: ")),
        other => panic!("expected NeedSecret, got {:?}", other.err()),
    }

    let mut r = req(h.clone(), known.clone());
    r.secret = Some("wrong".into());
    assert!(matches!(ssh::connect(r, 80, 24).await, Err(Error::AuthFailed { secret: Some(SecretKind::Password) })));

    let mut r = req(h, known);
    r.secret = Some("pw".into());
    let mut c = ssh::connect(r, 80, 24).await.unwrap();
    assert!(c.new_keys.is_empty());
    read_until(&mut c.events, "ready$ ").await;
}

#[tokio::test]
async fn key_with_passphrase_prompt_wrong_and_right() {
    let entry = keys::generate("k", KeyAlgorithm::Ed25519, Some("secret"), false).unwrap();
    let allowed = PublicKey::from_openssh(&entry.public_key).unwrap();
    let f = start_server(Some(allowed)).await;
    let h = host(f.port, Auth::Key { key_id: entry.id.clone() });
    let mk = |secret: Option<&str>| ConnectRequest { host: h.clone(), key: Some(entry.clone()), secret: secret.map(str::to_string), known: Some(pinned(&f)), trust_fingerprint: None, route: Default::default() };

    assert!(matches!(ssh::connect(mk(None), 80, 24).await, Err(Error::NeedSecret { secret: SecretKind::Passphrase, .. })));
    assert!(matches!(ssh::connect(mk(Some("nope")), 80, 24).await, Err(Error::AuthFailed { secret: Some(SecretKind::Passphrase) })));
    let mut c = ssh::connect(mk(Some("secret")), 80, 24).await.unwrap();
    read_until(&mut c.events, "ready$ ").await;
}

#[tokio::test]
async fn key_not_accepted_by_server_fails_cleanly() {
    let entry = keys::generate("k", KeyAlgorithm::Ed25519, None, false).unwrap();
    let f = start_server(None).await;
    let r = ConnectRequest { host: host(f.port, Auth::Key { key_id: entry.id.clone() }), key: Some(entry), secret: None, known: Some(pinned(&f)), trust_fingerprint: None, route: Default::default() };
    assert!(matches!(ssh::connect(r, 80, 24).await, Err(Error::AuthFailed { secret: None })));
}

#[tokio::test]
async fn unreachable_host_reports_connect_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(matches!(ssh::connect(req(host(port, Auth::Agent), None), 80, 24).await, Err(Error::Connect { .. } | Error::Unreachable { .. })));
}
