//! A Detour file: hosts with what logs in to them (keys, passwords, proxies, pinned server
//! keys), sealed with a password of its own, for another Detour to take in. An ssh config
//! can't carry passwords or proxies Detour reaches its hosts through; this can.
//!
//! The file is age, armored, sealed with scrypt (as the vault's master password): JSON
//! inside, with a version so a newer file is refused rather than misread.

use std::io::{Read, Write};

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::model::{Group, Host, KeyEntry, KnownHost, ProxyEntry};
use crate::{Error, Result};

const FORMAT: &str = "detour-hosts";
const VERSION: u32 = 1;

/// What goes into a file besides the hosts themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Include {
    /// Saved passwords of the hosts and of their proxies.
    pub passwords: bool,
    /// The keys the hosts log in with (and their saved passphrases).
    pub keys: bool,
    /// Saved proxies the hosts go through.
    pub proxies: bool,
    /// The servers' pinned keys.
    pub known_hosts: bool,
    /// The hosts' groups and their places in them.
    pub groups: bool,
}

impl Include {
    pub const ALL: Include = Include { passwords: true, keys: true, proxies: true, known_hosts: true, groups: true };
}

/// What a file holds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
    #[serde(default)]
    pub proxies: Vec<ProxyEntry>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub known_hosts: Vec<KnownHost>,
}

impl Bundle {
    pub fn new() -> Self {
        Self { format: FORMAT.into(), version: VERSION, ..Default::default() }
    }
}

/// How many of each a file would carry (or carries), for choosing what goes in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub hosts: usize,
    pub passwords: usize,
    pub keys: usize,
    pub proxies: usize,
    pub known_hosts: usize,
    pub groups: usize,
}

impl Counts {
    pub fn of(b: &Bundle) -> Self {
        let passwords = b.hosts.iter().filter(|h| h.password.is_some() || h.proxy_password.is_some()).count() + b.proxies.iter().filter(|p| p.password.is_some()).count();
        let mut groups: Vec<&str> = b.hosts.iter().map(|h| h.group.as_str()).filter(|g| !g.is_empty()).collect();
        groups.sort_unstable();
        groups.dedup();
        Counts { hosts: b.hosts.len(), passwords, keys: b.keys.len(), proxies: b.proxies.len(), known_hosts: b.known_hosts.len(), groups: groups.len() }
    }
}

/// The file's text, sealed with `password`.
pub fn seal(b: &Bundle, password: &str) -> Result<String> {
    if password.is_empty() {
        return Err(Error::InvalidId { reason: "password".into() });
    }
    let plain = serde_json::to_vec(b)?;
    let recipient = age::scrypt::Recipient::new(SecretString::from(password.to_owned()));
    let internal = |e: &dyn std::fmt::Display| Error::Internal { message: e.to_string() };
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient)).map_err(|e| internal(&e))?;
    let mut out = Vec::new();
    let armor = age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor).map_err(|e| internal(&e))?;
    let mut w = encryptor.wrap_output(armor).map_err(|e| internal(&e))?;
    w.write_all(&plain).map_err(|e| internal(&e))?;
    w.finish().and_then(|armor| armor.finish()).map_err(|e| internal(&e))?;
    String::from_utf8(out).map_err(|e| internal(&e))
}

/// Opens a file's text with its password.
pub fn open(text: &[u8], password: &str) -> Result<Bundle> {
    let not_ours = || Error::NotADetourFile;
    let decryptor = age::Decryptor::new(age::armor::ArmoredReader::new(text)).map_err(|_| not_ours())?;
    if !decryptor.is_scrypt() {
        return Err(not_ours());
    }
    let identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    let mut r = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity)).map_err(|_| Error::WrongFilePassword)?;
    let mut plain = Vec::new();
    r.read_to_end(&mut plain).map_err(|_| not_ours())?;
    let b: Bundle = serde_json::from_slice(&plain).map_err(|_| not_ours())?;
    if b.format != FORMAT {
        return Err(not_ours());
    }
    if b.version > VERSION {
        return Err(Error::NewerFile);
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Auth;

    fn host(id: &str) -> Host {
        Host {
            id: id.into(),
            group: "Web".into(),
            label: id.into(),
            address: "10.0.0.1".into(),
            port: 22,
            user: "root".into(),
            auth: Auth::Password,
            tags: vec![],
            last_connected: None,
            password: Some("hunter2".into()),
            proxy: None,
            proxy_password: None,
            proxy_id: None,
            forward_agent: false,
            order: None,
        }
    }

    #[test]
    fn a_sealed_file_opens_only_with_its_password() {
        let mut b = Bundle::new();
        b.hosts.push(host("web"));
        let text = seal(&b, "file pass").unwrap();
        assert!(text.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"), "{text}");
        assert!(!text.contains("hunter2"));
        let back = open(text.as_bytes(), "file pass").unwrap();
        assert_eq!(back.hosts[0].password.as_deref(), Some("hunter2"));
        assert!(matches!(open(text.as_bytes(), "other"), Err(Error::WrongFilePassword)));
        assert!(matches!(open(b"Host web\n  HostName 10.0.0.1\n", "file pass"), Err(Error::NotADetourFile)));
        assert!(matches!(seal(&b, ""), Err(Error::InvalidId { .. })));
    }

    #[test]
    fn a_file_from_a_newer_detour_is_refused() {
        let mut b = Bundle::new();
        b.version = VERSION + 1;
        let text = seal(&b, "p").unwrap();
        assert!(matches!(open(text.as_bytes(), "p"), Err(Error::NewerFile)));
    }
}
