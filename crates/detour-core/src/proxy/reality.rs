//! REALITY: a TLS 1.3 connection whose ClientHello proves this client to the server.
//!
//! The session id carries the client's version, the time and the short id, sealed with
//! a key only the server can derive (X25519 of this hello's key share and the server's
//! REALITY key). A server that recognises it finishes the handshake itself, with a
//! throwaway certificate signed by that key; anyone else gets the real site it hides
//! behind, whose certificate fails that check here.
//!
//! No stock TLS library lets the session id depend on the serialized hello, so this is
//! a small TLS 1.3 client of its own: a ClientHello shaped like Chrome's, X25519MLKEM768
//! (which REALITY requires) and X25519 key shares, the three TLS 1.3 cipher suites, and
//! the record layer with key updates.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use base64::Engine;
use curve25519_dalek::MontgomeryPoint;
use ml_kem::kem::Decapsulate;
use ml_kem::{Kem, KeyExport, MlKem768};
use rand::RngExt;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::{digest, hmac};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

use super::failed;
use crate::Result;
use crate::model::{ProxyEntry, ProxyKind};

/// The Xray version the session id claims (servers may require a minimum).
const VERSION: [u8; 3] = [26, 11, 9];
const X25519MLKEM768: u16 = 0x11ec;
const X25519: u16 = 0x001d;
const MAX_RECORD: usize = 1 << 14;

/* ---------- the parts of TLS 1.3 needed ---------- */

#[derive(Clone, Copy)]
enum Hash {
    Sha256,
    Sha384,
}

impl Hash {
    fn len(self) -> usize {
        match self {
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
        }
    }
    fn digest(self, data: &[u8]) -> Vec<u8> {
        let alg = match self {
            Hash::Sha256 => &digest::SHA256,
            Hash::Sha384 => &digest::SHA384,
        };
        digest::digest(alg, data).as_ref().to_vec()
    }
    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        let alg = match self {
            Hash::Sha256 => hmac::HMAC_SHA256,
            Hash::Sha384 => hmac::HMAC_SHA384,
        };
        hmac::sign(&hmac::Key::new(alg, key), data).as_ref().to_vec()
    }
    fn extract(self, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
        let zeros = vec![0u8; self.len()];
        self.hmac(if salt.is_empty() { &zeros } else { salt }, ikm)
    }
    fn expand(self, prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut t = Vec::new();
        let mut i = 1u8;
        while out.len() < len {
            let mut block = t.clone();
            block.extend_from_slice(info);
            block.push(i);
            t = self.hmac(prk, &block);
            out.extend_from_slice(&t);
            i += 1;
        }
        out.truncate(len);
        out
    }
    fn expand_label(self, secret: &[u8], label: &str, context: &[u8], len: usize) -> Vec<u8> {
        let full = format!("tls13 {label}");
        let mut info = Vec::new();
        info.extend_from_slice(&(len as u16).to_be_bytes());
        info.push(full.len() as u8);
        info.extend_from_slice(full.as_bytes());
        info.push(context.len() as u8);
        info.extend_from_slice(context);
        self.expand(secret, &info, len)
    }
    fn derive(self, secret: &[u8], label: &str, transcript: &[u8]) -> Vec<u8> {
        self.expand_label(secret, label, &self.digest(transcript), self.len())
    }
}

#[derive(Clone, Copy)]
struct Suite {
    hash: Hash,
    aead: &'static aead::Algorithm,
}

fn suite(id: u16) -> Option<Suite> {
    match id {
        0x1301 => Some(Suite { hash: Hash::Sha256, aead: &aead::AES_128_GCM }),
        0x1302 => Some(Suite { hash: Hash::Sha384, aead: &aead::AES_256_GCM }),
        0x1303 => Some(Suite { hash: Hash::Sha256, aead: &aead::CHACHA20_POLY1305 }),
        _ => None,
    }
}

/// One direction's traffic keys.
struct Keys {
    suite: Suite,
    secret: Vec<u8>,
    key: LessSafeKey,
    iv: [u8; 12],
    seq: u64,
}

impl Keys {
    fn new(suite: Suite, secret: Vec<u8>) -> Self {
        let h = suite.hash;
        let key = h.expand_label(&secret, "key", &[], suite.aead.key_len());
        let iv: [u8; 12] = h.expand_label(&secret, "iv", &[], 12).try_into().unwrap();
        Keys { suite, key: LessSafeKey::new(UnboundKey::new(suite.aead, &key).unwrap()), iv, seq: 0, secret }
    }

