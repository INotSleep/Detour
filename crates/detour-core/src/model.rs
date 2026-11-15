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

/// A group of hosts: its place among the others and the proxy its hosts go through.
/// Hosts name their group (`Host::group`); a group without a record is listed after those with one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub order: f64,
    /// A saved proxy of the same vault for its hosts that have none of their own.
    #[serde(default)]
    pub proxy_id: Option<String>,
}

/// A person's own grouping of a team's hosts ("Mine"), kept in their personal vault.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    #[serde(default)]
    pub groups: Vec<MyGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyGroup {
    pub id: String,
    pub name: String,
    /// Host ids, in order.
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// A saved proxy of some vault: this person's own or a team's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRef {
    pub vault: String,
    pub id: String,
}

/// This person's own settings for a group (`<vault>/team:<name>` or `<vault>/mine:<id>`),
/// used before the host's: the way to its hosts and the login to them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPrefs {
    #[serde(default)]
    pub proxy: Option<ProxyRef>,
    #[serde(default)]
    pub user: Option<String>,
    /// A key here is a personal key.
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPrefsView {
    pub key: String,
    #[serde(default)]
    pub proxy: Option<ProxyRef>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub has_password: bool,
    /// Incoming only: a password typed in; empty removes the saved one.
    #[serde(default, skip_serializing)]
    pub new_password: Option<String>,
}

impl GroupPrefs {
    pub fn view(&self, key: &str) -> GroupPrefsView {
        GroupPrefsView { key: key.to_string(), proxy: self.proxy.clone(), user: self.user.clone(), auth: self.auth.clone(), has_password: self.password.is_some(), new_password: None }
    }
    pub fn is_empty(&self) -> bool {
        self.proxy.is_none() && self.user.is_none() && self.auth.is_none()
    }
}

/// A saved proxy that hosts of the same vault reach their server through.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyEntry {
    pub id: String,
    pub name: String,
    pub kind: ProxyKind,
    /// SOCKS5 and HTTP: the password of `user`.
    #[serde(default)]
    pub password: Option<String>,
    /// WireGuard: this side's private key, base64.
    #[serde(default)]
    pub private_key: Option<String>,
    /// WireGuard: the peer's preshared key, base64.
    #[serde(default)]
    pub preshared_key: Option<String>,
    /// VLESS: the user id.
    #[serde(default)]
    pub uuid: Option<String>,
}

