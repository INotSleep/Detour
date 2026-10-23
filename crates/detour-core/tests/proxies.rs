//! Reaching hosts through a jump host, a SOCKS5 proxy and an HTTP proxy,
//! all in-process: SSH servers that forward like sshd, and tiny proxies.

use std::sync::{Arc, Mutex};

use detour_core::Error;
use detour_core::model::{Auth, HostView, KeyView, KnownHost, Proxy};
use detour_core::ssh::{self, ConnectRequest, Route};
use detour_core::store::{Store, VaultUpdate};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Default)]
struct Sshd {
    /// Where this server was asked to connect on (`ssh -J` asks the jump host).
    forwarded: Arc<Mutex<Vec<String>>>,
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

    async fn channel_open_direct_tcpip(&mut self, channel: Channel<Msg>, host: &str, port: u32, _: &str, _: u32, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        self.forwarded.lock().unwrap().push(format!("{host}:{port}"));
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
}

struct Server {
    port: u16,
    known: KnownHost,
    forwarded: Arc<Mutex<Vec<String>>>,
}

async fn sshd() -> Server {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(server::Config { keys: vec![key.clone()], ..Default::default() });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut srv = Sshd::default();
    let forwarded = srv.forwarded.clone();
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    });
    let pk = key.public_key();
    let known = KnownHost { endpoint: format!("127.0.0.1:{port}"), key: pk.to_openssh().unwrap(), fingerprint: pk.fingerprint(Default::default()).to_string(), added_at: 0 };
    Server { port, known, forwarded }
}

fn request(s: &Server, known: bool, route: Route) -> ConnectRequest {
    let host = detour_core::model::Host {
        id: format!("h{}", s.port),
        group: String::new(),
        label: "t".into(),
        address: "127.0.0.1".into(),
        port: s.port,
        user: "u".into(),
        auth: Auth::Password,
        tags: vec![],
        last_connected: None,
        password: Some("pw".into()),
        proxy: None,
        proxy_password: None,
    };
    ConnectRequest { host, key: None, secret: None, known: known.then(|| s.known.clone()), trust_fingerprint: None, route }
}

/// A SOCKS5 proxy (no auth, or user "p" / password "pp"); counts what it connected to.
async fn socks5(auth: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut c, _)) = l.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut h = [0u8; 2];
                c.read_exact(&mut h).await?;
                let mut methods = vec![0u8; h[1] as usize];
                c.read_exact(&mut methods).await?;
                if auth {
                    c.write_all(&[5, 2]).await?;
                    let mut v = [0u8; 2];
                    c.read_exact(&mut v).await?;
                    let mut user = vec![0u8; v[1] as usize];
                    c.read_exact(&mut user).await?;
                    let mut pl = [0u8; 1];
                    c.read_exact(&mut pl).await?;
                    let mut pass = vec![0u8; pl[0] as usize];
                    c.read_exact(&mut pass).await?;
                    let ok = user == b"p" && pass == b"pp";
                    c.write_all(&[1, if ok { 0 } else { 1 }]).await?;
                    if !ok {
                        return Ok::<_, std::io::Error>(());
                    }
                } else {
                    c.write_all(&[5, 0]).await?;
                }
                let mut req = [0u8; 5];
                c.read_exact(&mut req).await?;
                let mut name = vec![0u8; req[4] as usize];
                c.read_exact(&mut name).await?;
                let mut p = [0u8; 2];
                c.read_exact(&mut p).await?;
                let target = format!("{}:{}", String::from_utf8_lossy(&name), u16::from_be_bytes(p));
                log.lock().unwrap().push(target.clone());
                let mut up = TcpStream::connect(&target).await?;
                c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                tokio::io::copy_bidirectional(&mut c, &mut up).await?;
                Ok(())
            });
        }
    });
    (port, seen)
}

/// An HTTP CONNECT proxy wanting Basic "p:pp".
async fn http_proxy() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut c, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if c.read(&mut b).await? == 0 {
                        return Ok::<_, std::io::Error>(());
                    }
                    head.push(b[0]);
                }
                let text = String::from_utf8_lossy(&head).to_string();
                if !text.contains("Proxy-Authorization: Basic cDpwcA==") {
                    c.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await?;
                    return Ok(());
                }
                let target = text.split_whitespace().nth(1).unwrap_or_default().to_string();
                let Ok(mut up) = TcpStream::connect(&target).await else {
                    c.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                    return Ok(());
                };
                c.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
                tokio::io::copy_bidirectional(&mut c, &mut up).await?;
                Ok(())
            });
        }
    });
    port
}

/// Proves the connection works by running something on it.
async fn works(req: &ConnectRequest) -> Result<ssh::NewKeys, Error> {
    let (session, keys) = ssh::session(req).await?;
    session.subsystem("nothing").await.err();
    assert!(!session.is_closed());
    Ok(keys)
}