    /// The next generation (KeyUpdate).
    fn update(&mut self) {
        let next = self.suite.hash.expand_label(&self.secret, "traffic upd", &[], self.suite.hash.len());
        *self = Keys::new(self.suite, next);
    }

    fn nonce(&mut self) -> Nonce {
        let mut n = self.iv;
        for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        self.seq += 1;
        Nonce::assume_unique_for_key(n)
    }

    /// A record of content `kind`.
    fn seal(&mut self, kind: u8, data: &[u8]) -> Vec<u8> {
        let len = data.len() + 1 + self.suite.aead.tag_len();
        let header = [23, 3, 3, (len >> 8) as u8, len as u8];
        let mut body = Vec::with_capacity(len);
        body.extend_from_slice(data);
        body.push(kind);
        let nonce = self.nonce();
        self.key.seal_in_place_append_tag(nonce, Aad::from(header), &mut body).expect("seal");
        let mut record = header.to_vec();
        record.extend(body);
        record
    }

    /// The content type and data of a record's body.
    fn open(&mut self, header: [u8; 5], mut body: Vec<u8>) -> io::Result<(u8, Vec<u8>)> {
        let nonce = self.nonce();
        let plain = self.key.open_in_place(nonce, Aad::from(header), &mut body).map_err(|_| io::Error::other("reality: bad record"))?;
        let end = plain.iter().rposition(|b| *b != 0).ok_or_else(|| io::Error::other("reality: empty record"))?;
        let kind = plain[end];
        Ok((kind, plain[..end].to_vec()))
    }
}

/* ---------- the ClientHello ---------- */

fn grease() -> u16 {
    let n: u16 = rand::rng().random_range(0..16);
    (n << 12) | 0x0a00 | (n << 4) | 0x0a
}

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn with_len16(data: &[u8]) -> Vec<u8> {
    let mut v = u16b(data.len() as u16).to_vec();
    v.extend_from_slice(data);
    v
}

fn ext(kind: u16, data: &[u8]) -> Vec<u8> {
    let mut v = u16b(kind).to_vec();
    v.extend(with_len16(data));
    v
}

/// A ClientHello message like Chrome's, with a zeroed session id at bytes 39..71.
fn client_hello(sni: &str, random: &[u8; 32], mlkem: &[u8], x25519: &[u8; 32]) -> Vec<u8> {
    let (g1, g2, g3, g4) = (grease(), grease(), grease(), grease());
    let mut rng = rand::rng();

    let mut name = vec![0u8];
    name.extend(with_len16(sni.as_bytes()));
    let mut hybrid = mlkem.to_vec();
    hybrid.extend_from_slice(x25519);
    let mut shares = Vec::new();
    shares.extend(u16b(g2));
    shares.extend(with_len16(&[0]));
    shares.extend(u16b(X25519MLKEM768));
    shares.extend(with_len16(&hybrid));
    shares.extend(u16b(X25519));
    shares.extend(with_len16(x25519));
    let groups: Vec<u8> = [g2, X25519MLKEM768, X25519, 0x0017, 0x0018].iter().flat_map(|g| u16b(*g)).collect();
    let sigs: Vec<u8> = [0x0403u16, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601].iter().flat_map(|s| u16b(*s)).collect();
    let mut alpn = Vec::new();
    for p in ["h2", "http/1.1"] {
        alpn.push(p.len() as u8);
        alpn.extend_from_slice(p.as_bytes());
    }
    let versions: Vec<u8> = [6u8].into_iter().chain(u16b(g3)).chain(u16b(0x0304)).chain(u16b(0x0303)).collect();
    // A made-up encrypted_client_hello, as Chrome sends when it has no ECH config.
    let mut ech = vec![0, 0, 1, 0, 1, rng.random()];
    ech.extend(with_len16(&rand::random::<[u8; 32]>()));
    let payload: Vec<u8> = (0..[144, 176, 208, 240][rng.random_range(0..4)]).map(|_| rng.random()).collect();
    ech.extend(with_len16(&payload));

    let mut exts = vec![
        ext(0x0000, &with_len16(&name)),
        ext(0x0017, &[]),
        ext(0xff01, &[0]),
        ext(0x000a, &with_len16(&groups)),
        ext(0x000b, &[1, 0]),
        ext(0x0023, &[]),
        ext(0x0010, &with_len16(&alpn)),
        ext(0x0005, &[1, 0, 0, 0, 0]),
        ext(0x000d, &with_len16(&sigs)),
        ext(0x0012, &[]),
        ext(0x0033, &with_len16(&shares)),
        ext(0x002d, &[1, 1]),
        ext(0x002b, &versions),
        ext(0x001b, &[2, 0, 2]),
        ext(0x44cd, &[0, 3, 2, b'h', b'2']),
        ext(0xfe0d, &ech),
    ];
    // Chrome shuffles its extensions, GREASE first and last.
    for i in (1..exts.len()).rev() {
        exts.swap(i, rng.random_range(0..=i));
    }
    let mut all = ext(g1, &[]);
    for e in exts {
        all.extend(e);
    }
    all.extend(ext(g4, &[0]));

    let suites: Vec<u8> = [grease(), 0x1301, 0x1302, 0x1303, 0xc02b, 0xc02f, 0xc02c, 0xc030, 0xcca9, 0xcca8, 0xc013, 0xc014, 0x009c, 0x009d, 0x002f, 0x0035].iter().flat_map(|s| u16b(*s)).collect();
    let mut body = vec![3, 3];
    body.extend_from_slice(random);
    body.push(32);
    body.extend_from_slice(&[0; 32]);
    body.extend(with_len16(&suites));
    body.extend_from_slice(&[1, 0]);
    body.extend(with_len16(&all));
    let mut msg = vec![1];
    msg.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    msg.extend(body);
    msg
}

