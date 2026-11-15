//! VLESS Encryption (Xray's `mlkem768x25519plus`): a layer under VLESS, over whatever
//! carries it, that the server's own keys open. As Xray's client does it
//! (`proxy/vless/encryption`):
//!
//! - The client's first message reaches the server's keys in turn (X25519, or ML-KEM-768
//!   for a 1184-byte key), each chained to the next, and carries a fresh ML-KEM-768 and
//!   X25519 key for this connection; the server answers with its halves. Both sides then
//!   hold a key of both exchanges together with the first one's.
//! - Then bytes go in records shaped as TLS application data, sealed with AES-256-GCM
//!   under keys derived with BLAKE3.
//! - `xorpub` hides the client's keys from anyone without the config; `random` also hides
//!   the record headers, so nothing on the wire looks like anything.
//!
//! Detour always does the full exchange: a server set to `0rtt` takes that too, and an SSH
//! connection doesn't miss the round trip.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use aes::cipher::{KeyIvInit, StreamCipher};
use base64::Engine;
use curve25519_dalek::MontgomeryPoint;
use ml_kem::kem::{Decapsulate, Encapsulate, TryKeyInit};
use ml_kem::{Kem, KeyExport, MlKem768};
use rand::RngExt;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use super::blake3;
use super::failed;
use crate::Result;

const PREFIX: &str = "mlkem768x25519plus";
const MAX_NONCE: [u8; 12] = [0xff; 12];
/// The most a record carries; the peer's are at most a TLS record (16384 + 256).
const CHUNK: usize = 8192;
const MAX_RECORD: usize = 16640;

/// How a link's or config's `encryption` asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encryption {
    /// The server's keys, in order: 32 bytes for X25519, 1184 for ML-KEM-768.
    keys: Vec<Vec<u8>>,
    xor: Xor,
    /// Padding of the first message: (chance %, min, max) of each part's length, and of the waits between them in ms.
    lens: Vec<[u32; 3]>,
    gaps: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Xor {
    Native,
    Public,
    Random,
}

/// What `encryption` asks for: nothing for `none` (or empty), or an error if this isn't
/// a value Xray takes.
pub fn parse(text: &str) -> Result<Option<Encryption>> {
    let text = text.trim();
    if text.is_empty() || text == "none" {
        return Ok(None);
    }
    let bad = || failed("unsupported:VLESS encryption");
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() < 4 || parts[0] != PREFIX {
        return Err(bad());
    }
    let xor = match parts[1] {
        "native" => Xor::Native,
        "xorpub" => Xor::Public,
        "random" => Xor::Random,
        _ => return Err(bad()),
    };
    if !matches!(parts[2], "1rtt" | "0rtt") {
        return Err(bad());
    }
    let (mut keys, mut padding) = (Vec::new(), Vec::new());
    for p in &parts[3..] {
        // As Xray: what is short is padding, the rest are keys.
        if p.len() < 20 {
            padding.push(*p);
            continue;
        }
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p).map_err(|_| bad())?;
        if key.len() != 32 && key.len() != 1184 {
            return Err(bad());
        }
        keys.push(key);
    }
    if keys.is_empty() {
        return Err(bad());
    }
    let (lens, gaps) = parse_padding(&padding).ok_or_else(bad)?;
    Ok(Some(Encryption { keys, xor, lens, gaps }))
}

/// `chance-min-max` parts, lengths and waits by turn; the first length at least 35 bytes.
fn parse_padding(parts: &[&str]) -> Option<(Vec<[u32; 3]>, Vec<[u32; 3]>)> {
    let (mut lens, mut gaps) = (Vec::new(), Vec::new());
    let mut most = 0u32;
    for (i, p) in parts.iter().enumerate() {
        let y: Vec<u32> = p.split('-').map(|n| n.parse().ok()).collect::<Option<_>>()?;
        let y: [u32; 3] = y.get(..3)?.try_into().ok()?;
        if i == 0 && (y[0] < 100 || y[1] < 18 + 17 || y[2] < 18 + 17) {
            return None;
        }
        if i % 2 == 0 {
            most += y[1].max(y[2]);
            lens.push(y);
        } else {
            gaps.push(y);
        }
    }
    (most <= 18 + 65535).then_some((lens, gaps))
}

