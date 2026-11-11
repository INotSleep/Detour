//! VLESS: a request header (version, user id, flow, command, target) before the
//! connection's bytes, and a two-byte answer before the server's.
//!
//! With `xtls-rprx-vision` the first writes each way are framed and padded so their
//! sizes don't give away what is inside the TLS; then the bytes flow as they are.
//! Detour only carries SSH, never TLS inside, so the frames end after a few writes
//! (as Xray's own client does for such traffic) and "direct" copying never starts.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use rand::RngExt;
use sha1::Digest as _;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

use super::{Stream, failed, reach};
use crate::model::{ProxyEntry, ProxyKind, VlessSecurity, VlessTransport};
use crate::Result;

pub const VISION: &str = "xtls-rprx-vision";

/// The 16 bytes of a user id: a UUID, or (like Xray) a UUIDv5 of a shorter string.
pub fn user_id(text: &str) -> Result<[u8; 16]> {
    let t = text.trim();
    if (32..=36).contains(&t.len()) {
        let hex: String = t.chars().filter(|c| *c != '-').collect();
        if hex.len() == 32 {
            let bytes: Option<Vec<u8>> = (0..32).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
            if let Some(b) = bytes {
                return Ok(b.try_into().unwrap());
            }
        }
        return Err(failed("auth"));
    }
    if t.is_empty() || t.len() > 30 {
        return Err(failed("auth"));
    }
    let mut h = sha1::Sha1::new();
    h.update([0u8; 16]);
    h.update(t.as_bytes());
    let mut u: [u8; 16] = h.finalize()[..16].try_into().unwrap();
    u[6] = (u[6] & 0x0f) | (5 << 4);
    u[8] = (u[8] & 0x3f) | 0x80;
    Ok(u)
}

/// The request header for a TCP connection to `target:port`.
pub fn request(id: &[u8; 16], vision: bool, target: &str, port: u16) -> Result<Vec<u8>> {
    let mut h = vec![0u8];
    h.extend_from_slice(id);
    if vision {
        // Protobuf `Addons { string Flow = 1; }`.
        h.push(2 + VISION.len() as u8);
        h.extend_from_slice(&[0x0a, VISION.len() as u8]);
        h.extend_from_slice(VISION.as_bytes());
    } else {
        h.push(0);
    }
    h.push(1);
    h.extend_from_slice(&port.to_be_bytes());
    let bare = target.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            h.push(1);
            h.extend_from_slice(&v4.octets());
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            h.push(3);
            h.extend_from_slice(&v6.octets());
        }
        Err(_) => {
            if bare.is_empty() || bare.len() > 255 {
                return Err(failed("protocol"));
            }
            h.push(2);
            h.push(bare.len() as u8);
            h.extend_from_slice(bare.as_bytes());
        }
    }
    Ok(h)
}

/// A stream to `target:port` through a VLESS proxy over TCP.
pub async fn connect(p: &ProxyEntry, target: &str, port: u16) -> Result<Stream> {
    let ProxyKind::Vless { address, port: pport, security, sni, alpn, pinned_cert, transport, flow, .. } = &p.kind else { return Err(failed("protocol")) };
    let id = user_id(p.uuid.as_deref().ok_or_else(|| failed("auth"))?)?;
    if *transport == VlessTransport::Xhttp {
        return super::xhttp::connect(p, &id, target, port).await;
    }
    let vision = flow == VISION;
    let tcp = reach(address, *pport).await?;
    let sni = if sni.is_empty() { address.as_str() } else { sni.as_str() };
    match security {
        VlessSecurity::None if vision => Err(failed("unsupported:Vision without TLS")),
        VlessSecurity::None => Ok(Box::new(VlessStream::open(tcp, &id, false, target, port).await?)),
        VlessSecurity::Tls => {
            // As Xray: both, when the link names none.
            let alpn = if alpn.is_empty() { vec!["h2".to_string(), "http/1.1".to_string()] } else { alpn.clone() };
            let tls = super::tls::connect(tcp, sni, &alpn, pinned_cert).await?;
            Ok(Box::new(VlessStream::open(tls, &id, vision, target, port).await?))
        }
        VlessSecurity::Reality => {
            let tls = super::reality::connect(tcp, p, sni).await?;
            Ok(Box::new(VlessStream::open(tls, &id, vision, target, port).await?))
        }
    }
}

/* ---------- Vision frames ---------- */

const CONTINUE: u8 = 0;
const END: u8 = 1;
const DIRECT: u8 = 2;
/// Xray's buffer, which a frame must fit in with its 21 bytes of UUID and header.
const FRAME: usize = 8192 - 21;
/// Writes framed before the bytes go as they are, the first (empty) one included.
const FRAMED_WRITES: u32 = 7;