#[tokio::test]
async fn through_a_jump_host() {
    let jump = sshd().await;
    let target = sshd().await;
    let req = request(&target, true, Route::Jump(Box::new(request(&jump, true, Route::Direct))));
    works(&req).await.unwrap();
    assert_eq!(jump.forwarded.lock().unwrap().as_slice(), [format!("127.0.0.1:{}", target.port)]);
}

#[tokio::test]
async fn unknown_keys_on_the_way_are_asked_one_by_one_and_all_pinned() {
    let jump = sshd().await;
    let target = sshd().await;
    let mut req = request(&target, false, Route::Jump(Box::new(request(&jump, false, Route::Direct))));
    // The jump host's key first, marked as the jump host's.
    let first = match works(&req).await {
        Err(Error::HostKeyUnknown { fingerprint, via: Some(via) }) => {
            assert_eq!(via, format!("h{}", jump.port));
            fingerprint
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(first, jump.known.fingerprint);
    req.trust_fingerprint = Some(first.clone());
    let second = match works(&req).await {
        Err(Error::HostKeyUnknown { fingerprint, via: None }) => fingerprint,
        other => panic!("{other:?}"),
    };
    assert_eq!(second, target.known.fingerprint);
    req.trust_fingerprint = Some(format!("{first},{second}"));
    let keys = works(&req).await.unwrap();
    let pinned: Vec<&str> = keys.iter().map(|k| k.fingerprint.as_str()).collect();
    assert!(pinned.contains(&first.as_str()) && pinned.contains(&second.as_str()), "{pinned:?}");
}

#[tokio::test]
async fn through_socks5_with_and_without_a_login() {
    let target = sshd().await;
    let (open, seen) = socks5(false).await;
    works(&request(&target, true, Route::Socks5 { address: "127.0.0.1".into(), port: open, user: String::new(), password: None })).await.unwrap();
    assert_eq!(seen.lock().unwrap().as_slice(), [format!("127.0.0.1:{}", target.port)]);

    let (locked, _) = socks5(true).await;
    let with = |pass: &str| request(&target, true, Route::Socks5 { address: "127.0.0.1".into(), port: locked, user: "p".into(), password: Some(pass.into()) });
    // The wrong password first: a working connection would be kept and reused.
    assert!(matches!(works(&with("wrong")).await, Err(Error::Proxy { reason }) if reason == "auth"));
    works(&with("pp")).await.unwrap();
}

#[tokio::test]
async fn through_an_http_proxy() {
    let target = sshd().await;
    let port = http_proxy().await;
    let with = |pass: &str| request(&target, true, Route::Http { address: "127.0.0.1".into(), port, user: "p".into(), password: Some(pass.into()) });
    assert!(matches!(works(&with("no")).await, Err(Error::Proxy { reason }) if reason == "auth"));
    works(&with("pp")).await.unwrap();
    let to_nowhere = request(&Server { port: 1, known: target.known.clone(), forwarded: Default::default() }, true, Route::Http { address: "127.0.0.1".into(), port, user: "p".into(), password: Some("pp".into()) });
    assert!(matches!(works(&to_nowhere).await, Err(Error::Proxy { reason }) if reason == "refused"));
    let dead_proxy = request(&target, true, Route::Socks5 { address: "127.0.0.1".into(), port: 1, user: String::new(), password: None });
    assert!(matches!(works(&dead_proxy).await, Err(Error::Proxy { reason }) if reason == "unreachable"));
}

fn view(id: &str, port: u16, proxy: Option<Proxy>) -> HostView {
    HostView { id: id.into(), group: String::new(), label: id.into(), address: "127.0.0.1".into(), port, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: Some("pw".into()), own: None, proxy, has_proxy_password: false, new_proxy_password: None }
}

#[tokio::test]
async fn a_vault_host_goes_through_another_one() {
    let jump = sshd().await;
    let target = sshd().await;
    let dir = std::env::temp_dir().join(format!("detour-proxy-{}-{}", std::process::id(), jump.port));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Store::open(&dir);
    s.create("m").unwrap();
    let keys: Vec<KeyView> = vec![];
    s.put(VaultUpdate {
        hosts: vec![view("bastion", jump.port, None), view("db", target.port, Some(Proxy::Jump { host_id: "bastion".into() }))],
        keys,
        snippets: vec![],
        tunnels: vec![],
    })
    .unwrap();
    s.record_connection("bastion", vec![jump.known.clone()]).unwrap();
    s.record_connection("db", vec![target.known.clone()]).unwrap();
    let req = s.connect_request("db", None, None).unwrap();
    assert!(matches!(&req.route, Route::Jump(j) if j.host.id == "bastion"));
    works(&req).await.unwrap();

    // A host that goes through itself, directly or round a circle, is refused.
    s.put(VaultUpdate {
        hosts: vec![view("bastion", jump.port, Some(Proxy::Jump { host_id: "db".into() })), view("db", target.port, Some(Proxy::Jump { host_id: "bastion".into() }))],
        keys: vec![],
        snippets: vec![],
        tunnels: vec![],
    })
    .unwrap();
    assert!(matches!(s.connect_request("db", None, None), Err(Error::JumpLoop)));
}
