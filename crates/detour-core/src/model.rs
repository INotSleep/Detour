//! Domain model shared by every frontend.
//!
//! Two shapes exist for records that hold secrets: the stored one (`Host`,
//! `KeyEntry`) and the view handed to a UI (`HostView`, `KeyView`), which never
//! contains passwords, passphrases or private keys. Field names are serialized
//! in camelCase to match `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

/// Unix time in milliseconds.
pub type Millis = i64;

pub fn now_ms() -> Millis {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as Millis).unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Auth {
    Key { key_id: String },
    Password,
    Agent,
}

/// How a host is reached when not directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Proxy {
    /// Through another host of the same vault (`ssh -J`).
    Jump { host_id: String },
    Socks5 {
        address: String,
        port: u16,
        /// Empty for none; the password is the host's `proxy_password`.
        #[serde(default)]
        user: String,
    },
    /// An HTTP proxy that tunnels with `CONNECT`.
    Http {
        address: String,
        port: u16,
        #[serde(default)]
        user: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub group: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub last_connected: Option<Millis>,
    /// Saved password for `Auth::Password`; `None` = ask every time.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub proxy: Option<Proxy>,
    /// Password for a SOCKS5 or HTTP proxy with a user.
    #[serde(default)]
    pub proxy_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub id: String,
    pub group: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub last_connected: Option<Millis>,
    #[serde(default)]
    pub has_password: bool,
    /// Incoming only: a password typed in the UI to save with this edit.
    #[serde(default, skip_serializing)]
    pub new_password: Option<String>,
    /// Team hosts: this person's own login, used instead of the shared one.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub own: Option<OwnLoginView>,
    #[serde(default)]
    pub proxy: Option<Proxy>,
    #[serde(default)]
    pub has_proxy_password: bool,
    /// Incoming only: a proxy password typed in the UI to save with this edit.
    #[serde(default, skip_serializing)]
    pub new_proxy_password: Option<String>,
}

/// A member's own login for a team host, kept in their personal vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnLogin {
    pub user: String,
    /// A key here refers to a personal key.
    pub auth: Auth,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnLoginView {
    pub user: String,
    pub auth: Auth,
    #[serde(default)]
    pub has_password: bool,
    /// Incoming only: a password typed in the UI to save.
    #[serde(default, skip_serializing)]
    pub new_password: Option<String>,
}

impl Proxy {
    /// The user a SOCKS5 or HTTP proxy is logged in to with, if any.
    pub fn user(&self) -> Option<&str> {
        match self {
            Proxy::Socks5 { user, .. } | Proxy::Http { user, .. } if !user.is_empty() => Some(user),
            _ => None,
        }
    }
}

impl From<&OwnLogin> for OwnLoginView {
    fn from(o: &OwnLogin) -> Self {
        Self { user: o.user.clone(), auth: o.auth.clone(), has_password: o.password.is_some(), new_password: None }
    }
}