/// One frame: the user id before the first, then command, lengths, data and padding.
fn frame(id: Option<&[u8; 16]>, command: u8, data: &[u8], long: bool) -> Vec<u8> {
    let mut rng = rand::rng();
    let content = data.len();
    let mut padding = if long && content < 900 { rng.random_range(0..500) + 900 - content } else { rng.random_range(0..256) };
    padding = padding.min(8192 - 21 - content);
    let mut f = Vec::with_capacity(21 + content + padding);
    if let Some(id) = id {
        f.extend_from_slice(id);
    }
    f.push(command);
    f.extend_from_slice(&(content as u16).to_be_bytes());
    f.extend_from_slice(&(padding as u16).to_be_bytes());
    f.extend_from_slice(data);
    let start = f.len();
    f.resize(start + padding, 0);
    rng.fill(&mut f[start..]);
    f
}

/// Takes the server's frames apart, across reads.
#[derive(Debug)]
enum Unpad {
    /// The user id before the first frame.
    Id { left: usize },
    Head { buf: [u8; 5], have: usize },
    Content { left: usize, command: u8, padding: usize },
    Padding { left: usize, command: u8 },
    Raw,
}

impl Unpad {
    fn feed(&mut self, mut input: &[u8], out: &mut VecDeque<u8>) -> io::Result<()> {
        while !input.is_empty() {
            match self {
                Unpad::Raw => {
                    out.extend(input);
                    return Ok(());
                }
                Unpad::Id { left } => {
                    let n = (*left).min(input.len());
                    *left -= n;
                    input = &input[n..];
                    if *left == 0 {
                        *self = Unpad::Head { buf: [0; 5], have: 0 };
                    }
                }
                Unpad::Head { buf, have } => {
                    let n = (5 - *have).min(input.len());
                    buf[*have..*have + n].copy_from_slice(&input[..n]);
                    *have += n;
                    input = &input[n..];
                    if *have == 5 {
                        let command = buf[0];
                        if command >= DIRECT {
                            return Err(io::Error::other("vision: unexpected command"));
                        }
                        let left = u16::from_be_bytes([buf[1], buf[2]]) as usize;
                        let padding = u16::from_be_bytes([buf[3], buf[4]]) as usize;
                        *self = Unpad::Content { left, command, padding };
                    }
                }
                Unpad::Content { left, command, padding } => {
                    let n = (*left).min(input.len());
                    out.extend(&input[..n]);
                    *left -= n;
                    input = &input[n..];
                    if *left == 0 {
                        *self = Unpad::Padding { left: *padding, command: *command };
                    }
                }
                Unpad::Padding { left, command } => {
                    let n = (*left).min(input.len());
                    *left -= n;
                    input = &input[n..];
                    if *left == 0 {
                        *self = if *command == END { Unpad::Raw } else { Unpad::Head { buf: [0; 5], have: 0 } };
                    }
                }
            }
            // A frame with nothing in it ends here too.
            if let Unpad::Content { left: 0, command, padding } = *self {
                *self = Unpad::Padding { left: padding, command };
            }
            if let Unpad::Padding { left: 0, command } = *self {
                *self = if command == END { Unpad::Raw } else { Unpad::Head { buf: [0; 5], have: 0 } };
            }
        }
        Ok(())
    }
}

/* ---------- the stream ---------- */

/// The server's answer before its bytes: version, then addons to skip.
enum Answer {
    Version,
    AddonsLen,
    Addons(usize),
    Done,
}

pub struct VlessStream<S> {
    inner: S,
    answer: Answer,
    unpad: Option<Unpad>,
    /// Bytes read and taken apart, not yet given out.
    ready: VecDeque<u8>,
    /// Encoded, not yet written.
    pending: Vec<u8>,
    pending_at: usize,
    vision: bool,
    framed: u32,
}

impl<S: AsyncRead + AsyncWrite + Unpin> VlessStream<S> {
    /// Sends the request right away (SSH may wait for the server to speak first).
    pub async fn open(mut inner: S, id: &[u8; 16], vision: bool, target: &str, port: u16) -> Result<Self> {
        let mut first = request(id, vision, target, port)?;
        if vision {
            // As Xray does when nothing comes to send: an empty frame, long padded.
            first.extend(frame(Some(id), CONTINUE, &[], true));
        }
        inner.write_all(&first).await.map_err(|_| failed("unreachable"))?;
        inner.flush().await.map_err(|_| failed("unreachable"))?;
        Ok(Self {
            inner,
            answer: Answer::Version,
            unpad: vision.then_some(Unpad::Id { left: 16 }),
            ready: VecDeque::new(),
            pending: Vec::new(),
            pending_at: 0,
            vision,
            framed: 1,
        })
    }

