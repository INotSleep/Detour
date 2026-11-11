//! Reaching a host through a saved proxy: a stream to the host that carries the SSH connection.

pub mod link;
mod plain;
mod reality;
mod tls;
pub mod vless;
pub mod wireguard;
mod xhttp;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::model::ProxyEntry;
use crate::model::ProxyKind;
use crate::{Error, Result};

pub use plain::{http_connect, socks5};

/// A byte stream to the host, whatever carries it.
pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}
pub type Stream = Box<dyn Io>;

/// Why a proxy failed, as `Error::Proxy { reason }`: `unreachable`, `auth`, `refused`,
/// `protocol`, `handshake`, `tls`, `noAccess`, `missing`, or `unsupported:<what>`.
pub(crate) fn failed(reason: &str) -> Error {
    Error::Proxy { reason: reason.to_string() }
}

pub(crate) async fn reach(address: &str, port: u16) -> Result<TcpStream> {
    let tcp = TcpStream::connect((address, port)).await.map_err(|_| failed("unreachable"))?;
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

/// Opens a stream to `target:port` through the proxy.
pub async fn connect(p: &ProxyEntry, target: &str, port: u16) -> Result<Stream> {
    match &p.kind {
        ProxyKind::Socks5 { address, port: pp, user } => Ok(Box::new(socks5(address, *pp, user, p.password.as_deref(), target, port).await?)),
        ProxyKind::Http { address, port: pp, user } => Ok(Box::new(http_connect(address, *pp, user, p.password.as_deref(), target, port).await?)),
        ProxyKind::Wireguard { .. } => wireguard::connect(p, target, port).await,
        ProxyKind::Vless { .. } => vless::connect(p, target, port).await,
    }
}

/// Tries a proxy: how long reaching something through it takes, in milliseconds.
/// A WireGuard proxy is tried by its handshake: what it routes to is up to its server.
pub async fn check(p: &ProxyEntry) -> Result<u64> {
    let started = std::time::Instant::now();
    let limit = std::time::Duration::from_secs(15);
    if matches!(p.kind, ProxyKind::Wireguard { .. }) {
        tokio::time::timeout(limit, wireguard::handshake(p)).await.map_err(|_| failed("handshake"))??;
    } else {
        tokio::time::timeout(limit, connect(p, "1.1.1.1", 443)).await.map_err(|_| failed("unreachable"))??;
    }
    Ok(started.elapsed().as_millis() as u64)
}