/* ---------- reading records and messages ---------- */

async fn read_record(tcp: &mut TcpStream) -> io::Result<([u8; 5], Vec<u8>)> {
    let mut header = [0u8; 5];
    tcp.read_exact(&mut header).await?;
    let len = u16::from_be_bytes([header[3], header[4]]) as usize;
    if len > MAX_RECORD + 256 {
        return Err(io::Error::other("reality: record too long"));
    }
    let mut body = vec![0u8; len];
    tcp.read_exact(&mut body).await?;
    Ok((header, body))
}

/// The next whole handshake message in `buf`, if there is one.
fn take_message(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes([0, buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    Some(buf.drain(..4 + len).collect())
}

/// A minimal DER reader: tag, contents, and what follows.
fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *input.first()?;
    let first = *input.get(1)? as usize;
    let (len, start) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *input.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let end = start.checked_add(len)?;
    Some((tag, input.get(start..end)?, input.get(end..)?))
}

/// An Ed25519 certificate's public key and signature value.
fn ed25519_cert(cert: &[u8]) -> Option<([u8; 32], Vec<u8>)> {
    let (_, cert, _) = der(cert)?;
    let (_, tbs, rest) = der(cert)?;
    let (_, _alg, rest) = der(rest)?;
    let (_, sig, _) = der(rest)?;
    let mut fields = tbs;
    // version [0], serial, signature, issuer, validity, subject, then the key.
    let mut items = Vec::new();
    while let Some((tag, content, rest)) = der(fields) {
        items.push((tag, content));
        fields = rest;
    }
    let skip = if items.first()?.0 == 0xa0 { 1 } else { 0 };
    let (_, spki) = *items.get(skip + 5)?;
    let (_, alg, rest) = der(spki)?;
    if alg != [0x06, 0x03, 0x2b, 0x65, 0x70] {
        return None;
    }
    let (_, key, _) = der(rest)?;
    let key: [u8; 32] = key.get(1..)?.try_into().ok()?;
    Some((key, sig.get(1..)?.to_vec()))
}

/* ---------- the handshake ---------- */

fn decode_key(text: &str) -> Result<[u8; 32]> {
    let t = text.trim().trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).ok_or_else(|| failed("auth"))
}

fn short_id(text: &str) -> Result<[u8; 8]> {
    let t = text.trim();
    if t.len() > 16 || t.len() % 2 == 1 {
        return Err(failed("auth"));
    }
    let mut id = [0u8; 8];
    for (i, b) in id.iter_mut().enumerate().take(t.len() / 2) {
        *b = u8::from_str_radix(&t[2 * i..2 * i + 2], 16).map_err(|_| failed("auth"))?;
    }
    Ok(id)
}

fn protocol(_: impl std::fmt::Debug) -> crate::Error {
    failed("protocol")
}

