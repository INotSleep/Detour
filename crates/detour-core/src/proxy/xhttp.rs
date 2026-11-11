//! XHTTP: VLESS carried in HTTP requests.

use super::{Stream, failed};
use crate::Result;
use crate::model::ProxyEntry;

pub async fn connect(_p: &ProxyEntry, _id: &[u8; 16], _target: &str, _port: u16) -> Result<Stream> {
    Err(failed("unsupported:XHTTP"))
}
