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
//!
//! A body carries encrypted parts. A data part is encrypted with a symmetric
//! key and names it only by its id (a hash of the key), so the storage learns
//! nothing but which entries share a key. A key drop is age-encrypted to one
//! recipient and hands over keys; age hides who the recipient is.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key as AeadKey, Nonce};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::{Identity, PublicIdentity};
use crate::{Error, Result};

pub const FORMAT: u32 = 2;

pub type Key = [u8; 32];

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

/// The last entry of a device: its sequence number and file hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub seq: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Part {
    /// Encrypted with the key whose id is `kid`.
    Data { kid: String, ct: String },
    /// Keys for one recipient, age-encrypted.
    Drop { ct: String },
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
    /// Head of every device the author had read when writing this.
    pub seen: BTreeMap<String, Head>,
    /// Empty for an acknowledgement.
    pub parts: Vec<Part>,
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

pub fn new_key() -> Key {
    rand::rng().random()
}

/// Public id of a key: lets a reader find the key without the storage learning it.
pub fn key_id(key: &Key) -> String {
    let mut h = Sha256::new();
    h.update(b"detour key id v2");
    h.update(key);
    B64.encode(&h.finalize()[..16])
}

#[derive(Serialize, Deserialize)]
struct DropPayload {
    keys: Vec<String>,
}

pub fn seal_drop(keys: &[Key], to: &PublicIdentity) -> Result<Part> {
    let plain = serde_json::to_vec(&DropPayload { keys: keys.iter().map(|k| B64.encode(k)).collect() })?;
    let r = to.recipient();
    let enc = age::Encryptor::with_recipients(std::iter::once(&r as &dyn age::Recipient)).map_err(|e| corrupt(e.to_string()))?;
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out)?;
    w.write_all(&plain)?;
    w.finish()?;
    Ok(Part::Drop { ct: B64.encode(out) })
}

/// `None` when the drop is not addressed to `me`.
pub fn open_drop(ct: &str, me: &Identity) -> Option<Vec<Key>> {
    let raw = B64.decode(ct).ok()?;
    let plain = age::decrypt(&me.age, &raw).ok()?;
    let p: DropPayload = serde_json::from_slice(&plain).ok()?;
    p.keys.into_iter().map(|k| B64.decode(k).ok()?.try_into().ok()).collect()
}

pub fn seal_data(key: &Key, plain: &[u8]) -> Result<Part> {
    let kid = key_id(key);
    let nonce: [u8; 12] = rand::rng().random();
    let ct = ChaCha20Poly1305::new(AeadKey::from_slice(key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: kid.as_bytes() })
        .map_err(|_| corrupt("encrypt"))?;
    Ok(Part::Data { ct: B64.encode([nonce.as_slice(), &ct].concat()), kid })
}

pub fn open_data(key: &Key, kid: &str, ct: &str) -> Option<Vec<u8>> {
    let raw = B64.decode(ct).ok()?;
    if raw.len() < 12 {
        return None;
    }
    ChaCha20Poly1305::new(AeadKey::from_slice(key)).decrypt(Nonce::from_slice(&raw[..12]), Payload { msg: &raw[12..], aad: kid.as_bytes() }).ok()
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

/// A file in the vault folder that is not an entry at its own path.
#[derive(Debug, Clone)]
pub struct Extra {
    /// Relative to the vault root, with `/`.
    pub path: String,
    /// Left behind by a sync tool or by us (temp files): not a problem by itself.
    pub debris: bool,
    /// Set for files inside a device directory, which may be a second version of an entry.
    pub device: Option<String>,
    pub bytes: Option<Vec<u8>>,
}

/// Everything found in the vault folder.
#[derive(Debug, Default)]
pub struct Scan {
    /// Device -> entries in sequence order (file name = sequence number).
    pub logs: BTreeMap<String, Vec<(u64, Vec<u8>)>>,
    pub extras: Vec<Extra>,
}

/// Names sync tools and editors leave around, and our own temp files.
fn is_debris(name: &str) -> bool {
    const EXACT: [&str; 9] = [".stfolder", ".stignore", ".stversions", "desktop.ini", ".DS_Store", "Thumbs.db", ".dropbox", ".dropbox.cache", ".git"];
    EXACT.contains(&name)
        || name.starts_with(".syncthing.")
        || name.starts_with("~syncthing~")
        || name.ends_with(".tmp")
        || name.contains(".sync-conflict-")
        || name.contains("(conflicted copy")
        || name.starts_with("._")
}

/// Directories whose contents are a sync tool's business.
fn is_debris_dir(name: &str) -> bool {
    matches!(name, ".stversions" | ".dropbox.cache" | ".git" | ".stfolder")
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
        let meta: VaultMeta = serde_json::from_slice(&std::fs::read(p)?).map_err(|_| corrupt("vault.json"))?;
        if meta.format != FORMAT {
            return Err(Error::NotATeamVault);
        }
        Ok(Some(meta))
    }

    pub fn write_meta(&self, meta: &VaultMeta) -> Result<()> {
        write_new(&self.root.join("vault.json"), &serde_json::to_vec_pretty(meta)?)
    }

    /// Reads every file in the folder and sorts it into entries and everything else.
    pub fn scan(&self) -> Result<Scan> {
        let mut scan = Scan::default();
        for item in std::fs::read_dir(&self.root)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().to_string();
            let is_dir = item.file_type()?.is_dir();
            match (name.as_str(), is_dir) {
                ("vault.json", false) | ("ops", true) => {}
                _ if is_dir && is_debris_dir(&name) => {}
                _ => self.extra(&mut scan, &item.path(), is_debris(&name), None)?,
            }
        }
        let ops = self.root.join("ops");
        if !ops.is_dir() {
            return Ok(scan);
        }
        for dev in std::fs::read_dir(ops)? {
            let dev = dev?;
            let name = dev.file_name().to_string_lossy().to_string();
            if !dev.file_type()?.is_dir() {
                self.extra(&mut scan, &dev.path(), is_debris(&name), None)?;
                continue;
            }
            let mut files = Vec::new();
            for f in std::fs::read_dir(dev.path())? {
                let f = f?;
                let fname = f.file_name().to_string_lossy().to_string();
                let seq = fname.strip_suffix(".json").and_then(|s| s.parse::<u64>().ok());
                match seq {
                    Some(seq) if fname == format!("{seq:08}.json") && f.file_type()?.is_file() => files.push((seq, std::fs::read(f.path())?)),
                    _ => self.extra(&mut scan, &f.path(), is_debris(&fname), Some(name.clone()))?,
                }
            }
            files.sort_by_key(|(s, _)| *s);
            scan.logs.insert(name, files);
        }
        Ok(scan)
    }

    fn extra(&self, scan: &mut Scan, path: &Path, debris: bool, device: Option<String>) -> Result<()> {
        let rel = path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        // Only a file inside a device directory can be a competing version of an entry.
        let bytes = if device.is_some() && path.is_file() { std::fs::read(path).ok() } else { None };
        scan.extras.push(Extra { path: rel, debris, device, bytes });
        Ok(())
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