/// A REALITY connection over `tcp`, its handshake done.
pub async fn connect(mut tcp: TcpStream, p: &ProxyEntry, sni: &str) -> Result<RealityStream> {
    let ProxyKind::Vless { public_key, short_id: sid_text, .. } = &p.kind else { return Err(failed("protocol")) };
    let server_key = decode_key(public_key)?;
    let short = short_id(sid_text)?;

    let x_secret: [u8; 32] = rand::random();
    let x_public = MontgomeryPoint::mul_base_clamped(x_secret).0;
    let (kem_secret, kem_public) = MlKem768::generate_keypair_from_rng(&mut rand::rng());
    let random: [u8; 32] = rand::random();
    let mut hello = client_hello(sni, &random, &kem_public.to_bytes(), &x_public);

    // The session id: who this is, sealed for the server with this hello as associated data.
    let shared = MontgomeryPoint(server_key).mul_clamped(x_secret).0;
    let auth = Hash::Sha256.expand(&Hash::Sha256.hmac(&random[..20], &shared), b"REALITY", 32);
    let mut sid = [0u8; 16];
    sid[..3].copy_from_slice(&VERSION);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    sid[4..8].copy_from_slice(&now.to_be_bytes());
    sid[8..].copy_from_slice(&short);
    let key = LessSafeKey::new(UnboundKey::new(&aead::AES_256_GCM, &auth).unwrap());
    let mut sealed = sid.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(random[20..].try_into().unwrap()), Aad::from(&hello[..]), &mut sealed).map_err(protocol)?;
    hello[39..71].copy_from_slice(&sealed);

    let mut record = vec![22, 3, 1];
    record.extend(with_len16(&hello));
    tcp.write_all(&record).await.map_err(|_| failed("unreachable"))?;
    let mut transcript = hello;

    // ServerHello, in the clear.
    let mut hs = Vec::new();
    let server_hello = loop {
        if let Some(m) = take_message(&mut hs) {
            break m;
        }
        let (header, body) = read_record(&mut tcp).await.map_err(|_| failed("unreachable"))?;
        match header[0] {
            22 => hs.extend(body),
            // An alert before anything else: the server isn't a REALITY one for this.
            21 => return Err(failed("auth")),
            _ => return Err(failed("protocol")),
        }
    };
    if server_hello[0] != 2 {
        return Err(failed("protocol"));
    }
    transcript.extend_from_slice(&server_hello);
    let (suite, group, share) = parse_server_hello(&server_hello[4..]).ok_or_else(|| failed("protocol"))?;
    let secret: Vec<u8> = match group {
        X25519MLKEM768 if share.len() == 1088 + 32 => {
            let ct = ml_kem::Ciphertext::<MlKem768>::try_from(&share[..1088]).map_err(protocol)?;
            let pq = kem_secret.decapsulate(&ct);
            let x: [u8; 32] = share[1088..].try_into().unwrap();
            let mut s = pq.to_vec();
            s.extend_from_slice(&MontgomeryPoint(x).mul_clamped(x_secret).0);
            s
        }
        X25519 if share.len() == 32 => MontgomeryPoint(share.try_into().unwrap()).mul_clamped(x_secret).0.to_vec(),
        _ => return Err(failed("protocol")),
    };

    let h = suite.hash;
    let early = h.extract(&[], &vec![0u8; h.len()]);
    let derived = h.derive(&early, "derived", &[]);
    let hs_secret = h.extract(&derived, &secret);
    let c_hs = h.derive(&hs_secret, "c hs traffic", &transcript);
    let s_hs = h.derive(&hs_secret, "s hs traffic", &transcript);
    let master = h.extract(&h.derive(&hs_secret, "derived", &[]), &vec![0u8; h.len()]);
    let mut rx = Keys::new(suite, s_hs.clone());

    // EncryptedExtensions, Certificate, CertificateVerify, Finished.
    let mut cert_key: Option<[u8; 32]> = None;
    loop {
        let Some(m) = take_message(&mut hs) else {
            let (header, body) = read_record(&mut tcp).await.map_err(|_| failed("unreachable"))?;
            match header[0] {
                20 => continue,
                23 => {
                    let (kind, data) = rx.open(header, body).map_err(protocol)?;
                    if kind != 22 {
                        return Err(failed("protocol"));
                    }
                    hs.extend(data);
                }
                _ => return Err(failed("protocol")),
            }
            continue;
        };
        match m[0] {
            8 => transcript.extend_from_slice(&m),
            11 => {
                let cert = first_certificate(&m[4..]).ok_or_else(|| failed("protocol"))?;
                // Signed with the key both sides derived: this is the REALITY server, not the site.
                let (key, sig) = ed25519_cert(cert).ok_or_else(|| failed("auth"))?;
                let expected = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA512, &auth), &key);
                if expected.as_ref() != sig.as_slice() {
                    return Err(failed("auth"));
                }
                cert_key = Some(key);
                transcript.extend_from_slice(&m);
            }
            15 => {
                let key = cert_key.ok_or_else(|| failed("protocol"))?;
                let body = &m[4..];
                let scheme = u16::from_be_bytes([*body.first().ok_or_else(|| failed("protocol"))?, *body.get(1).ok_or_else(|| failed("protocol"))?]);
                let sig = body.get(4..).ok_or_else(|| failed("protocol"))?;
                let mut signed = vec![0x20u8; 64];
                signed.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
                signed.extend(h.digest(&transcript));
                if scheme != 0x0807 || ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key).verify(&signed, sig).is_err() {
                    return Err(failed("auth"));
                }
                transcript.extend_from_slice(&m);
            }
            20 => {
                if cert_key.is_none() {
                    return Err(failed("protocol"));
                }
                let finished_key = h.expand_label(&s_hs, "finished", &[], h.len());
                if h.hmac(&finished_key, &h.digest(&transcript)) != m[4..] {
                    return Err(failed("auth"));
                }
                transcript.extend_from_slice(&m);
                break;
            }
            _ => return Err(failed("protocol")),
        }
    }

    let c_ap = h.derive(&master, "c ap traffic", &transcript);
    let s_ap = h.derive(&master, "s ap traffic", &transcript);
    let mut tx = Keys::new(suite, c_hs.clone());
    let finished_key = h.expand_label(&c_hs, "finished", &[], h.len());
    let mut fin = vec![20, 0, 0, h.len() as u8];
    fin.extend(h.hmac(&finished_key, &h.digest(&transcript)));
    // A change_cipher_spec first, as clients in middlebox compatibility mode send.
    let mut out = vec![20, 3, 3, 0, 1, 1];
    out.extend(tx.seal(22, &fin));
    tcp.write_all(&out).await.map_err(|_| failed("unreachable"))?;

    Ok(RealityStream {
        tcp,
        rx: Keys::new(suite, s_ap),
        tx: Keys::new(suite, c_ap),
        raw: Vec::new(),
        plain: VecDeque::new(),
        hs: Vec::new(),
        pending: Vec::new(),
        pending_at: 0,
        eof: false,
    })
}

