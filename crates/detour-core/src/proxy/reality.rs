//! REALITY: TLS 1.3 whose session id proves the client to the server.

use tokio::net::TcpStream;

use super::{Stream, failed};
use crate::Result;
use crate::model::ProxyEntry;

pub async fn connect(_tcp: TcpStream, _p: &ProxyEntry, _sni: &str) -> Result<Stream> {
    Err(failed("unsupported:REALITY"))
}
