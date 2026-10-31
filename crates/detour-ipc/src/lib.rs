//! How the Detour core (`detour daemon`) and its clients (the desktop app, the
//! command line, the MCP server for agents) talk over a local socket.
//!
//! Each frame is a big-endian `u32` length, a kind byte and a payload:
//! a JSON [`Message`] (calls, replies, events, JSON items of a stream), or raw
//! bytes of a stream (terminal output) after its `u64` id. A call that streams
//! takes the id of a stream its caller opened with [`Client::stream`].

mod client;
mod name;
mod start;

pub use client::{Client, Event, StreamItem};
pub use interprocess::local_socket::tokio::Stream;
pub use name::{Server, listen, socket_name};
pub use start::{connect_or_start, connect_or_start_with, runnable_copy, start_core, start_process};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Longest frame either side accepts (a file read through MCP, a vault view).
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// What the core's methods are, as a number that grows when clients need new ones
/// (`hello` says it). A core without it is older than AI agents (2).
pub const PROTOCOL: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Message {
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
    /// The answer to a call: `err` is set when it failed (a core error, `{ "kind": ... }`).
    Reply {
        id: u64,
        #[serde(default)]
        ok: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<Value>,
    },
    /// Something changed in the core that clients may want to know (`vault.changed`, ...).
    Event {
        name: String,
        #[serde(default)]
        data: Value,
    },
    /// A JSON item of a stream: progress, a session closing.
    Stream { stream: u64, data: Value },
}

#[derive(Debug, Clone)]
pub enum Frame {
    Msg(Message),
    Data { stream: u64, bytes: Vec<u8> },
}

const KIND_JSON: u8 = 0;
const KIND_DATA: u8 = 1;

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Frame> {
    let len = r.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(invalid("frame size"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    match buf[0] {
        KIND_JSON => Ok(Frame::Msg(serde_json::from_slice(&buf[1..]).map_err(|e| invalid(&e.to_string()))?)),
        KIND_DATA if buf.len() >= 9 => {
            let stream = u64::from_be_bytes(buf[1..9].try_into().unwrap());
            Ok(Frame::Data { stream, bytes: buf[9..].to_vec() })
        }
        _ => Err(invalid("frame kind")),
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> std::io::Result<()> {
    let mut buf = Vec::new();
    match frame {
        Frame::Msg(m) => {
            buf.push(KIND_JSON);
            serde_json::to_writer(&mut buf, m).map_err(|e| invalid(&e.to_string()))?;
        }
        Frame::Data { stream, bytes } => {
            buf.push(KIND_DATA);
            buf.extend(stream.to_be_bytes());
            buf.extend_from_slice(bytes);
        }
    }
    if buf.len() > MAX_FRAME {
        return Err(invalid("frame size"));
    }
    w.write_u32(buf.len() as u32).await?;
    w.write_all(&buf).await?;
    w.flush().await
}

fn invalid(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("detour ipc: bad {what}"))
}

/// The process on the other end of a connection, where the system says.
#[cfg(not(target_os = "macos"))]
pub fn peer_pid(stream: &Stream) -> Option<u32> {
    use interprocess::local_socket::traits::StreamCommon;
    #[allow(clippy::unnecessary_cast)]
    stream.peer_creds().ok()?.pid().map(|p| p as u32)
}

/// macOS keeps the peer's pid apart from its credentials (LOCAL_PEERPID), which interprocess doesn't read.
#[cfg(target_os = "macos")]
pub fn peer_pid(stream: &Stream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let Stream::UdSocket(s) = stream;
    let fd = s.inner().as_raw_fd();
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: a connected socket of this process, and a pid_t-sized buffer with its size.
    let r = unsafe { libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&raw mut pid).cast(), &mut len) };
    (r == 0 && pid > 0).then_some(pid as u32)
}

/// An error made on this side of the socket, shaped like the core's.
pub fn local_error(kind: &str, message: impl Into<String>) -> Value {
    serde_json::json!({ "kind": kind, "message": message.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let call = Frame::Msg(Message::Call { id: 7, method: "vault_get".into(), params: serde_json::json!({ "x": 1 }) });
        write_frame(&mut a, &call).await.unwrap();
        write_frame(&mut a, &Frame::Data { stream: 3, bytes: b"hi".to_vec() }).await.unwrap();
        match read_frame(&mut b).await.unwrap() {
            Frame::Msg(Message::Call { id, method, params }) => assert_eq!((id, method.as_str(), params["x"].as_i64()), (7, "vault_get", Some(1))),
            other => panic!("{other:?}"),
        }
        match read_frame(&mut b).await.unwrap() {
            Frame::Data { stream, bytes } => assert_eq!((stream, bytes.as_slice()), (3, b"hi".as_slice())),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_reply_without_err_is_a_success_even_when_null() {
        let r: Message = serde_json::from_str(r#"{"t":"reply","id":1,"ok":null}"#).unwrap();
        assert!(matches!(r, Message::Reply { err: None, .. }));
    }
}