/// A number in `[from, to)` (`from` if they are equal), as Xray's `RandBetween`.
fn between(a: u32, b: u32) -> u32 {
    let (from, to) = (a.min(b), a.max(b));
    if from == to { from } else { rand::rng().random_range(from..to) }
}

/// The padding's part lengths and the waits after each part, drawn by their chances.
fn padding(e: &Encryption) -> (Vec<usize>, Vec<Duration>) {
    let (lens, gaps) = if e.lens.is_empty() { (vec![[100, 111, 1111], [50, 0, 3333]], vec![[75, 0, 111]]) } else { (e.lens.clone(), e.gaps.clone()) };
    let draw = |y: &[u32; 3]| if y[0] >= between(0, 100) { between(y[1], y[2]) } else { 0 };
    (lens.iter().map(|y| draw(y) as usize).collect(), gaps.iter().map(|y| Duration::from_millis(draw(y) as u64)).collect())
}

type Ctr = ctr::Ctr128BE<aes::Aes256>;

/// AES-256-CTR under a key derived from `key`, the counter starting at `iv`.
fn ctr(key: &[u8], iv: &[u8; 16]) -> Ctr {
    Ctr::new(&blake3::derive_key(b"VLESS", key).into(), &(*iv).into())
}

/// AES-256-GCM under a key derived from `key` for `ctx`, its nonce counting up from one.
struct Aead {
    key: LessSafeKey,
    nonce: [u8; 12],
}

impl Aead {
    fn new(ctx: &[u8], key: &[u8]) -> Self {
        let k = blake3::derive_key(ctx, key);
        Self { key: LessSafeKey::new(UnboundKey::new(&aead::AES_256_GCM, &k).unwrap()), nonce: [0; 12] }
    }

    fn next(&mut self) -> [u8; 12] {
        for b in self.nonce.iter_mut().rev() {
            *b = b.wrapping_add(1);
            if *b != 0 {
                break;
            }
        }
        self.nonce
    }

    fn seal(&mut self, nonce: Option<[u8; 12]>, plain: &[u8], aad: &[u8]) -> Vec<u8> {
        let n = nonce.unwrap_or_else(|| self.next());
        let mut out = plain.to_vec();
        self.key.seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::from(aad), &mut out).expect("sealed");
        out
    }

    fn open(&mut self, nonce: Option<[u8; 12]>, sealed: &[u8], aad: &[u8]) -> io::Result<Vec<u8>> {
        let n = nonce.unwrap_or_else(|| self.next());
        let mut buf = sealed.to_vec();
        let len = self.key.open_in_place(Nonce::assume_unique_for_key(n), Aad::from(aad), &mut buf).map_err(|_| io::Error::other("vless encryption: bad record"))?.len();
        buf.truncate(len);
        Ok(buf)
    }
}

fn length(n: usize) -> [u8; 2] {
    (n as u16).to_be_bytes()
}

/// A record header (TLS application data); `len` counts the tag.
fn header(len: usize) -> [u8; 5] {
    [23, 3, 3, (len >> 8) as u8, len as u8]
}

fn record_len(h: &[u8; 5]) -> io::Result<usize> {
    let l = (h[3] as usize) << 8 | h[4] as usize;
    if h[..3] != [23, 3, 3] || !(17..=MAX_RECORD).contains(&l) {
        return Err(io::Error::other("vless encryption: bad record header"));
    }
    Ok(l)
}

