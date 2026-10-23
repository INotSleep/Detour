//! Member identities and the Detour ID.
//!
//! An identity is an age X25519 key (encryption) plus an Ed25519 key
//! (signatures). Its public half, the Detour ID, is one copyable string:
//! `detour1` + base32(x25519 public ‖ ed25519 public ‖ 4-byte checksum).

use std::str::FromStr;

use age::secrecy::ExposeSecret;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use bech32::{FromBase32, ToBase32};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

const ID_PREFIX: &str = "detour1";
const RECOVERY_PREFIX: &str = "DETOUR-RECOVERY-";

fn bad(what: &str) -> Error {
    Error::InvalidId { reason: what.to_string() }
}

/// A member's secret keys.
#[derive(Clone)]
pub struct Identity {
    pub(crate) age: age::x25519::Identity,
    pub(crate) signing: SigningKey,
}

/// What others know about a member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicIdentity {
    x25519: [u8; 32],
    pub(crate) verifying: VerifyingKey,
}

/// Serialized form kept inside the personal vault.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredIdentity {
    age: String,
    signing: String,
}

impl std::fmt::Debug for StoredIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoredIdentity(…)")
    }
}

impl Identity {
    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::rng().random();
        Self { age: age::x25519::Identity::generate(), signing: SigningKey::from_bytes(&seed) }
    }

    pub fn public(&self) -> PublicIdentity {
        PublicIdentity { x25519: recipient_bytes(&self.age.to_public()), verifying: self.signing.verifying_key() }
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.signing.sign(msg).to_bytes().to_vec()
    }

    pub fn to_stored(&self) -> StoredIdentity {
        StoredIdentity { age: self.age.to_string().expose_secret().to_string(), signing: B64.encode(self.signing.to_bytes()) }
    }

    pub fn from_stored(s: &StoredIdentity) -> Result<Self> {
        let age = age::x25519::Identity::from_str(&s.age).map_err(|_| bad("stored identity"))?;
        let bytes: [u8; 32] = B64.decode(&s.signing).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| bad("stored identity"))?;
        Ok(Self { age, signing: SigningKey::from_bytes(&bytes) })
    }

    /// The recovery key text: both secret keys, grouped for writing down.
    pub fn to_recovery_key(&self) -> String {
        let mut raw = Vec::with_capacity(68);
        raw.extend_from_slice(&age_secret_bytes(&self.age));
        raw.extend_from_slice(&self.signing.to_bytes());
        raw.extend_from_slice(&checksum(&raw));
        let text = base32(&raw).to_uppercase();
        let groups: Vec<&str> = text.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect();
        format!("{RECOVERY_PREFIX}{}", groups.join("-"))
    }

    pub fn from_recovery_key(text: &str) -> Result<Self> {
        let clean: String = text.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let prefix: String = RECOVERY_PREFIX.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let body = clean.strip_prefix(&prefix).unwrap_or(&clean);
        let raw = unbase32(&body.to_lowercase()).ok_or_else(|| bad("recovery key"))?;
        if raw.len() != 68 || checksum(&raw[..64]) != raw[64..] {
            return Err(bad("recovery key"));
        }
        let age_secret: [u8; 32] = raw[..32].try_into().unwrap();
        let signing: [u8; 32] = raw[32..64].try_into().unwrap();
        Ok(Self { age: age_identity_from_bytes(&age_secret)?, signing: SigningKey::from_bytes(&signing) })
    }
}