/// The secret fields of a proxy record.
pub const PROXY_SECRETS: [&str; 4] = ["password", "privateKey", "presharedKey", "uuid"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProxyKind {
    Socks5 {
        address: String,
        port: u16,
        /// Empty for none.
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
    /// A WireGuard peer; only Detour's own connections go through it.
    Wireguard {
        /// The peer's `host:port`.
        endpoint: String,
        /// The peer's public key, base64.
        public_key: String,
        /// This side's addresses in the tunnel (`10.0.0.2/32`).
        #[serde(default)]
        addresses: Vec<String>,
        /// Name servers reached through the tunnel.
        #[serde(default)]
        dns: Vec<String>,
        #[serde(default)]
        mtu: Option<u16>,
        /// Kept for export: only connections to hosts go through the tunnel.
        #[serde(default)]
        allowed_ips: Vec<String>,
        #[serde(default)]
        keepalive: Option<u16>,
    },
    Vless {
        address: String,
        port: u16,
        #[serde(default)]
        security: VlessSecurity,
        /// The name sent in TLS (SNI); the address when empty.
        #[serde(default)]
        sni: String,
        #[serde(default)]
        alpn: Vec<String>,
        /// REALITY: the server's public key (`pbk`), base64url.
        #[serde(default)]
        public_key: String,
        /// REALITY: the short id (`sid`), hex.
        #[serde(default)]
        short_id: String,
        /// TLS: SHA-256 of the server's certificate to accept instead of a trusted one (`pcs`).
        #[serde(default)]
        pinned_cert: String,
        #[serde(default)]
        transport: VlessTransport,
        /// `xtls-rprx-vision` or empty.
        #[serde(default)]
        flow: String,
        /// VLESS Encryption (`mlkem768x25519plus...` with the server's keys); empty for none.
        #[serde(default)]
        encryption: String,
        /// XHTTP: the path, `Host` and mode (`auto`, `packet-up`, `stream-up`, `stream-one`).
        #[serde(default)]
        path: String,
        #[serde(default)]
        host: String,
        #[serde(default)]
        mode: String,
        /// XHTTP: further settings, the JSON of a link's `extra`.
        #[serde(default)]
        extra: String,
        /// Link parameters not used here, kept for the link this proxy is copied as.
        #[serde(default)]
        params: std::collections::BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VlessSecurity {
    #[default]
    None,
    Tls,
    Reality,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VlessTransport {
    #[default]
    Tcp,
    Xhttp,
}

impl ProxyKind {
    /// The secret fields this kind uses.
    pub fn secrets(&self) -> &'static [&'static str] {
        match self {
            ProxyKind::Socks5 { user, .. } | ProxyKind::Http { user, .. } if !user.is_empty() => &["password"],
            ProxyKind::Socks5 { .. } | ProxyKind::Http { .. } => &[],
            ProxyKind::Wireguard { .. } => &["privateKey", "presharedKey"],
            ProxyKind::Vless { .. } => &["uuid"],
        }
    }
}

impl ProxyEntry {
    pub fn secret(&self, field: &str) -> Option<&String> {
        match field {
            "password" => self.password.as_ref(),
            "privateKey" => self.private_key.as_ref(),
            "presharedKey" => self.preshared_key.as_ref(),
            "uuid" => self.uuid.as_ref(),
            _ => None,
        }
    }

    fn secret_mut(&mut self, field: &str) -> Option<&mut Option<String>> {
        match field {
            "password" => Some(&mut self.password),
            "privateKey" => Some(&mut self.private_key),
            "presharedKey" => Some(&mut self.preshared_key),
            "uuid" => Some(&mut self.uuid),
            _ => None,
        }
    }

    /// A proxy as a UI edited it, with the secrets it typed in or else those kept from `old`.
    /// Secrets the kind doesn't use are dropped; a typed empty one is removed.
    pub fn from_view(v: ProxyView, old: Option<&ProxyEntry>) -> ProxyEntry {
        let mut p = ProxyEntry { id: v.id, name: v.name, kind: v.kind, password: None, private_key: None, preshared_key: None, uuid: None };
        for field in p.kind.secrets() {
            let value = match v.new_secrets.get(*field) {
                Some(typed) => Some(typed.clone()).filter(|s| !s.is_empty()),
                None => old.and_then(|o| o.secret(field).cloned()),
            };
            if let Some(slot) = p.secret_mut(field) {
                *slot = value;
            }
        }
        p
    }
}

/// What a UI sees of a proxy: no secrets, only which are saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyView {
    pub id: String,
    pub name: String,
    pub kind: ProxyKind,
    /// The secret fields that are saved.
    #[serde(default)]
    pub saved: Vec<String>,
    /// Incoming only: secrets typed in the UI, by field; empty removes one.
    #[serde(default, skip_serializing)]
    pub new_secrets: std::collections::BTreeMap<String, String>,
}

impl From<&ProxyEntry> for ProxyView {
    fn from(p: &ProxyEntry) -> Self {
        let saved = PROXY_SECRETS.iter().filter(|f| p.secret(f).is_some()).map(|f| f.to_string()).collect();
        Self { id: p.id.clone(), name: p.name.clone(), kind: p.kind.clone(), saved, new_secrets: Default::default() }
    }
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
    /// A saved proxy of the same vault; used before `proxy`, which older versions still read.
    #[serde(default)]
    pub proxy_id: Option<String>,
    /// Lets the server use this person's keys while connected (`ssh -A`).
    #[serde(default)]
    pub forward_agent: bool,
    /// Its place in its group; hosts without one come after, as they are.
    #[serde(default)]
    pub order: Option<f64>,
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
    #[serde(default)]
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub forward_agent: bool,
    #[serde(default)]
    pub order: Option<f64>,
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
    /// Used to log in; off, it is kept while the shared login is used instead.
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnLoginView {
    pub user: String,
    pub auth: Auth,
    #[serde(default)]
    pub has_password: bool,
    #[serde(default = "yes")]
    pub active: bool,
    /// Incoming only: a password typed in the UI to save.
    #[serde(default, skip_serializing)]
    pub new_password: Option<String>,
}

impl Proxy {
    /// A SOCKS5 or HTTP proxy written into a host (before saved proxies) as one.
    pub fn as_kind(&self) -> Option<ProxyKind> {
        match self.clone() {
            Proxy::Socks5 { address, port, user } => Some(ProxyKind::Socks5 { address, port, user }),
            Proxy::Http { address, port, user } => Some(ProxyKind::Http { address, port, user }),
            Proxy::Jump { .. } => None,
        }
    }

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
        Self { user: o.user.clone(), auth: o.auth.clone(), has_password: o.password.is_some(), active: o.active, new_password: None }
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
            proxy_id: h.proxy_id.clone(),
            forward_agent: h.forward_agent,
            order: h.order,
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
    pub proxies: Vec<ProxyEntry>,
    #[serde(default)]
    pub groups: Vec<Group>,
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
    /// This person's own grouping of each team's hosts, by team id.
    #[serde(default)]
    pub layouts: std::collections::BTreeMap<String, Layout>,
    /// This person's own settings of groups, by `<vault>/team:<name>` or `<vault>/mine:<id>`.
    #[serde(default)]
    pub group_prefs: std::collections::BTreeMap<String, GroupPrefs>,
    /// Team hosts this person forwards ssh-agent to (`<team id>/<host id>`). Never shared:
    /// whoever edits a team host must not be able to reach its members' keys.
    #[serde(default)]
    pub team_forward_agent: std::collections::BTreeSet<String>,
    /// Per team, the head of every device this person has read: storage can't roll them back.
    #[serde(default)]
    pub team_heads: std::collections::BTreeMap<String, std::collections::BTreeMap<String, crate::team::log::Head>>,
    /// Where this personal vault syncs with this person's other devices, if it does.
    #[serde(default)]
    pub sync: Option<TeamRef>,
    /// The synced records as of the last sync: what this device changed since is measured against it.
    #[serde(default)]
    pub sync_base: std::collections::BTreeMap<String, crate::personal::SyncRecord>,
    /// Synced records this version can't read (written by a newer one): kept as they are,
    /// so they aren't taken for deleted here.
    #[serde(default)]
    pub sync_unread: std::collections::BTreeMap<String, crate::personal::SyncRecord>,
    /// Teams on all of this person's devices, joined here or not.
    #[serde(default)]
    pub team_links: Vec<TeamLink>,
    /// This device's GitHub sign-in; never syncs (see `github`).
    #[serde(default)]
    pub github: Option<crate::github::GithubLogin>,
    /// AI agents' tokens on this device; never syncs (see `tokens`).
    #[serde(default)]
    pub agents: Vec<crate::tokens::AgentToken>,
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
    #[serde(default)]
    pub proxies: Vec<ProxyView>,
    #[serde(default)]
    pub groups: Vec<Group>,
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
            proxy_id: None,
            forward_agent: false, order: None,
        };
        let json = serde_json::to_string(&HostView::from(&host)).unwrap();
        assert!(!json.contains("secret"));
        assert!(json.contains(r#""hasPassword":true"#));
    }
}
