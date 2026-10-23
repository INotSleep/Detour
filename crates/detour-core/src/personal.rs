//! Syncing the personal vault between one person's devices.
//!
//! The synced copy is a vault in the team format with a single member: the
//! person's identity, which every one of their devices holds. Each device
//! keeps its own personal vault file as before; a sync is a three-way merge:
//! what this device changed since the last sync (against the base it
//! remembers) is written to the shared vault, then the shared vault's records
//! become this device's records and the new base.
//!
//! A new device gets the identity through a pairing offer: an existing device
//! writes the identity to `pair/` sealed with a one-time code (age scrypt),
//! and the new device opens it with the code the user types in. The storage
//! never sees the code, and an offer expires after an hour. The identity's
//! recovery key works in place of a code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::identity::{Identity, StoredIdentity};
use crate::model::*;
use crate::team::log::VaultMeta;
use crate::team::state::Coll;
use crate::team::{Change, from_fields};
use crate::{Error, Result};

/// How long a pairing code works.
pub const OFFER_TTL_MS: Millis = 60 * 60 * 1000;

/// Crockford's base32: no I, L, O or U to misread.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The first symbols of a code name the offer's file; the rest is the secret.
const LOCATOR: usize = 4;
const SECRET: usize = 12;

/// One record of the synced vault, as this device last saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecord {
    pub coll: Coll,
    pub fields: BTreeMap<String, Value>,
}

/// A code another device can join with.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    /// `XXXX-XXXX-XXXX-XXXX`
    pub code: String,
    pub expires_at: Millis,
}

/// What a pairing offer holds once opened.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Offer {
    vault: String,
    identity: StoredIdentity,
    expires_at: Millis,
}

/* ---------- records ---------- */

/// A stable record id for something that has no id of its own (a pinned key
/// by endpoint, an own login by team host), keyed with the identity so the
/// storage can't guess endpoints from it.
fn derived_id(me: &Identity, kind: &str, name: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"detour personal record id\0");
    h.update(me.signing.to_bytes());
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(name.as_bytes());
    B64.encode(&h.finalize()[..16])
}

fn fields_of<T: Serialize>(t: &T, skip: &[&str]) -> BTreeMap<String, Value> {
    crate::team::fields_of(t, skip)
}

/// Everything of a personal vault that syncs, by record id.
/// Login times stay on each device; teams and the identity are not records.
pub fn records_of(v: &VaultData, me: &Identity) -> BTreeMap<String, SyncRecord> {
    let mut out = BTreeMap::new();
    let mut put = |id: String, coll: Coll, fields: BTreeMap<String, Value>| {
        out.insert(id, SyncRecord { coll, fields });
    };
    for h in &v.hosts {
        put(h.id.clone(), Coll::Host, fields_of(h, &["lastConnected"]));
    }
    for k in &v.keys {
        put(k.id.clone(), Coll::Key, fields_of(k, &[]));
    }
    for s in &v.snippets {
        put(s.id.clone(), Coll::Snippet, fields_of(s, &[]));
    }
    for t in &v.tunnels {
        put(t.id.clone(), Coll::Tunnel, fields_of(t, &[]));
    }
    for k in &v.known_hosts {
        put(derived_id(me, "known", &k.endpoint), Coll::KnownHost, fields_of(k, &[]));
    }
    for l in &v.team_links {
        put(derived_id(me, "team", &l.team_id), Coll::TeamLink, fields_of(l, &[]));
    }
    for (key, o) in &v.team_own {
        let mut f = fields_of(o, &[]);
        f.insert("key".into(), Value::String(key.clone()));
        put(derived_id(me, "own", key), Coll::OwnLogin, f);
    }
    out
}