impl PublicIdentity {
    pub fn recipient(&self) -> age::x25519::Recipient {
        let enc = bech32::encode("age", self.x25519.to_base32(), bech32::Variant::Bech32).expect("valid hrp");
        age::x25519::Recipient::from_str(&enc).expect("valid recipient")
    }

    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        let Ok(sig) = ed25519_dalek::Signature::from_slice(sig) else { return false };
        self.verifying.verify(msg, &sig).is_ok()
    }

    /// Stable short id used in logs and member lists.
    pub fn key_id(&self) -> String {
        B64.encode(self.verifying.to_bytes())
    }

    pub fn from_key_id(id: &str) -> Option<VerifyingKey> {
        let bytes: [u8; 32] = B64.decode(id).ok()?.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    }

    /// `detour1…`
    pub fn detour_id(&self) -> String {
        let mut raw = Vec::with_capacity(68);
        raw.extend_from_slice(&self.x25519);
        raw.extend_from_slice(self.verifying.as_bytes());
        raw.extend_from_slice(&checksum(&raw));
        format!("{ID_PREFIX}{}", base32(&raw))
    }

    pub fn parse(id: &str) -> Result<Self> {
        let id = id.trim().to_lowercase();
        let body = id.strip_prefix(ID_PREFIX).ok_or_else(|| bad("detour id"))?;
        let raw = unbase32(body).ok_or_else(|| bad("detour id"))?;
        if raw.len() != 68 || checksum(&raw[..64]) != raw[64..] {
            return Err(bad("detour id"));
        }
        let verifying = VerifyingKey::from_bytes(raw[32..64].try_into().unwrap()).map_err(|_| bad("detour id"))?;
        Ok(Self { x25519: raw[..32].try_into().unwrap(), verifying })
    }

    /// Short fingerprint for comparing out of band, e.g. `K7QM-2XPA-9DTE-WN4F`.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.detour_id().as_bytes());
        let text = base32(&digest[..10]).to_uppercase();
        text.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
    }
}

fn checksum(data: &[u8]) -> [u8; 4] {
    let d = Sha256::digest(data);
    [d[0], d[1], d[2], d[3]]
}

fn recipient_bytes(r: &age::x25519::Recipient) -> [u8; 32] {
    let (_, data, _) = bech32::decode(&r.to_string()).expect("age recipients are bech32");
    Vec::<u8>::from_base32(&data).expect("valid base32").try_into().expect("32-byte key")
}

fn age_secret_bytes(id: &age::x25519::Identity) -> [u8; 32] {
    let (_, data, _) = bech32::decode(&id.to_string().expose_secret().to_lowercase()).expect("age identities are bech32");
    Vec::<u8>::from_base32(&data).expect("valid base32").try_into().expect("32-byte key")
}

fn age_identity_from_bytes(bytes: &[u8; 32]) -> Result<age::x25519::Identity> {
    let enc = bech32::encode("age-secret-key-", bytes.to_base32(), bech32::Variant::Bech32).map_err(|_| bad("recovery key"))?;
    age::x25519::Identity::from_str(&enc.to_uppercase()).map_err(|_| bad("recovery key"))
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detour_id_roundtrips_and_rejects_typos() {
        let me = Identity::generate();
        let id = me.public().detour_id();
        assert!(id.starts_with("detour1"));
        assert_eq!(PublicIdentity::parse(&id).unwrap(), me.public());
        assert_eq!(PublicIdentity::parse(&id.to_uppercase()).unwrap(), me.public());
        let mut typo = id.clone().into_bytes();
        typo[20] = if typo[20] == b'a' { b'b' } else { b'a' };
        assert!(PublicIdentity::parse(std::str::from_utf8(&typo).unwrap()).is_err());
    }

    #[test]
    fn recipient_from_detour_id_decrypts_for_owner() {
        let me = Identity::generate();
        let other = PublicIdentity::parse(&me.public().detour_id()).unwrap();
        let ct = age::encrypt(&other.recipient(), b"hello").unwrap();
        assert_eq!(age::decrypt(&me.age, &ct).unwrap(), b"hello");
    }

    #[test]
    fn signatures_verify_only_for_signer() {
        let a = Identity::generate();
        let b = Identity::generate();
        let sig = a.sign(b"msg");
        assert!(a.public().verify(b"msg", &sig));
        assert!(!a.public().verify(b"other", &sig));
        assert!(!b.public().verify(b"msg", &sig));
    }

    #[test]
    fn recovery_key_restores_identity() {
        let me = Identity::generate();
        let key = me.to_recovery_key();
        assert!(key.starts_with(RECOVERY_PREFIX));
        let back = Identity::from_recovery_key(&key.to_lowercase().replace('-', " ")).unwrap();
        assert_eq!(back.public(), me.public());
        let mut tampered = key.clone().into_bytes();
        let i = RECOVERY_PREFIX.len() + 2;
        tampered[i] = if tampered[i] == b'Q' { b'R' } else { b'Q' };
        assert!(Identity::from_recovery_key(std::str::from_utf8(&tampered).unwrap()).is_err());
    }

    #[test]
    fn stored_identity_roundtrips() {
        let me = Identity::generate();
        assert_eq!(Identity::from_stored(&me.to_stored()).unwrap().public(), me.public());
    }
}
