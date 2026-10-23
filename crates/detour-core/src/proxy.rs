//! Reaching a host through a SOCKS5 or HTTP proxy: a TCP stream to the
//! proxy that, once the handshake is done, carries the SSH connection.

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::{Error, Result};

fn failed(reason: &str) -> Error {
    Error::Proxy { reason: reason.to_string() }
}

async fn reach(address: &str, port: u16) -> Result<TcpStream> {
    let tcp = TcpStream::connect((address, port)).await.map_err(|_| failed("unreachable"))?;
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

/// SOCKS5 (RFC 1928), with user and password (RFC 1929) when `user` is not empty.
/// The target's name goes to the proxy to resolve, as `ssh -o ProxyCommand` with `nc -X 5` does.
pub async fn socks5(address: &str, port: u16, user: &str, password: Option<&str>, target: &str, target_port: u16) -> Result<TcpStream> {
    let mut s = reach(address, port).await?;
    let io = |_| failed("protocol");
    let methods: &[u8] = if user.is_empty() { &[0x00] } else { &[0x00, 0x02] };
    let mut hello = vec![0x05, methods.len() as u8];
    hello.extend_from_slice(methods);
    s.write_all(&hello).await.map_err(io)?;
    let mut chosen = [0u8; 2];
    s.read_exact(&mut chosen).await.map_err(io)?;
    match chosen {
        [0x05, 0x00] => {}
        [0x05, 0x02] => {
            let pass = password.unwrap_or_default();
            if user.len() > 255 || pass.len() > 255 {
                return Err(failed("auth"));
            }
            let mut login = vec![0x01, user.len() as u8];
            login.extend_from_slice(user.as_bytes());
            login.push(pass.len() as u8);
            login.extend_from_slice(pass.as_bytes());
            s.write_all(&login).await.map_err(io)?;
            let mut status = [0u8; 2];
            s.read_exact(&mut status).await.map_err(io)?;
            if status[1] != 0 {
                return Err(failed("auth"));
            }
        }
        [0x05, 0xff] => return Err(failed("auth")),
        _ => return Err(failed("protocol")),
    }
    if target.len() > 255 {
        return Err(failed("protocol"));
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, target.len() as u8];
    req.extend_from_slice(target.as_bytes());
    req.extend_from_slice(&target_port.to_be_bytes());
    s.write_all(&req).await.map_err(io)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(io)?;
    if head[0] != 0x05 {
        return Err(failed("protocol"));
    }
    if head[1] != 0x00 {
        return Err(failed("refused"));
    }
    // The address the proxy bound: skip it.
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await.map_err(io)?;
            len[0] as usize
        }
        _ => return Err(failed("protocol")),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await.map_err(io)?;
    Ok(s)
}

/// An HTTP proxy's `CONNECT`, with Basic authentication when `user` is not empty.
pub async fn http_connect(address: &str, port: u16, user: &str, password: Option<&str>, target: &str, target_port: u16) -> Result<TcpStream> {
    let mut s = reach(address, port).await?;
    let io = |_| failed("protocol");
    let authority = if target.contains(':') { format!("[{target}]:{target_port}") } else { format!("{target}:{target_port}") };
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if !user.is_empty() {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{}", password.unwrap_or_default()));
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.map_err(io)?;
    // Read the response head byte by byte: anything after it is the SSH server talking.
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || s.read(&mut b).await.map_err(io)? == 0 {
            return Err(failed("protocol"));
        }
        head.push(b[0]);
    }
    let status = String::from_utf8_lossy(&head).split_whitespace().nth(1).and_then(|c| c.parse::<u16>().ok()).ok_or_else(|| failed("protocol"))?;
    match status {
        200..=299 => Ok(s),
        407 => Err(failed("auth")),
        _ => Err(failed("refused")),
    }
}