/// Opens the layer over `conn`: the exchange with the server, after which bytes go sealed.
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(mut conn: S, e: &Encryption) -> Result<EncryptedStream<S>> {
    let hs = |_| failed("handshake");
    let relays: usize = e.keys.iter().map(|k| if k.len() == 32 { 64 } else { 1088 + 32 }).sum::<usize>() - 32;
    let start = 16 + relays;
    const PFS: usize = 18 + 1184 + 32 + 16;
    let (mut lens, gaps) = padding(e);
    let pad: usize = lens.iter().sum();
    let mut hello = vec![0u8; start + PFS + pad];
    let iv: [u8; 16] = rand::random();
    hello[..16].copy_from_slice(&iv);

    // The server's keys, each one's share chained to the next: only the right server gets to the last.
    let mut nfs = [0u8; 32];
    let mut last: Option<Ctr> = None;
    let mut at = 16;
    for (j, key) in e.keys.iter().enumerate() {
        let index = if key.len() == 32 {
            let secret: [u8; 32] = rand::random();
            hello[at..at + 32].copy_from_slice(&MontgomeryPoint::mul_base_clamped(secret).0);
            nfs = MontgomeryPoint(key[..].try_into().unwrap()).mul_clamped(secret).0;
            if nfs == [0; 32] {
                return Err(failed("handshake"));
            }
            32
        } else {
            let ek = ml_kem::EncapsulationKey::<MlKem768>::new_from_slice(key).map_err(|_| failed("unsupported:VLESS encryption"))?;
            // The thread's generator isn't held across an await: the future must be Send.
            let (ct, shared) = ek.encapsulate_with_rng(&mut rand::rng());
            hello[at..at + 1088].copy_from_slice(&ct);
            nfs.copy_from_slice(&shared);
            1088
        };
        if e.xor != Xor::Native {
            ctr(key, &iv).apply_keystream(&mut hello[at..at + index]);
        }
        if let Some(c) = last.as_mut() {
            c.apply_keystream(&mut hello[at..at + 32]);
        }
        if j == e.keys.len() - 1 {
            break;
        }
        let mut c = ctr(&nfs, &iv);
        let mut next = blake3::hash(&e.keys[j + 1]);
        c.apply_keystream(&mut next);
        hello[at + index..at + index + 32].copy_from_slice(&next);
        last = Some(c);
        at += index + 32;
    }
    let mut first = Aead::new(&iv, &nfs);

    // This connection's own keys, and the padding.
    let (kem_secret, kem_public) = MlKem768::generate_keypair_from_rng(&mut rand::rng());
    let x_secret: [u8; 32] = rand::random();
    let mut pfs_public = kem_public.to_bytes().to_vec();
    pfs_public.extend_from_slice(&MontgomeryPoint::mul_base_clamped(x_secret).0);
    hello[start..start + 18].copy_from_slice(&first.seal(None, &length(PFS - 18), &[]));
    hello[start + 18..start + PFS].copy_from_slice(&first.seal(None, &pfs_public, &[]));
    let p = start + PFS;
    hello[p..p + 18].copy_from_slice(&first.seal(None, &length(pad - 18), &[]));
    let filler = vec![0u8; pad - 34];
    hello[p + 18..p + pad].copy_from_slice(&first.seal(None, &filler, &[]));

    // Sent in parts with waits between, so its sizes and timing vary.
    lens[0] += start + PFS;
    let mut sent = 0;
    for (i, l) in lens.iter().enumerate() {
        if *l > 0 {
            conn.write_all(&hello[sent..sent + l]).await.map_err(|_| failed("unreachable"))?;
            conn.flush().await.map_err(|_| failed("unreachable"))?;
            sent += l;
        }
        if let Some(gap) = gaps.get(i) {
            tokio::time::sleep(*gap).await;
        }
    }

    // The server's halves of this connection's keys.
    let mut answer = vec![0u8; 1088 + 32 + 16];
    conn.read_exact(&mut answer).await.map_err(hs)?;
    let theirs = first.open(Some(MAX_NONCE), &answer, &[]).map_err(hs)?;
    let ct = ml_kem::Ciphertext::<MlKem768>::try_from(&theirs[..1088]).map_err(|_| failed("handshake"))?;
    let kem = kem_secret.decapsulate(&ct);
    let x = MontgomeryPoint(theirs[1088..1120].try_into().unwrap()).mul_clamped(x_secret).0;
    if x == [0; 32] {
        return Err(failed("handshake"));
    }
    let mut united = kem.to_vec();
    united.extend_from_slice(&x);
    united.extend_from_slice(&nfs);
    let aead = Aead::new(&pfs_public, &united);
    let mut peer = Aead::new(&theirs, &united);

    // A ticket (for 0-RTT, which Detour doesn't use), then the server's padding.
    let mut ticket = [0u8; 32];
    conn.read_exact(&mut ticket).await.map_err(hs)?;
    let ticket = peer.open(None, &ticket, &[]).map_err(hs)?;
    let mut len = [0u8; 18];
    conn.read_exact(&mut len).await.map_err(hs)?;
    let len = peer.open(None, &len, &[]).map_err(hs)?;
    let mut padding = vec![0u8; u16::from_be_bytes([len[0], len[1]]) as usize];
    conn.read_exact(&mut padding).await.map_err(hs)?;
    peer.open(None, &padding, &[]).map_err(hs)?;

    let (out_xor, in_xor) = match e.xor {
        Xor::Random => (Some(ctr(&united, &iv)), Some(ctr(&united, ticket[..16].try_into().unwrap()))),
        _ => (None, None),
    };
    Ok(EncryptedStream { inner: conn, aead, peer, united, out_xor, in_xor, raw: Vec::new(), head: None, plain: Vec::new(), plain_at: 0, out: Vec::new(), out_at: 0 })
}

