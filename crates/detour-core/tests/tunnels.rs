//! Tunnels against an in-process SSH server that forwards ports like sshd.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use detour_core::Error;
use detour_core::model::{Auth, Host, KnownHost, TunnelKind};
use detour_core::ssh::ConnectRequest;
use detour_core::tunnel::{TunnelSpec, TunnelState, Tunnels};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Default)]
struct Sshd {
    /// Refuse `ssh -R`, like `AllowTcpForwarding local`.
    refuse_remote: bool,
    /// The last client's session, to cut it off.
    handle: Arc<Mutex<Option<server::Handle>>>,
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

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        *self.handle.lock().unwrap() = Some(session.handle());
        match TcpStream::connect((host, port as u16)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut ch, &mut tcp).await;
                });
            }
            Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    async fn tcpip_forward(&mut self, address: &str, port: &mut u32, session: &mut Session) -> Result<bool, Self::Error> {
        if self.refuse_remote {
            return Ok(false);
        }
        let Ok(listener) = TcpListener::bind(("127.0.0.1", *port as u16)).await else { return Ok(false) };
        let (handle, address, port) = (session.handle(), address.to_string(), *port);
        tokio::spawn(async move {
            while let Ok((mut sock, from)) = listener.accept().await {
                let Ok(ch) = handle.channel_open_forwarded_tcpip(address.clone(), port, from.ip().to_string(), from.port() as u32).await else { continue };
                tokio::spawn(async move {
                    let mut ch = ch.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut ch, &mut sock).await;
                });
            }
        });
        Ok(true)
    }
}

struct Fixture {
    req: ConnectRequest,
    handle: Arc<Mutex<Option<server::Handle>>>,
}

async fn sshd(refuse_remote: bool) -> Fixture {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(server::Config { keys: vec![key.clone()], ..Default::default() });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut srv = Sshd { refuse_remote, ..Default::default() };
    let handle = srv.handle.clone();
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    });
    let pk = key.public_key();
    let known = KnownHost { endpoint: format!("127.0.0.1:{port}"), key: pk.to_openssh().unwrap(), fingerprint: pk.fingerprint(Default::default()).to_string(), added_at: 0 };
    let host = Host { id: "h".into(), group: String::new(), label: "t".into(), address: "127.0.0.1".into(), port, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, password: Some("pw".into()), proxy: None, proxy_password: None };
    Fixture { req: ConnectRequest { host, key: None, secret: None, known: Some(known), trust_fingerprint: None, route: Default::default() }, handle }
}

/// A TCP echo service; returns its port.
async fn echo() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn roundtrip(s: &mut TcpStream, msg: &[u8]) -> Vec<u8> {
    s.write_all(msg).await.unwrap();
    let mut buf = vec![0u8; msg.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut buf)).await.unwrap().unwrap();
    buf
}

fn status(t: &Tunnels, key: &str) -> Option<detour_core::tunnel::TunnelStatus> {
    t.statuses().into_iter().find(|s| s.key == key)
}