impl From<&Host> for HostView {
    fn from(h: &Host) -> Self {
        Self {
            id: h.id.clone(),
            group: h.group.clone(),
            label: h.label.clone(),
            address: h.address.clone(),
            port: h.port,
            user: h.user.clone(),
            auth: h.auth.clone(),
            tags: h.tags.clone(),
            last_connected: h.last_connected,
            has_password: h.password.is_some(),
            new_password: None,
            own: None,
            proxy: h.proxy.clone(),
            has_proxy_password: h.proxy_password.is_some(),
            new_proxy_password: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyAlgorithm {
    Ed25519,
    Rsa,
    Ecdsa,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    pub id: String,
    pub name: String,
    pub algorithm: KeyAlgorithm,
    #[serde(default)]
    pub bits: Option<u32>,
    pub fingerprint: String,
    /// OpenSSH public key line.
    pub public_key: String,
    /// OpenSSH private key; encrypted with the passphrase when `has_passphrase`.
    /// Empty for a team key whose secret part this member can't read.
    #[serde(default)]
    pub private_key: String,
    pub has_passphrase: bool,
    /// Saved passphrase; `None` = ask when the key is used.
    #[serde(default)]
    pub passphrase: Option<String>,
    pub created_at: Millis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub id: String,
    pub name: String,
    pub algorithm: KeyAlgorithm,
    #[serde(default)]
    pub bits: Option<u32>,
    pub fingerprint: String,
    pub public_key: String,
    pub has_passphrase: bool,
    #[serde(default)]
    pub passphrase_saved: bool,
    pub created_at: Millis,
}

impl From<&KeyEntry> for KeyView {
    fn from(k: &KeyEntry) -> Self {
        Self {
            id: k.id.clone(),
            name: k.name.clone(),
            algorithm: k.algorithm,
            bits: k.bits,
            fingerprint: k.fingerprint.clone(),
            public_key: k.public_key.clone(),
            has_passphrase: k.has_passphrase,
            passphrase_saved: k.passphrase.is_some(),
            created_at: k.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TunnelKind {
    /// `ssh -L`
    Local,
    /// `ssh -R`
    Remote,
    /// `ssh -D` (SOCKS5)
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    pub kind: TunnelKind,
    pub bind_port: u16,
    pub via_host_id: String,
    #[serde(default)]
    pub target: Option<String>,
}

/// A pinned server key (trust on first use).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    /// `address:port`
    pub endpoint: String,
    /// OpenSSH public key line of the server.
    pub key: String,
    pub fingerprint: String,
    pub added_at: Millis,
}

/// Everything stored in one vault file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultData {
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
    #[serde(default)]
    pub known_hosts: Vec<KnownHost>,
    /// This person's team identity (Detour ID secret keys).
    #[serde(default)]
    pub member_identity: Option<crate::identity::StoredIdentity>,
    /// This installation's id in team logs.
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub teams: Vec<TeamRef>,
    /// Last login per team host (`<team id>/<host id>`), kept locally.
    #[serde(default)]
    pub team_last_connected: std::collections::BTreeMap<String, Millis>,
    /// Own logins for team hosts (`<team id>/<host id>`).
    #[serde(default)]
    pub team_own: std::collections::BTreeMap<String, OwnLogin>,
    /// Per team, the head of every device this person has read: storage can't roll them back.
    #[serde(default)]
    pub team_heads: std::collections::BTreeMap<String, std::collections::BTreeMap<String, crate::team::log::Head>>,
    /// Where this personal vault syncs with this person's other devices, if it does.
    #[serde(default)]
    pub sync: Option<TeamRef>,
    /// The synced records as of the last sync: what this device changed since is measured against it.
    #[serde(default)]
    pub sync_base: std::collections::BTreeMap<String, crate::personal::SyncRecord>,
    /// Teams on all of this person's devices, joined here or not.
    #[serde(default)]
    pub team_links: Vec<TeamLink>,
    /// This device's GitHub sign-in; never syncs (see `github`).
    #[serde(default)]
    pub github: Option<crate::github::GithubLogin>,
}

/// A team as this person's devices share it: where it is, not where it sits on one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamLink {
    pub team_id: String,
    pub name: String,
    /// Detour ID of the vault's creator, confirmed when it was first joined.
    pub root: String,
    /// The repository of a team in Git; `None` for a team in a folder (each device picks its own).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub token: Option<String>,
    /// The repository is reached with each device's own GitHub sign-in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub github: bool,
}

/// A team vault this person has joined.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamRef {
    pub id: String,
    pub path: String,
    /// Detour ID of the vault's creator, confirmed when joining.
    pub root: String,
    /// Set for a team kept in a Git repository; `path` is then its local copy.
    #[serde(default)]
    pub remote: Option<crate::team::git::GitRemote>,
}

/// What a UI sees of a vault: no secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    pub hosts: Vec<HostView>,
    pub keys: Vec<KeyView>,
    pub snippets: Vec<Snippet>,
    pub tunnels: Vec<Tunnel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_serializes_with_type_tag_and_camel_case_fields() {
        let json = serde_json::to_string(&Auth::Key { key_id: "k1".into() }).unwrap();
        assert_eq!(json, r#"{"type":"key","keyId":"k1"}"#);
        assert_eq!(serde_json::to_string(&Auth::Agent).unwrap(), r#"{"type":"agent"}"#);
    }

    #[test]
    fn host_view_hides_password() {
        let host = Host {
            id: "h".into(),
            group: "g".into(),
            label: "l".into(),
            address: "a".into(),
            port: 22,
            user: "u".into(),
            auth: Auth::Password,
            tags: vec![],
            last_connected: None,
            password: Some("secret".into()),
            proxy: None,
            proxy_password: None,
        };
        let json = serde_json::to_string(&HostView::from(&host)).unwrap();
        assert!(!json.contains("secret"));
        assert!(json.contains(r#""hasPassword":true"#));
    }
}