/// Bytes in records after the exchange.
pub struct EncryptedStream<S> {
    inner: S,
    aead: Aead,
    peer: Aead,
    united: Vec<u8>,
    /// `random`: the record headers each way go through these.
    out_xor: Option<Ctr>,
    in_xor: Option<Ctr>,
    /// Read, not yet a whole record.
    raw: Vec<u8>,
    /// The header of the record being read.
    head: Option<([u8; 5], usize)>,
    plain: Vec<u8>,
    plain_at: usize,
    /// Sealed, not yet written.
    out: Vec<u8>,
    out_at: usize,
}

impl<S> EncryptedStream<S> {
    /// The next whole record in what was read, opened.
    fn next_record(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.head.is_none() {
            if self.raw.len() < 5 {
                return Ok(None);
            }
            let mut h: [u8; 5] = self.raw[..5].try_into().unwrap();
            if let Some(x) = self.in_xor.as_mut() {
                x.apply_keystream(&mut h);
            }
            self.head = Some((h, record_len(&h)?));
            self.raw.drain(..5);
        }
        let (h, len) = self.head.unwrap();
        if self.raw.len() < len {
            return Ok(None);
        }
        let sealed: Vec<u8> = self.raw.drain(..len).collect();
        self.head = None;
        // At the end of its nonces a key gives way to one derived from this record.
        let next = (self.peer.nonce == MAX_NONCE).then(|| Aead::new(&[&h[..], &sealed].concat(), &self.united));
        let plain = self.peer.open(None, &sealed, &h);
        if let Some(n) = next {
            self.peer = n;
        }
        plain.map(Some)
    }

    fn seal(&mut self, data: &[u8]) {
        let h = header(data.len() + 16);
        let rekey = self.aead.nonce == MAX_NONCE;
        let mut record = h.to_vec();
        record.extend(self.aead.seal(None, data, &h));
        if rekey {
            self.aead = Aead::new(&record, &self.united);
        }
        if let Some(x) = self.out_xor.as_mut() {
            x.apply_keystream(&mut record[..5]);
        }
        self.out = record;
        self.out_at = 0;
    }
}