/// What this device changed since `base`, to write to the synced vault.
pub fn changes(base: &BTreeMap<String, SyncRecord>, local: &BTreeMap<String, SyncRecord>) -> Vec<Change> {
    let mut out = Vec::new();
    for (id, r) in local {
        let fields = match base.get(id) {
            // Only the fields changed here: others may have changed the rest meanwhile.
            Some(b) if b.coll == r.coll => r.fields.iter().filter(|(k, v)| b.fields.get(*k) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => r.fields.clone(),
        };
        if !fields.is_empty() {
            out.push(Change::Put { coll: r.coll, id: id.clone(), fields });
        }
    }
    for id in base.keys().filter(|id| !local.contains_key(*id)) {
        out.push(Change::Delete { id: id.clone() });
    }
    out
}

/// Keeps the order this device had, with new records after it.
fn merged<T, K: Fn(&T) -> String>(old: &[T], new: Vec<T>, key: K) -> Vec<T> {
    let pos: BTreeMap<String, usize> = old.iter().enumerate().map(|(i, t)| (key(t), i)).collect();
    let mut items: Vec<(usize, T)> = new.into_iter().map(|t| (pos.get(&key(&t)).copied().unwrap_or(usize::MAX), t)).collect();
    items.sort_by_key(|(i, _)| *i);
    items.into_iter().map(|(_, t)| t).collect()
}

fn of<T: serde::de::DeserializeOwned>(records: &BTreeMap<String, SyncRecord>, coll: Coll) -> Vec<T> {
    records.iter().filter(|(_, r)| r.coll == coll).filter_map(|(id, r)| from_fields(id, r.fields.clone())).collect()
}

/// Makes the synced records this device's records, keeping what never syncs.
/// Returns the teams taken off by another device (to leave here as well).
pub fn adopt(v: &mut VaultData, records: &BTreeMap<String, SyncRecord>) -> Vec<String> {
    let links: Vec<TeamLink> = records.values().filter(|r| r.coll == Coll::TeamLink).filter_map(|r| serde_json::from_value(Value::Object(r.fields.clone().into_iter().collect())).ok()).collect();
    let gone: Vec<String> = v.team_links.iter().filter(|old| !links.iter().any(|l| l.team_id == old.team_id)).map(|l| l.team_id.clone()).collect();
    v.team_links = merged(&v.team_links, links, |l| l.team_id.clone());
    let last: BTreeMap<String, Option<Millis>> = v.hosts.iter().map(|h| (h.id.clone(), h.last_connected)).collect();
    let mut hosts: Vec<Host> = of(records, Coll::Host);
    for h in &mut hosts {
        h.last_connected = last.get(&h.id).copied().flatten();
    }
    v.hosts = merged(&v.hosts, hosts, |h| h.id.clone());
    v.keys = merged(&v.keys, of(records, Coll::Key), |k| k.id.clone());
    v.snippets = merged(&v.snippets, of(records, Coll::Snippet), |s| s.id.clone());
    v.tunnels = merged(&v.tunnels, of(records, Coll::Tunnel), |t| t.id.clone());
    v.known_hosts = merged(&v.known_hosts, of(records, Coll::KnownHost), |k| k.endpoint.clone());
    v.team_own = records
        .values()
        .filter(|r| r.coll == Coll::OwnLogin)
        .filter_map(|r| {
            let key = r.fields.get("key")?.as_str()?.to_string();
            Some((key, serde_json::from_value(Value::Object(r.fields.clone().into_iter().collect())).ok()?))
        })
        .collect();
    gone
}

/* ---------- pairing ---------- */

fn random_symbols(n: usize) -> String {
    let mut rng = rand::rng();
    (0..n).map(|_| ALPHABET[rng.random_range(0..32)] as char).collect()
}

/// Upper case, without separators, with look-alike letters read as digits.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

fn pair_dir(vault: &Path) -> PathBuf {
    vault.join("pair")
}

/// The offer file a code points to: `pair/<locator>-<expiry>.age`.
fn offer_file(vault: &Path, locator: &str) -> Option<PathBuf> {
    let prefix = format!("{}-", locator.to_lowercase());
    std::fs::read_dir(pair_dir(vault)).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".age")))
}

