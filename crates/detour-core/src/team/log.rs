//! On-storage format of a team vault: signed, per-device, append-only logs.
//!
//! ```text
//! <root>/vault.json                  { format, id, root }
//! <root>/ops/<device>/00000001.json  { body: base64(json Body), sig: base64 }
//! ```
//!
//! The signature covers the exact body bytes, so no canonical JSON is needed.
//! A device only ever writes its own directory, so storage never sees two
//! writers on one file.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::{Identity, PublicIdentity};
use crate::{Error, Result};

pub const FORMAT: u32 = 1;

fn corrupt(what: impl Into<String>) -> Error {
    Error::Internal { message: format!("team vault: {}", what.into()) }
}

/// Hybrid logical clock: wall time plus a counter, so ordering survives skewed clocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    pub ms: u64,
    pub c: u32,
}

impl Hlc {
    pub fn tick(latest_seen: Hlc) -> Hlc {
        let now = crate::model::now_ms().max(0) as u64;
        if now > latest_seen.ms { Hlc { ms: now, c: 0 } } else { Hlc { ms: latest_seen.ms, c: latest_seen.c + 1 } }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Payload: epoch keys, age-encrypted to their recipients.
    Keys,
    /// Payload: `Vec<Op>` encrypted with the epoch key.
    Data,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    pub v: u32,
    pub vault: String,
    pub device: String,
    /// Author's signing key id.
    pub author: String,
    pub seq: u64,
    /// Hash of this device's previous log file.
    pub prev: Option<String>,
    pub hlc: Hlc,
    /// Highest sequence number read from every device when this was written.
    pub seen: BTreeMap<String, u64>,
    pub kind: Kind,
    pub epoch: u32,
    pub payload: String,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    body: String,
    sig: String,
}

/// A verified log entry.
#[derive(Debug, Clone)]
pub struct Entry {
    pub body: Body,
    /// Hash of the file, referenced by the next entry of the same device.
    pub hash: String,
}

/// Every epoch key the reader can open, by epoch number.
pub type EpochKeys = BTreeMap<u32, [u8; 32]>;

#[derive(Serialize, Deserialize)]
struct KeysPayload {
    keys: BTreeMap<u32, String>,
}

pub fn new_epoch_key() -> [u8; 32] {
    rand::rng().random()
}

pub fn seal_keys(keys: &EpochKeys, recipients: &[PublicIdentity]) -> Result<String> {
    let payload = KeysPayload { keys: keys.iter().map(|(e, k)| (*e, B64.encode(k))).collect() };
    let plain = serde_json::to_vec(&payload)?;
    let rs: Vec<age::x25519::Recipient> = recipients.iter().map(PublicIdentity::recipient).collect();
    let enc = age::Encryptor::with_recipients(rs.iter().map(|r| r as &dyn age::Recipient)).map_err(|e| corrupt(e.to_string()))?;
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out)?;
    w.write_all(&plain)?;
    w.finish()?;
    Ok(B64.encode(out))
}

/// `None` when the keys were not addressed to `me`.
pub fn open_keys(payload: &str, me: &Identity) -> Option<EpochKeys> {
    let ct = B64.decode(payload).ok()?;
    let plain = age::decrypt(&me.age, &ct).ok()?;
    let p: KeysPayload = serde_json::from_slice(&plain).ok()?;
    p.keys.into_iter().map(|(e, k)| Some((e, B64.decode(k).ok()?.try_into().ok()?))).collect()
}

pub fn seal_data(key: &[u8; 32], plain: &[u8]) -> Result<String> {
    let nonce: [u8; 12] = rand::rng().random();
    let ct = ChaCha20Poly1305::new(Key::from_slice(key)).encrypt(Nonce::from_slice(&nonce), plain).map_err(|_| corrupt("encrypt"))?;
    Ok(B64.encode([nonce.as_slice(), &ct].concat()))
}

pub fn open_data(key: &[u8; 32], payload: &str) -> Option<Vec<u8>> {
    let raw = B64.decode(payload).ok()?;
    if raw.len() < 12 {
        return None;
    }
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()
}

pub fn sign(body: &Body, me: &Identity) -> Result<Vec<u8>> {
    let body_bytes = serde_json::to_vec(body)?;
    let env = Envelope { body: B64.encode(&body_bytes), sig: B64.encode(me.sign(&body_bytes)) };
    Ok(serde_json::to_vec(&env)?)
}

/// Parses and verifies one log file. Errors describe why it was rejected.
pub fn verify(file: &[u8]) -> std::result::Result<Entry, String> {
    let env: Envelope = serde_json::from_slice(file).map_err(|_| "unreadable".to_string())?;
    let body_bytes = B64.decode(&env.body).map_err(|_| "unreadable body")?;
    let sig = B64.decode(&env.sig).map_err(|_| "unreadable signature")?;
    let body: Body = serde_json::from_slice(&body_bytes).map_err(|_| "unreadable body")?;
    let vk = PublicIdentity::from_key_id(&body.author).ok_or("unknown author key")?;
    let ok = ed25519_dalek::Signature::from_slice(&sig).map(|s| vk.verify_strict(&body_bytes, &s).is_ok()).unwrap_or(false);
    if !ok {
        return Err("bad signature".into());
    }
    Ok(Entry { body, hash: file_hash(file) })
}

pub fn file_hash(file: &[u8]) -> String {
    B64.encode(Sha256::digest(file))
}

/* ---------- folder transport ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultMeta {
    pub format: u32,
    pub id: String,
    /// Detour ID of the creator: the root of trust a joining member confirms.
    pub root: String,
}

/// A team vault in a plain directory (Syncthing, Dropbox, SMB, a Git checkout...).
#[derive(Debug, Clone)]
pub struct Folder {
    root: PathBuf,
}

impl Folder {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    pub fn read_meta(&self) -> Result<Option<VaultMeta>> {
        let p = self.root.join("vault.json");
        if !p.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&std::fs::read(p)?).map_err(|_| corrupt("vault.json"))?))
    }

    pub fn write_meta(&self, meta: &VaultMeta) -> Result<()> {
        write_new(&self.root.join("vault.json"), &serde_json::to_vec_pretty(meta)?)
    }

    /// All log files: device -> files in sequence order (file name = sequence number).
    pub fn read_logs(&self) -> Result<BTreeMap<String, Vec<(u64, Vec<u8>)>>> {
        let mut out = BTreeMap::new();
        let ops = self.root.join("ops");
        if !ops.is_dir() {
            return Ok(out);
        }
        for dev in std::fs::read_dir(ops)? {
            let dev = dev?;
            if !dev.file_type()?.is_dir() {
                continue;
            }
            let name = dev.file_name().to_string_lossy().to_string();
            let mut files = Vec::new();
            for f in std::fs::read_dir(dev.path())? {
                let f = f?;
                let fname = f.file_name().to_string_lossy().to_string();
                // Sync tools leave temp and conflict copies around; only `<seq>.json` counts.
                let Some(seq) = fname.strip_suffix(".json").and_then(|s| s.parse::<u64>().ok()) else { continue };
                files.push((seq, std::fs::read(f.path())?));
            }
            files.sort_by_key(|(s, _)| *s);
            out.insert(name, files);
        }
        Ok(out)
    }

    pub fn write_log(&self, device: &str, seq: u64, bytes: &[u8]) -> Result<()> {
        write_new(&self.root.join("ops").join(device).join(format!("{seq:08}.json")), bytes)
    }
}

/// Writes a file that must not exist yet, via a temp file so readers never see half of it.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Err(corrupt(format!("{} already exists", path.display())));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
