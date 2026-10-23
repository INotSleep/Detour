//! SSH tunnels: a local port to a place the server reaches (`ssh -L`), a port
//! on the server to a place this computer reaches (`ssh -R`), and a SOCKS5
//! proxy whose connections leave from the server (`ssh -D`).
//!
//! Local ports listen on 127.0.0.1 and the server's on its loopback, so a
//! tunnel never opens anything to the network around either end. Tunnels to
//! one host share its kept SSH connection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::model::TunnelKind;
use crate::ssh::{self, ConnectRequest, RemoteForward, Session};
use crate::{Error, Result};

pub struct TunnelSpec {
    pub kind: TunnelKind,
    pub bind_port: u16,
    /// `host:port`; not used by a SOCKS proxy.
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TunnelState {
    On,
    /// Stopped by itself: the connection was lost.
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub key: String,
    pub state: TunnelState,
    /// Connections going through it now.
    pub connections: u32,
    /// Why it stopped, or why the last connection through it failed (a serialized `Error`).
    pub error: Option<serde_json::Value>,
}

/// `host:port` or `[v6]:port`.
pub fn parse_target(t: &str) -> Result<(String, u16)> {
    let bad = || Error::Connect { message: format!("invalid target {t:?}, expected host:port") };
    let t = t.trim();
    let (host, port) = t.rsplit_once(':').ok_or_else(bad)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port: u16 = port.parse().map_err(|_| bad())?;
    if host.is_empty() || port == 0 {
        return Err(bad());
    }
    Ok((host.to_string(), port))
}

struct Entry {
    status: TunnelStatus,
    run: u64,
    stop: watch::Sender<bool>,
}

/// Running tunnels by key (the UI uses `<vault>/<tunnel>`).
#[derive(Default, Clone)]
pub struct Tunnels {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
}

static RUNS: AtomicU64 = AtomicU64::new(1);

/// Updates one run's status; a newer run of the same tunnel is left alone.
#[derive(Clone)]
struct Reporter {
    map: Arc<Mutex<HashMap<String, Entry>>>,
    key: String,
    run: u64,
}

impl Reporter {
    fn with(&self, f: impl FnOnce(&mut TunnelStatus)) {
        if let Some(e) = self.map.lock().unwrap().get_mut(&self.key).filter(|e| e.run == self.run) {
            f(&mut e.status);
        }
    }
    fn connection(&self, delta: i32) {
        self.with(|s| {
            s.connections = s.connections.saturating_add_signed(delta);
            if delta > 0 {
                s.error = None;
            }
        });
    }
    fn failed_connection(&self, e: &Error) {
        self.with(|s| s.error = serde_json::to_value(e).ok());
    }
    fn lost(&self) {
        self.with(|s| {
            s.state = TunnelState::Error;
            s.connections = 0;
            s.error = serde_json::to_value(Error::Connect { message: "connection lost".into() }).ok();
        });
    }
}

impl Tunnels {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn statuses(&self) -> Vec<TunnelStatus> {
        self.inner.lock().unwrap().values().map(|e| e.status.clone()).collect()
    }

    pub fn stop(&self, key: &str) {
        if let Some(e) = self.inner.lock().unwrap().remove(key) {
            let _ = e.stop.send(true);
        }
    }

    /// Stops every tunnel (when the vault locks).
    pub fn stop_all(&self) {
        for (_, e) in self.inner.lock().unwrap().drain() {
            let _ = e.stop.send(true);
        }
    }

    /// Opens a tunnel through the host `req` connects to. Returns a server key to pin, if new.
    pub async fn start(&self, key: &str, spec: TunnelSpec, req: &ConnectRequest) -> Result<ssh::NewKeys> {
        self.stop(key);
        let target = match spec.kind {
            TunnelKind::Dynamic => None,
            _ => Some(parse_target(spec.target.as_deref().unwrap_or_default())?),
        };
        // A busy local port fails before connecting anywhere.
        let listener = match spec.kind {
            TunnelKind::Remote => None,
            _ => Some(bind(spec.bind_port).await?),
        };
        let (session, known) = ssh::session(req).await?;

        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let (stop, stopped) = watch::channel(false);
        let status = TunnelStatus { key: key.to_string(), state: TunnelState::On, connections: 0, error: None };
        self.inner.lock().unwrap().insert(key.to_string(), Entry { status, run, stop });
        let report = Reporter { map: self.inner.clone(), key: key.to_string(), run };

        match (spec.kind, listener, target) {
            (TunnelKind::Remote, _, Some((host, port))) => {
                let counter = report.clone();
                let fwd = RemoteForward { host, port, on_connection: Arc::new(move |d| counter.connection(d)) };
                if let Err(e) = session.listen_remote(spec.bind_port, fwd).await {
                    self.inner.lock().unwrap().remove(key);
                    return Err(e);
                }
                let bind_port = spec.bind_port;
                tokio::spawn(async move {
                    watch_session(&session, stopped, &report).await;
                    session.stop_remote(bind_port).await;
                });
            }
            (kind, Some(listener), target) => {
                tokio::spawn(serve_local(listener, session, kind, target, stopped, report));
            }
            _ => unreachable!("every kind has what it needs"),
        }
        Ok(known)
    }
}

async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => Error::PortBusy { port },
        _ => Error::Connect { message: format!("port {port}: {e}") },
    })
}