    fn poll_pending(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.pending_at < self.pending.len() {
            let n = ready!(Pin::new(&mut self.inner).poll_write(cx, &self.pending[self.pending_at..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.pending_at += n;
        }
        self.pending.clear();
        self.pending_at = 0;
        Poll::Ready(Ok(()))
    }

    /// Server bytes after the answer.
    fn take_in(&mut self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            match self.answer {
                Answer::Version => {
                    if data[0] != 0 {
                        return Err(io::Error::other("vless: unexpected version"));
                    }
                    self.answer = Answer::AddonsLen;
                    data = &data[1..];
                }
                Answer::AddonsLen => {
                    self.answer = if data[0] == 0 { Answer::Done } else { Answer::Addons(data[0] as usize) };
                    data = &data[1..];
                }
                Answer::Addons(left) => {
                    let n = left.min(data.len());
                    self.answer = if left == n { Answer::Done } else { Answer::Addons(left - n) };
                    data = &data[n..];
                }
                Answer::Done => {
                    match &mut self.unpad {
                        Some(u) => u.feed(data, &mut self.ready)?,
                        None => self.ready.extend(data),
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for VlessStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if !this.ready.is_empty() {
                let n = buf.remaining().min(this.ready.len());
                let (a, b) = this.ready.as_slices();
                let from_a = n.min(a.len());
                buf.put_slice(&a[..from_a]);
                buf.put_slice(&b[..n - from_a]);
                this.ready.drain(..n);
                return Poll::Ready(Ok(()));
            }
            let mut tmp = [0u8; 16 * 1024];
            let mut rb = ReadBuf::new(&mut tmp);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut rb))?;
            let got = rb.filled();
            if got.is_empty() {
                return Poll::Ready(Ok(()));
            }
            let got = got.to_vec();
            this.take_in(&got)?;
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for VlessStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_pending(cx))?;
        if !this.vision || this.framed >= FRAMED_WRITES {
            return Pin::new(&mut this.inner).poll_write(cx, data);
        }
        let n = data.len().min(FRAME);
        this.framed += 1;
        let command = if this.framed >= FRAMED_WRITES { END } else { CONTINUE };
        this.pending = frame(None, command, &data[..n], false);
        this.pending_at = 0;
        // Written later if the inner stream is busy: the bytes are taken.
        let _ = this.poll_pending(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_pending(cx))?;
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_pending(cx))?;
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_like_xray() {
        assert_eq!(user_id("b831381d-6324-4d53-ad4f-8cda48b30811").unwrap()[..4], [0xb8, 0x31, 0x38, 0x1d]);
        assert_eq!(user_id("b831381d63244d53ad4f8cda48b30811").unwrap(), user_id("b831381d-6324-4d53-ad4f-8cda48b30811").unwrap());
        // Xray's own example: "example" becomes this UUIDv5.
        let v5 = user_id("example").unwrap();
        assert_eq!(v5[6] >> 4, 5);
        assert!(user_id("").is_err());
    }

    #[test]
    fn the_header_names_the_target() {
        let id = [7u8; 16];
        let h = request(&id, true, "10.0.0.5", 22).unwrap();
        assert_eq!(h[0], 0);
        assert_eq!(&h[1..17], &id);
        assert_eq!(h[17] as usize, 2 + VISION.len());
        let rest = &h[18 + h[17] as usize..];
        assert_eq!(rest, &[1, 0, 22, 1, 10, 0, 0, 5]);
        let named = request(&id, false, "db.internal", 2222).unwrap();
        assert_eq!(&named[17..], &[0, 1, 0x08, 0xae, 2, 11, b'd', b'b', b'.', b'i', b'n', b't', b'e', b'r', b'n', b'a', b'l']);
    }

    #[test]
    fn frames_come_apart_however_they_are_cut() {
        let id = [9u8; 16];
        let mut wire = frame(Some(&id), CONTINUE, b"", true);
        wire.extend(frame(None, CONTINUE, b"SSH-2.0-Server\r\n", false));
        wire.extend(frame(None, END, b"more", false));
        wire.extend(b"raw bytes");
        for cut in [1, 2, 3, 5, 7, 64, wire.len()] {
            let mut u = Unpad::Id { left: 16 };
            let mut out = VecDeque::new();
            for chunk in wire.chunks(cut) {
                u.feed(chunk, &mut out).unwrap();
            }
            assert_eq!(out.iter().copied().collect::<Vec<_>>(), b"SSH-2.0-Server\r\nmoreraw bytes", "cut {cut}");
        }
    }
}