/// The cipher suite, key share group and key share of a ServerHello.
fn parse_server_hello(b: &[u8]) -> Option<(Suite, u16, &[u8])> {
    const RETRY: [u8; 32] = [
        0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91, 0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
    ];
    if b.get(2..34)? == RETRY {
        return None;
    }
    let sid_len = *b.get(34)? as usize;
    let mut at = 35 + sid_len;
    let suite = suite(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))?;
    at += 3;
    let ext_len = u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]) as usize;
    let mut exts = b.get(at + 2..at + 2 + ext_len)?;
    let mut share = None;
    let mut tls13 = false;
    while exts.len() >= 4 {
        let kind = u16::from_be_bytes([exts[0], exts[1]]);
        let len = u16::from_be_bytes([exts[2], exts[3]]) as usize;
        let data = exts.get(4..4 + len)?;
        match kind {
            0x002b => tls13 = data == [3, 4],
            0x0033 => {
                let group = u16::from_be_bytes([*data.first()?, *data.get(1)?]);
                let n = u16::from_be_bytes([*data.get(2)?, *data.get(3)?]) as usize;
                share = Some((group, data.get(4..4 + n)?));
            }
            _ => {}
        }
        exts = &exts[4 + len..];
    }
    let (group, share) = share?;
    tls13.then_some((suite, group, share))
}

/// The first certificate of a Certificate message's body.
fn first_certificate(b: &[u8]) -> Option<&[u8]> {
    let ctx = *b.first()? as usize;
    let list = b.get(1 + ctx + 3..)?;
    let len = u32::from_be_bytes([0, *list.first()?, *list.get(1)?, *list.get(2)?]) as usize;
    list.get(3..3 + len)
}

/* ---------- after the handshake ---------- */

pub struct RealityStream {
    tcp: TcpStream,
    rx: Keys,
    tx: Keys,
    /// Bytes read, not yet a whole record.
    raw: Vec<u8>,
    plain: VecDeque<u8>,
    /// Post-handshake messages (tickets, key updates) across records.
    hs: Vec<u8>,
    pending: Vec<u8>,
    pending_at: usize,
    eof: bool,
}