/// Waits until the tunnel is stopped (true) or its connection is lost (false).
async fn watch_session(session: &Session, mut stopped: watch::Receiver<bool>, report: &Reporter) -> bool {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = stopped.changed() => return true,
            _ = tick.tick() => if session.is_closed() {
                report.lost();
                return false;
            },
        }
    }
}

async fn serve_local(listener: TcpListener, session: Session, kind: TunnelKind, target: Option<(String, u16)>, stopped: watch::Receiver<bool>, report: Reporter) {
    let mut watching = Box::pin({
        let (session, report) = (session.clone(), report.clone());
        async move { watch_session(&session, stopped, &report).await }
    });
    loop {
        tokio::select! {
            _ = &mut watching => return,
            accepted = listener.accept() => {
                let Ok((sock, from)) = accepted else { continue };
                let (session, target, report) = (session.clone(), target.clone(), report.clone());
                tokio::spawn(async move {
                    let result = match kind {
                        TunnelKind::Dynamic => socks(sock, from, &session, &report).await,
                        _ => {
                            let (host, port) = target.expect("a target");
                            forward(sock, from, &session, &host, port, &report).await
                        }
                    };
                    if let Err(e) = result {
                        report.failed_connection(&e);
                    }
                });
            }
        }
    }
}

async fn forward(mut sock: TcpStream, from: SocketAddr, session: &Session, host: &str, port: u16, report: &Reporter) -> Result<()> {
    let mut ch = session.connect_tcp(host, port, from).await?;
    report.connection(1);
    let _ = tokio::io::copy_bidirectional(&mut sock, &mut ch).await;
    report.connection(-1);
    Ok(())
}

/// A SOCKS5 `CONNECT` without authentication, the target reached from the server.
async fn socks(mut sock: TcpStream, from: SocketAddr, session: &Session, report: &Reporter) -> Result<()> {
    let proto = || Error::Connect { message: "not a SOCKS5 client".into() };
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(proto());
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        sock.write_all(&[5, 0xff]).await?;
        return Err(proto());
    }
    sock.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            sock.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| proto())?
        }
        4 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return Err(proto()),
    };
    let mut port = [0u8; 2];
    sock.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    // Only CONNECT: no binding or UDP on the server's behalf.
    if req[1] != 1 {
        sock.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(proto());
    }
    match session.connect_tcp(&host, port, from).await {
        Ok(mut ch) => {
            sock.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            report.connection(1);
            let _ = tokio::io::copy_bidirectional(&mut sock, &mut ch).await;
            report.connection(-1);
            Ok(())
        }
        Err(e) => {
            // "Host unreachable" covers what the server could not reach or refused.
            let _ = sock.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            Err(e)
        }
    }
}

/// Opens a tunnel of a vault, logging in to its host as a terminal would (own login, shared secret, prompt).
pub async fn start_in(store: &std::sync::Mutex<crate::store::Store>, tunnels: &Tunnels, vault: &str, tunnel_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<()> {
    let poisoned = || Error::Internal { message: "state lock poisoned".into() };
    let (spec, host_id, req) = {
        let s = store.lock().map_err(|_| poisoned())?;
        let t = s.tunnel_in(vault, tunnel_id)?;
        let req = s.connect_request_in(vault, &t.via_host_id, secret, trust_fingerprint)?;
        (TunnelSpec { kind: t.kind, bind_port: t.bind_port, target: t.target }, t.via_host_id, req)
    };
    let known = tunnels.start(&format!("{vault}/{tunnel_id}"), spec, &req).await?;
    store.lock().map_err(|_| poisoned())?.record_connection_in(vault, &host_id, known)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(parse_target("db:5432").unwrap(), ("db".into(), 5432));
        assert_eq!(parse_target("[::1]:22").unwrap(), ("::1".into(), 22));
        assert!(parse_target("db").is_err() && parse_target("db:0").is_err() && parse_target(":80").is_err());
    }
}