/// Seals this person's identity for a new device under a fresh code.
pub fn offer(vault: &Path, vault_id: &str, me: &Identity) -> Result<Pairing> {
    remove_expired(vault);
    let locator = random_symbols(LOCATOR);
    let secret = random_symbols(SECRET);
    let expires_at = now_ms() + OFFER_TTL_MS;
    let plain = serde_json::to_vec(&Offer { vault: vault_id.to_string(), identity: me.to_stored(), expires_at })?;
    let recipient = age::scrypt::Recipient::new(SecretString::from(secret.clone()));
    let sealed = age::encrypt(&recipient, &plain).map_err(|e| Error::Internal { message: e.to_string() })?;
    let dir = pair_dir(vault);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{}-{expires_at}.age", locator.to_lowercase())), sealed)?;
    let all = format!("{locator}{secret}");
    let groups: Vec<&str> = all.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect();
    Ok(Pairing { code: groups.join("-"), expires_at })
}

/// Opens an offer with the code the user typed, or takes a recovery key.
/// Either way the identity must be the vault's creator.
pub fn redeem(vault: &Path, meta: &VaultMeta, code: &str) -> Result<Identity> {
    let identity = if code.trim().to_uppercase().starts_with("DETOUR-RECOVERY") {
        Identity::from_recovery_key(code).map_err(|_| Error::InvalidCode)?
    } else {
        let code = normalize(code);
        if code.len() != LOCATOR + SECRET || !code.bytes().all(|b| ALPHABET.contains(&b)) {
            return Err(Error::InvalidCode);
        }
        let file = offer_file(vault, &code[..LOCATOR]).ok_or(Error::InvalidCode)?;
        let id = age::scrypt::Identity::new(SecretString::from(code[LOCATOR..].to_string()));
        let plain = age::decrypt(&id, &std::fs::read(file)?).map_err(|_| Error::InvalidCode)?;
        let offer: Offer = serde_json::from_slice(&plain).map_err(|_| Error::InvalidCode)?;
        if offer.vault != meta.id {
            return Err(Error::InvalidCode);
        }
        if offer.expires_at < now_ms() {
            return Err(Error::CodeExpired);
        }
        Identity::from_stored(&offer.identity)?
    };
    if identity.public().detour_id() != meta.root {
        return Err(Error::InvalidCode);
    }
    Ok(identity)
}

/// Deletes offers past their time. Anyone may: they are useless by then.
pub fn remove_expired(vault: &Path) {
    let Ok(dir) = std::fs::read_dir(pair_dir(vault)) else { return };
    let now = now_ms();
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let expiry = name.strip_suffix(".age").and_then(|n| n.split_once('-')).and_then(|(_, t)| t.parse::<Millis>().ok());
        if expiry.is_some_and(|t| t < now) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_read_back_with_look_alikes_and_separators() {
        assert_eq!(normalize("abcd-ef0o il1x"), "ABCDEF00111X");
    }

    #[test]
    fn changes_carry_only_what_this_device_changed() {
        let rec = |coll, pairs: &[(&str, &str)]| SyncRecord { coll, fields: pairs.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect() };
        let base = BTreeMap::from([("a".to_string(), rec(Coll::Snippet, &[("name", "x"), ("command", "ls")])), ("b".to_string(), rec(Coll::Snippet, &[("name", "y"), ("command", "pwd")]))]);
        let local = BTreeMap::from([("a".to_string(), rec(Coll::Snippet, &[("name", "x2"), ("command", "ls")])), ("c".to_string(), rec(Coll::Snippet, &[("name", "z"), ("command", "id")]))]);
        let ch = changes(&base, &local);
        assert_eq!(ch.len(), 3);
        assert!(matches!(&ch[0], Change::Put { id, fields, .. } if id == "a" && fields.len() == 1 && fields.contains_key("name")));
        assert!(matches!(&ch[1], Change::Put { id, fields, .. } if id == "c" && fields.len() == 2));
        assert!(matches!(&ch[2], Change::Delete { id } if id == "b"));
    }
}