impl RealityStream {
    fn poll_pending(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.pending_at < self.pending.len() {
            let n = ready!(Pin::new(&mut self.tcp).poll_write(cx, &self.pending[self.pending_at..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.pending_at += n;
        }
        self.pending.clear();
        self.pending_at = 0;
        Poll::Ready(Ok(()))
    }

    /// Takes whole records out of `raw`.
    fn records(&mut self) -> io::Result<()> {
        while self.raw.len() >= 5 {
            let len = u16::from_be_bytes([self.raw[3], self.raw[4]]) as usize;
            if self.raw.len() < 5 + len {
                break;
            }
            let header: [u8; 5] = self.raw[..5].try_into().unwrap();
            let body = self.raw[5..5 + len].to_vec();
            self.raw.drain(..5 + len);
            match header[0] {
                20 => {}
                23 => {
                    let (kind, data) = self.rx.open(header, body)?;
                    match kind {
                        23 => self.plain.extend(data),
                        22 => {
                            self.hs.extend(data);
                            while let Some(m) = take_message(&mut self.hs) {
                                // A key update: theirs from now on, and ours if asked.
                                if m[0] == 24 {
                                    self.rx.update();
                                    if m.get(4) == Some(&1) {
                                        self.pending.extend(self.tx.seal(22, &[24, 0, 0, 1, 0]));
                                        self.tx.update();
                                    }
                                }
                            }
                        }
                        21 => {
                            self.eof = true;
                            return Ok(());
                        }
                        _ => return Err(io::Error::other("reality: unexpected record")),
                    }
                }
                21 => {
                    self.eof = true;
                    return Ok(());
                }
                _ => return Err(io::Error::other("reality: unexpected record")),
            }
        }
        Ok(())
    }
}

impl AsyncRead for RealityStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if !this.plain.is_empty() {
                let n = buf.remaining().min(this.plain.len());
                let (a, b) = this.plain.as_slices();
                let from_a = n.min(a.len());
                buf.put_slice(&a[..from_a]);
                buf.put_slice(&b[..n - from_a]);
                this.plain.drain(..n);
                return Poll::Ready(Ok(()));
            }
            if this.eof {
                return Poll::Ready(Ok(()));
            }
            let mut tmp = [0u8; 32 * 1024];
            let mut rb = ReadBuf::new(&mut tmp);
            ready!(Pin::new(&mut this.tcp).poll_read(cx, &mut rb))?;
            if rb.filled().is_empty() {
                return Poll::Ready(Ok(()));
            }
            this.raw.extend_from_slice(rb.filled());
            this.records()?;
            // A key update may have queued an answer.
            if !this.pending.is_empty() {
                let _ = this.poll_pending(cx)?;
            }
        }
    }
}

impl AsyncWrite for RealityStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_pending(cx))?;
        let n = data.len().min(MAX_RECORD);
        this.pending = this.tx.seal(23, &data[..n]);
        this.pending_at = 0;
        let _ = this.poll_pending(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_pending(cx))?;
        Pin::new(&mut self.tcp).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_pending(cx))?;
        Pin::new(&mut self.tcp).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkdf_matches_rfc_5869() {
        // Test case 1.
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0..13).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let okm = Hash::Sha256.expand(&Hash::Sha256.extract(&salt, &ikm), &info, 42);
        assert_eq!(okm[..8], [0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a]);
        assert_eq!(okm[32..], [0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65]);
    }

    #[test]
    fn the_hello_has_its_session_id_where_reality_looks() {
        let hello = client_hello("www.microsoft.com", &[7; 32], &[1; 1184], &[2; 32]);
        assert_eq!(hello[0], 1);
        assert_eq!(&hello[6..38], &[7; 32]);
        assert_eq!(hello[38], 32);
        assert_eq!(&hello[39..71], &[0; 32]);
        assert_eq!(u32::from_be_bytes([0, hello[1], hello[2], hello[3]]) as usize, hello.len() - 4);
    }

    #[test]
    fn short_ids_and_keys() {
        assert_eq!(short_id("6ba851").unwrap(), [0x6b, 0xa8, 0x51, 0, 0, 0, 0, 0]);
        assert_eq!(short_id("").unwrap(), [0; 8]);
        assert!(short_id("abc").is_err());
        assert_eq!(decode_key("Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw").unwrap().len(), 32);
    }
}