async fn eventually(mut ok: impl FnMut() -> bool) {
    for _ in 0..50 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn local_forward_reaches_a_service_behind_the_server() {
    let f = sshd(false).await;
    let target = echo().await;
    let port = free_port();
    let t = Tunnels::new();
    t.start("v/l", TunnelSpec { kind: TunnelKind::Local, bind_port: port, target: Some(format!("127.0.0.1:{target}")) }, &f.req).await.unwrap();

    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    assert_eq!(roundtrip(&mut c, b"hello").await, b"hello");
    eventually(|| status(&t, "v/l").unwrap().connections == 1).await;
    drop(c);
    eventually(|| status(&t, "v/l").unwrap().connections == 0).await;

    // A target the server can't reach shows up on the tunnel, which stays open.
    let bad_port = free_port();
    t.start("v/bad", TunnelSpec { kind: TunnelKind::Local, bind_port: bad_port, target: Some(format!("127.0.0.1:{}", free_port())) }, &f.req).await.unwrap();
    let _c = TcpStream::connect(("127.0.0.1", bad_port)).await.unwrap();
    eventually(|| status(&t, "v/bad").unwrap().error.is_some()).await;
    assert_eq!(status(&t, "v/bad").unwrap().state, TunnelState::On);

    // Stopping frees the port.
    t.stop("v/l");
    assert!(status(&t, "v/l").is_none());
    eventually(|| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()).await;
}

#[tokio::test]
async fn a_busy_port_is_reported_before_connecting() {
    let f = sshd(false).await;
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let err = Tunnels::new().start("v/x", TunnelSpec { kind: TunnelKind::Dynamic, bind_port: port, target: None }, &f.req).await.unwrap_err();
    assert!(matches!(err, Error::PortBusy { port: p } if p == port), "{err:?}");
}

#[tokio::test]
async fn socks_proxy_connects_from_the_server() {
    let f = sshd(false).await;
    let target = echo().await;
    let port = free_port();
    let t = Tunnels::new();
    t.start("v/d", TunnelSpec { kind: TunnelKind::Dynamic, bind_port: port, target: None }, &f.req).await.unwrap();

    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    c.write_all(&[5, 1, 0]).await.unwrap();
    let mut hello = [0u8; 2];
    c.read_exact(&mut hello).await.unwrap();
    assert_eq!(hello, [5, 0]);
    // CONNECT by domain name: "localhost".
    let mut req = vec![5, 1, 0, 3, 9];
    req.extend_from_slice(b"localhost");
    req.extend_from_slice(&target.to_be_bytes());
    c.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    c.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "succeeded");
    assert_eq!(roundtrip(&mut c, b"through socks").await, b"through socks");
}

#[tokio::test]
async fn remote_forward_brings_server_connections_here() {
    let f = sshd(false).await;
    let target = echo().await;
    let server_port = free_port();
    let t = Tunnels::new();
    t.start("v/r", TunnelSpec { kind: TunnelKind::Remote, bind_port: server_port, target: Some(format!("127.0.0.1:{target}")) }, &f.req).await.unwrap();

    // Someone on the server connects to its port.
    let mut c = TcpStream::connect(("127.0.0.1", server_port)).await.unwrap();
    assert_eq!(roundtrip(&mut c, b"from the server").await, b"from the server");
    eventually(|| status(&t, "v/r").unwrap().connections == 1).await;
}

#[tokio::test]
async fn a_refused_remote_forward_is_an_error() {
    let f = sshd(true).await;
    let port = free_port();
    let err = Tunnels::new().start("v/r", TunnelSpec { kind: TunnelKind::Remote, bind_port: port, target: Some("127.0.0.1:1".into()) }, &f.req).await.unwrap_err();
    assert!(matches!(err, Error::ForwardRefused { port: p } if p == port), "{err:?}");
}

#[tokio::test]
async fn a_lost_connection_stops_the_tunnel_with_an_error() {
    let f = sshd(false).await;
    let target = echo().await;
    let port = free_port();
    let t = Tunnels::new();
    t.start("v/l", TunnelSpec { kind: TunnelKind::Local, bind_port: port, target: Some(format!("127.0.0.1:{target}")) }, &f.req).await.unwrap();
    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    roundtrip(&mut c, b"x").await;

    let handle = f.handle.lock().unwrap().clone().unwrap();
    let _ = handle.disconnect(russh::Disconnect::ByApplication, "bye".into(), "en".into()).await;
    eventually(|| status(&t, "v/l").unwrap().state == TunnelState::Error).await;
    // Its local port is free again.
    eventually(|| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()).await;
}

#[tokio::test]
async fn tunnels_to_one_host_share_its_connection() {
    let f = sshd(false).await;
    let target = echo().await;
    let t = Tunnels::new();
    let (p1, p2) = (free_port(), free_port());
    t.start("v/1", TunnelSpec { kind: TunnelKind::Local, bind_port: p1, target: Some(format!("127.0.0.1:{target}")) }, &f.req).await.unwrap();
    t.start("v/2", TunnelSpec { kind: TunnelKind::Dynamic, bind_port: p2, target: None }, &f.req).await.unwrap();
    assert_eq!(detour_core::ssh::pooled_count_for(&f.req), 1);
    t.stop_all();
    assert!(t.statuses().is_empty());
}