impl<S: AsyncWrite + Unpin> EncryptedStream<S> {
    fn poll_out(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.out_at < self.out.len() {
            let n = ready!(Pin::new(&mut self.inner).poll_write(cx, &self.out[self.out_at..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.out_at += n;
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for EncryptedStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if this.plain_at < this.plain.len() {
                let n = (this.plain.len() - this.plain_at).min(buf.remaining());
                buf.put_slice(&this.plain[this.plain_at..this.plain_at + n]);
                this.plain_at += n;
                return Poll::Ready(Ok(()));
            }
            if let Some(p) = this.next_record()? {
                (this.plain, this.plain_at) = (p, 0);
                continue;
            }
            let mut chunk = [0u8; 16 * 1024];
            let mut rb = ReadBuf::new(&mut chunk);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut rb))?;
            if rb.filled().is_empty() {
                // The end: clean only between records.
                return Poll::Ready(if this.raw.is_empty() && this.head.is_none() { Ok(()) } else { Err(io::ErrorKind::UnexpectedEof.into()) });
            }
            this.raw.extend_from_slice(rb.filled());
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_out(cx))?;
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = data.len().min(CHUNK);
        this.seal(&data[..n]);
        // Sent as far as it goes now; the rest on the next write or flush.
        let _ = this.poll_out(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_out(cx))?;
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_out(cx))?;
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const X: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn settings_are_read_as_xray_reads_them() {
        assert_eq!(parse("none").unwrap(), None);
        assert_eq!(parse("").unwrap(), None);
        let e = parse(&format!("mlkem768x25519plus.xorpub.0rtt.{X}")).unwrap().unwrap();
        assert_eq!((e.keys.len(), e.keys[0].len(), e.xor, e.lens.len()), (1, 32, Xor::Public, 0));
        let mlkem = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 1184]);
        let e = parse(&format!("mlkem768x25519plus.random.1rtt.100-111-1111.75-0-111.50-0-3333.{X}.{mlkem}")).unwrap().unwrap();
        assert_eq!((e.keys.len(), e.keys[1].len(), e.xor), (2, 1184, Xor::Random));
        assert_eq!((e.lens, e.gaps), (vec![[100, 111, 1111], [50, 0, 3333]], vec![[75, 0, 111]]));
        for bad in ["aes-128-gcm", "mlkem768x25519plus.native.0rtt", "mlkem768x25519plus.weird.0rtt.x", &format!("mlkem768x25519plus.native.2rtt.{X}"), "mlkem768x25519plus.native.0rtt.AAAAAAAAAAAAAAAAAAAAAAAAAA", &format!("mlkem768x25519plus.native.0rtt.50-111-1111.{X}")] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_exchange_can_run_on_any_thread() {
        fn send<T: Send>(_: &T) {}
        let (a, _b) = tokio::io::duplex(64);
        let e = parse(&format!("mlkem768x25519plus.native.0rtt.{X}")).unwrap().unwrap();
        send(&handshake(a, &e));
    }

    #[test]
    fn records_round_trip_and_rekey_at_the_end_of_nonces() {
        let united = vec![9u8; 96];
        let iv = [3u8; 16];
        let mut a = EncryptedStream { inner: (), aead: Aead::new(b"up", &united), peer: Aead::new(b"down", &united), united: united.clone(), out_xor: Some(ctr(&united, &iv)), in_xor: None, raw: vec![], head: None, plain: vec![], plain_at: 0, out: vec![], out_at: 0 };
        let mut b = EncryptedStream { inner: (), aead: Aead::new(b"down", &united), peer: Aead::new(b"up", &united), united: united.clone(), out_xor: None, in_xor: Some(ctr(&united, &iv)), raw: vec![], head: None, plain: vec![], plain_at: 0, out: vec![], out_at: 0 };
        a.aead.nonce = [0xff; 12];
        a.aead.nonce[11] = 0xfe;
        b.peer.nonce = a.aead.nonce;
        for msg in [&b"SSH-2.0-Detour\r\n"[..], b"second", b"third, under the next key"] {
            a.seal(msg);
            assert_ne!(&a.out[..3], &[23, 3, 3], "random hides the header");
            // Fed in pieces.
            for piece in a.out.chunks(7) {
                b.raw.extend_from_slice(piece);
            }
            assert_eq!(b.next_record().unwrap().unwrap(), msg);
        }
    }
}
