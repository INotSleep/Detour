//! The encrypted personal vault on disk.
//!
//! Layout of the data directory:
//! - `identity.age` — this device's age X25519 identity, encrypted with the
//!   master password (age scrypt recipient).
//! - `vaults/personal.age` — the personal vault as JSON, encrypted to that identity.
//!
//! Team vaults will be encrypted to every member's identity the same way.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use age::secrecy::{ExposeSecret, SecretString};

use crate::identity::{Identity, PublicIdentity};
use crate::model::*;
use crate::personal::{self, SyncRecord};
use crate::team::log::Head;
use crate::team::state::{Coll, Level, Role};
use crate::sync::Place;
use crate::team::git::{GitRemote, HttpRemote, HttpUrl, Mirror, Remote, SshUrl};
use crate::team::{AccessChange, ProblemView, RemoteView, TeamCache, TeamVault, TeamView, VaultInfo};
use crate::{Error, Result, SecretKind, keys, ssh};

/// The personal vault's id in every vault-addressed call.
pub const PERSONAL: &str = "personal";

const IDENTITY_FILE: &str = "identity.age";
const PERSONAL_FILE: &str = "vaults/personal.age";

struct Unlocked {
    identity: age::x25519::Identity,
    vault: VaultData,
}

pub struct Store {
    dir: PathBuf,
    state: Option<Unlocked>,
    /// Records deleted during this session, so a UI "undo" can bring their secrets back.
    trash_keys: HashMap<String, KeyEntry>,
    trash_passwords: HashMap<String, String>,
    trash_proxies: HashMap<String, ProxyEntry>,
    teams: BTreeMap<String, TeamVault>,
    /// Teams that could not be opened (folder unavailable, ...), with the reason as error JSON.
    team_errors: BTreeMap<String, String>,
    /// Git teams: the last failure reaching the repository (a serialized `Error`) and the last success.
    git_status: BTreeMap<String, (Option<serde_json::Value>, Option<Millis>)>,
    /// The synced copy of the personal vault, when sync is on and its storage could be opened.
    personal: Option<TeamVault>,
    /// Teams from other devices that could not be joined here, with why (a serialized `Error`).
    link_errors: BTreeMap<String, serde_json::Value>,
    /// Entries read per vault when its cache was last written.
    cached: HashMap<String, usize>,
    /// How syncing the personal vault last went: failure (a serialized `Error`) and last success.
    sync_state: (Option<serde_json::Value>, Option<Millis>),
}

/// Where the personal vault syncs and how that goes, for the UI.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    /// The folder, the repository's URL, or the drive's folder.
    pub location: Option<String>,
    pub git: bool,
    /// `folder`, `git`, `onedrive`, `gdrive` or `dropbox`.
    pub storage: Option<String>,
    /// The drive folder, when the vault syncs through a cloud drive (another device of this
    /// person's joins through it: the phone's QR carries it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drive: Option<crate::drive::DriveRemote>,
    /// Changes the repository doesn't have yet.
    pub pending: bool,
    /// Why the last sync failed (a serialized `Error`).
    pub error: Option<serde_json::Value>,
    pub synced_at: Option<Millis>,
    pub problems: Vec<ProblemView>,
}

/// A joined team as the UI lists it: its contents when available, otherwise why not.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamEntry {
    pub id: String,
    pub path: String,
    pub view: Option<TeamView>,
    pub error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyId {
    pub detour_id: String,
    pub fingerprint: String,
}

/// What a UI may change through `Store::put`. Secrets are never part of it.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultUpdate {
    pub hosts: Vec<HostView>,
    /// Existing keys only (rename, reorder, delete). New keys come from generate/import.
    pub keys: Vec<KeyView>,
    pub snippets: Vec<Snippet>,
    pub tunnels: Vec<Tunnel>,
    /// Absent from older clients: then the proxies stay as they are.
    #[serde(default)]
    pub proxies: Option<Vec<ProxyView>>,
    /// The same for groups.
    #[serde(default)]
    pub groups: Option<Vec<Group>>,
}

/// Makes the SOCKS5 and HTTP proxies written into hosts saved proxies the hosts refer to.
/// The hosts keep them written in too, for older versions on other devices.
/// Returns whether anything changed.
pub fn migrate_proxies(v: &mut VaultData) -> bool {
    let mut changed = false;
    for i in 0..v.hosts.len() {
        let h = &v.hosts[i];
        let Some(kind) = h.proxy.as_ref().and_then(Proxy::as_kind).filter(|_| h.proxy_id.is_none()) else { continue };
        let password = h.proxy_password.clone();
        let found = v.proxies.iter().find(|p| p.kind == kind && p.password == password).map(|p| p.id.clone());
        let id = found.unwrap_or_else(|| {
            let name = match &kind {
                ProxyKind::Socks5 { address, port, .. } | ProxyKind::Http { address, port, .. } => format!("{address}:{port}"),
                _ => String::new(),
            };
            let id = uuid::Uuid::new_v4().to_string();
            let password = if kind.secrets().is_empty() { None } else { password };
            v.proxies.push(ProxyEntry { id: id.clone(), name, kind, password, private_key: None, preshared_key: None, uuid: None });
            id
        });
        v.hosts[i].proxy_id = Some(id);
        changed = true;
    }
    changed
}

impl Store {
    /// Where Detour keeps its data on this OS.
    pub fn default_dir() -> PathBuf {
        directories::ProjectDirs::from("dev", "detour", "Detour")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".detour"))
    }

    pub fn open(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
            state: None,
            trash_keys: HashMap::new(),
            trash_passwords: HashMap::new(),
            trash_proxies: HashMap::new(),
            teams: BTreeMap::new(),
            team_errors: BTreeMap::new(),
            git_status: BTreeMap::new(),
            personal: None,
            sync_state: (None, None),
            link_errors: BTreeMap::new(),
            cached: HashMap::new(),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.dir.join(IDENTITY_FILE).exists()
    }

    pub fn is_unlocked(&self) -> bool {
        self.state.is_some()
    }

    /// First run: creates the device identity protected by `master_password`.
    pub fn create(&mut self, master_password: &str) -> Result<()> {
        if self.is_initialized() {
            return Err(Error::AlreadyInitialized);
        }
        let identity = age::x25519::Identity::generate();
        self.write_identity(&identity, master_password)?;
        self.state = Some(Unlocked { identity, vault: VaultData::default() });
        self.ensure_member()?;
        self.save()
    }

    pub fn unlock(&mut self, master_password: &str) -> Result<()> {
        if !self.is_initialized() {
            return Err(Error::NotInitialized);
        }
        let sealed = std::fs::read(self.dir.join(IDENTITY_FILE))?;
        let scrypt = age::scrypt::Identity::new(SecretString::from(master_password.to_owned()));
        let plain = age::decrypt(&scrypt, &sealed).map_err(|_| Error::WrongMasterPassword)?;
        let text = String::from_utf8(plain).map_err(|_| Error::WrongMasterPassword)?;
        let identity = age::x25519::Identity::from_str(text.trim()).map_err(|e| Error::Internal { message: e.to_string() })?;

        let vault_path = self.dir.join(PERSONAL_FILE);
        let vault = if vault_path.exists() {
            let plain = age::decrypt(&identity, &std::fs::read(vault_path)?).map_err(|e| Error::Internal { message: format!("personal vault: {e}") })?;
            serde_json::from_slice(&plain)?
        } else {
            VaultData::default()
        };
        self.state = Some(Unlocked { identity, vault });
        let migrated = migrate_proxies(self.vault_mut()?);
        if self.ensure_member()? || migrated {
            self.save()?;
        }
        self.open_teams();
        self.open_sync();
        Ok(())
    }

    /// Forgets every decrypted secret held in memory.
    pub fn lock(&mut self) {
        self.state = None;
        self.trash_keys.clear();
        self.trash_passwords.clear();
        self.trash_proxies.clear();
        self.teams.clear();
        self.team_errors.clear();
        self.git_status.clear();
        self.personal = None;
        self.sync_state = (None, None);
        ssh::close_pooled();
    }

    /// Is this the master password? (A client proves it is the person without unlocking again.)
    pub fn check_master_password(&self, master_password: &str) -> Result<()> {
        let sealed = std::fs::read(self.dir.join(IDENTITY_FILE))?;
        let scrypt = age::scrypt::Identity::new(SecretString::from(master_password.to_owned()));
        age::decrypt(&scrypt, &sealed).map(|_| ()).map_err(|_| Error::WrongMasterPassword)
    }

    pub fn change_master_password(&mut self, current: &str, new: &str) -> Result<()> {
        let identity = {
            let sealed = std::fs::read(self.dir.join(IDENTITY_FILE))?;
            let scrypt = age::scrypt::Identity::new(SecretString::from(current.to_owned()));
            let plain = age::decrypt(&scrypt, &sealed).map_err(|_| Error::WrongMasterPassword)?;
            age::x25519::Identity::from_str(String::from_utf8_lossy(&plain).trim()).map_err(|e| Error::Internal { message: e.to_string() })?
        };
        self.write_identity(&identity, new)
    }

    fn write_identity(&self, identity: &age::x25519::Identity, master_password: &str) -> Result<()> {
        let recipient = age::scrypt::Recipient::new(SecretString::from(master_password.to_owned()));
        let sealed = age::encrypt(&recipient, identity.to_string().expose_secret().as_bytes()).map_err(|e| Error::Internal { message: e.to_string() })?;
        write_atomic(&self.dir.join(IDENTITY_FILE), &sealed)
    }

    fn unlocked(&self) -> Result<&Unlocked> {
        self.state.as_ref().ok_or(Error::Locked)
    }

    pub(crate) fn vault(&self) -> Result<&VaultData> {
        Ok(&self.unlocked()?.vault)
    }

    pub(crate) fn vault_mut(&mut self) -> Result<&mut VaultData> {
        Ok(&mut self.state.as_mut().ok_or(Error::Locked)?.vault)
    }

    /// Writes the vault file; with sync on, what changed goes to the synced copy first.
    pub(crate) fn save(&mut self) -> Result<()> {
        self.refresh_team_links();
        if self.personal.is_some() && self.has_unsynced() {
            let merged = self.merge_personal().map(|_| ());
            // A repository is reached later, in the background; a folder is reached now.
            if merged.is_err() || !self.sync_is_git() {
                self.note_sync(&merged);
            }
        }
        self.write_vault()
    }

    fn write_vault(&self) -> Result<()> {
        let s = self.unlocked()?;
        let json = serde_json::to_vec(&s.vault)?;
        let sealed = age::encrypt(&s.identity.to_public(), &json).map_err(|e| Error::Internal { message: e.to_string() })?;
        write_atomic(&self.dir.join(PERSONAL_FILE), &sealed)
    }

    pub fn view(&self) -> Result<VaultView> {
        let v = &self.unlocked()?.vault;
        Ok(VaultView {
            hosts: v.hosts.iter().map(HostView::from).collect(),
            keys: v.keys.iter().map(KeyView::from).collect(),
            snippets: v.snippets.clone(),
            tunnels: v.tunnels.clone(),
            proxies: v.proxies.iter().map(ProxyView::from).collect(),
            groups: v.groups.clone(),
        })
    }

    /// Applies a UI's edit of the vault while keeping every secret it can't see.
    pub fn put(&mut self, update: VaultUpdate) -> Result<VaultView> {
        // A key a repository is reached with stays: without it this device can't sync to get it back.
        let v = self.vault()?;
        if repo_keys(v).iter().any(|id| v.keys.iter().any(|k| &k.id == id) && !update.keys.iter().any(|k| &k.id == id)) {
            return Err(Error::KeyInUse);
        }
        let mut trash_keys = std::mem::take(&mut self.trash_keys);
        let mut trash_passwords = std::mem::take(&mut self.trash_passwords);
        let mut trash_proxies = std::mem::take(&mut self.trash_proxies);
        let vault = self.vault_mut()?;

        let mut old_hosts: HashMap<String, Host> = vault.hosts.drain(..).map(|h| (h.id.clone(), h)).collect();
        vault.hosts = update
            .hosts
            .into_iter()
            .map(|v| {
                let old = old_hosts.remove(&v.id);
                // Login time is recorded by the core, never edited by a UI.
                let last_connected = old.as_ref().and_then(|h| h.last_connected);
                let kept_proxy = v.new_proxy_password.filter(|p| !p.is_empty()).or_else(|| old.as_ref().and_then(|h| h.proxy_password.clone()));
                let kept = v.new_password.filter(|p| !p.is_empty()).or_else(|| old.and_then(|h| h.password)).or_else(|| trash_passwords.remove(&v.id));
                Host {
                    password: if v.auth == Auth::Password { kept } else { None },
                    proxy_password: if v.proxy.as_ref().and_then(Proxy::user).is_some() { kept_proxy } else { None },
                    proxy: v.proxy,
                    proxy_id: v.proxy_id,
                    order: v.order,
                    forward_agent: v.forward_agent,
                    last_connected,
                    id: v.id,
                    group: v.group,
                    label: v.label,
                    address: v.address,
                    port: v.port,
                    user: v.user,
                    auth: v.auth,
                    tags: v.tags,
                }
            })
            .collect();
        for (id, h) in old_hosts {
            if let Some(p) = h.password {
                trash_passwords.insert(id, p);
            }
        }

        let mut old_keys: HashMap<String, KeyEntry> = vault.keys.drain(..).map(|k| (k.id.clone(), k)).collect();
        vault.keys = update
            .keys
            .into_iter()
            .filter_map(|v| {
                let mut k = old_keys.remove(&v.id).or_else(|| trash_keys.remove(&v.id))?;
                k.name = v.name;
                Some(k)
            })
            .collect();
        trash_keys.extend(old_keys);

        vault.snippets = update.snippets;
        vault.tunnels = update.tunnels;
        if let Some(groups) = update.groups {
            vault.groups = groups;
        }
        if let Some(proxies) = update.proxies {
            let mut old: HashMap<String, ProxyEntry> = std::mem::take(&mut trash_proxies);
            old.extend(vault.proxies.drain(..).map(|p| (p.id.clone(), p)));
            vault.proxies = proxies.into_iter().map(|v| ProxyEntry::from_view(v.clone(), old.get(&v.id))).collect();
            // Kept so that undoing a delete brings the secrets back.
            trash_proxies = old.into_iter().filter(|(id, _)| !vault.proxies.iter().any(|n| &n.id == id)).collect();
        }

        self.trash_keys = trash_keys;
        self.trash_passwords = trash_passwords;
        self.trash_proxies = trash_proxies;
        self.save()?;
        self.view()
    }

    /// A client's edit of the vault as it saw it (`base`): only what it changed is applied.
    pub fn put_from(&mut self, base: &VaultUpdate, next: VaultUpdate) -> Result<VaultView> {
        let current = self.view()?.into();
        self.put(crate::merge::merge(base, next, current))
    }

    pub fn set_host_password(&mut self, host_id: &str, password: Option<String>) -> Result<()> {
        let host = self.vault_mut()?.hosts.iter_mut().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?;
        host.password = password.filter(|p| !p.is_empty());
        self.save()
    }

    pub fn generate_key(&mut self, name: &str, algorithm: KeyAlgorithm, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        let entry = keys::generate(name, algorithm, passphrase, remember)?;
        self.add_key(entry)
    }

    pub fn import_key(&mut self, name: &str, text: &str, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        let entry = keys::import(name, text, passphrase, remember)?;
        self.add_key(entry)
    }

    fn add_key(&mut self, entry: KeyEntry) -> Result<KeyView> {
        let view = KeyView::from(&entry);
        self.vault_mut()?.keys.push(entry);
        self.save()?;
        Ok(view)
    }

    fn key(&self, key_id: &str) -> Result<&KeyEntry> {
        self.unlocked()?.vault.keys.iter().find(|k| k.id == key_id).ok_or_else(|| not_found("key"))
    }

    pub fn export_key(&self, key_id: &str, file_passphrase: Option<&str>) -> Result<String> {
        keys::export(self.key(key_id)?, file_passphrase)
    }

    pub fn set_key_passphrase(&mut self, key_id: &str, current: Option<&str>, new: Option<&str>, remember: bool) -> Result<KeyView> {
        let updated = keys::change_passphrase(self.key(key_id)?, current, new, remember)?;
        let view = KeyView::from(&updated);
        let slot = self.vault_mut()?.keys.iter_mut().find(|k| k.id == key_id).ok_or_else(|| not_found("key"))?;
        *slot = updated;
        self.save()?;
        Ok(view)
    }

    /// Everything a session needs to connect to a host, with secrets resolved.
    pub fn connect_request(&self, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<ssh::ConnectRequest> {
        self.connect_request_in(PERSONAL, host_id, secret, trust_fingerprint)
    }

    /// A personal host's login, without the way to it.
    fn personal_login(&self, host_id: &str) -> Result<(Host, Option<KeyEntry>)> {
        let v = &self.unlocked()?.vault;
        let host = v.hosts.iter().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?.clone();
        let key = match &host.auth {
            Auth::Key { key_id } => Some(v.keys.iter().find(|k| &k.id == key_id).ok_or_else(|| not_found("key"))?.clone()),
            _ => None,
        };
        Ok((host, key))
    }

    /// Records a successful login: pins newly trusted host keys and the time.
    pub fn record_connection(&mut self, host_id: &str, new_keys: ssh::NewKeys) -> Result<()> {
        let vault = self.vault_mut()?;
        for k in new_keys {
            vault.known_hosts.retain(|x| x.endpoint != k.endpoint);
            vault.known_hosts.push(k);
        }
        if let Some(h) = vault.hosts.iter_mut().find(|h| h.id == host_id) {
            h.last_connected = Some(now_ms());
        }
        self.save()
    }

    /// Saves a secret the user typed in during a login.
    pub fn save_secret(&mut self, host_id: &str, kind: SecretKind, value: String) -> Result<()> {
        let vault = self.vault_mut()?;
        let host = vault.hosts.iter().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?.clone();
        match (kind, &host.auth) {
            (SecretKind::Password, _) => {
                if let Some(h) = vault.hosts.iter_mut().find(|h| h.id == host_id) {
                    h.password = Some(value);
                }
            }
            (SecretKind::Passphrase, Auth::Key { key_id }) => {
                let k = vault.keys.iter_mut().find(|k| &k.id == key_id).ok_or_else(|| not_found("key"))?;
                k.passphrase = Some(value);
            }
            (SecretKind::Passphrase, _) => return Err(not_found("key")),
        }
        self.save()
    }

    /// Forgets a pinned host key, e.g. after the server was reinstalled.
    pub fn forget_host_key(&mut self, host_id: &str) -> Result<()> {
        let vault = self.vault_mut()?;
        let host = vault.hosts.iter().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?;
        let ep = endpoint(host);
        vault.known_hosts.retain(|k| k.endpoint != ep);
        self.save()
    }
}

/* ---------- teams ---------- */

impl Store {
    /// Gives this person a team identity and this installation a device id. Returns true if anything was created.
    fn ensure_member(&mut self) -> Result<bool> {
        let v = self.vault_mut()?;
        let mut changed = false;
        if v.member_identity.is_none() {
            v.member_identity = Some(Identity::generate().to_stored());
            changed = true;
        }
        if v.device_id.is_none() {
            v.device_id = Some(crate::new_id());
            changed = true;
        }
        Ok(changed)
    }

    fn member(&self) -> Result<(Identity, String)> {
        let v = &self.unlocked()?.vault;
        let id = Identity::from_stored(v.member_identity.as_ref().ok_or(Error::Locked)?)?;
        Ok((id, v.device_id.clone().ok_or(Error::Locked)?))
    }

    fn known_heads(&self, team: &str) -> BTreeMap<String, Head> {
        self.state.as_ref().and_then(|s| s.vault.team_heads.get(team).cloned()).unwrap_or_default()
    }

    /* ---------- what this device keeps to open vaults fast ---------- */

    fn cache_path(&self, id: &str) -> PathBuf {
        self.dir.join("cache").join(format!("{id}.age"))
    }

    /// A vault's cache, sealed to this device's key; empty if there is none or it can't be read.
    fn load_cache(&self, id: &str) -> TeamCache {
        let (Ok(s), Ok(sealed)) = (self.unlocked(), std::fs::read(self.cache_path(id))) else { return TeamCache::default() };
        age::decrypt(&s.identity, &sealed).ok().and_then(|plain| serde_json::from_slice(&plain).ok()).unwrap_or_default()
    }

    /// Writes a vault's cache when it read more entries since the last write.
    fn save_cache(&mut self, id: &str, t: &TeamVault) {
        let read = t.entries_read();
        if self.cached.get(id) == Some(&read) {
            return;
        }
        let Ok(s) = self.unlocked() else { return };
        let Ok(plain) = serde_json::to_vec(&t.cache()) else { return };
        if let Ok(sealed) = age::encrypt(&s.identity.to_public(), &plain)
            && write_atomic(&self.cache_path(id), &sealed).is_ok()
        {
            self.cached.insert(id.to_string(), read);
        }
    }

    /// Signatures a team checked since it was opened (for tests of the cache).
    #[doc(hidden)]
    pub fn team_signatures_checked(&self, id: &str) -> usize {
        self.teams.get(id).map_or(0, |t| t.signatures_checked())
    }

    fn drop_cache(&mut self, id: &str) {
        self.cached.remove(id);
        let _ = std::fs::remove_file(self.cache_path(id));
    }

    fn open_teams(&mut self) {
        let Ok((me, device)) = self.member() else { return };
        let refs = self.state.as_ref().map(|s| s.vault.teams.clone()).unwrap_or_default();
        for r in refs {
            match TeamVault::open_cached(&r.path, &r.root, &me, &device, self.known_heads(&r.id), self.load_cache(&r.id)) {
                Ok(t) => {
                    self.team_errors.remove(&r.id);
                    self.save_cache(&r.id, &t);
                    self.teams.insert(r.id.clone(), t);
                }
                Err(e) => {
                    self.team_errors.insert(r.id.clone(), serde_json::to_string(&e).unwrap_or_default());
                }
            }
        }
    }

    pub fn my_id(&self) -> Result<MyId> {
        let p = self.member()?.0.public();
        Ok(MyId { detour_id: p.detour_id(), fingerprint: p.fingerprint() })
    }

    /// Fingerprint of someone else's Detour ID, to compare out of band.
    pub fn inspect_id(detour_id: &str) -> Result<String> {
        Ok(PublicIdentity::parse(detour_id)?.fingerprint())
    }

    /// A fresh recovery key for a team about to be created.
    pub fn new_recovery_key() -> String {
        Identity::generate().to_recovery_key()
    }

    pub(crate) fn team(&self, id: &str) -> Result<&TeamVault> {
        self.teams.get(id).ok_or_else(|| self.team_unavailable(id))
    }

    fn team_mut(&mut self, id: &str) -> Result<&mut TeamVault> {
        let err = self.team_unavailable(id);
        self.teams.get_mut(id).ok_or(err)
    }

    fn team_unavailable(&self, id: &str) -> Error {
        match self.state.as_ref().and_then(|s| s.vault.teams.iter().find(|t| t.id == id)) {
            Some(r) => Error::Unavailable { path: r.path.clone() },
            None => not_found("team"),
        }
    }

    /// Team view with this person's last logins, own logins and repository status filled in.
    pub fn team_view(&self, id: &str) -> Result<TeamView> {
        let mut view = self.team(id)?.view();
        let v = &self.unlocked()?.vault;
        for h in &mut view.data.hosts {
            let key = format!("{id}/{}", h.id);
            h.last_connected = v.team_last_connected.get(&key).copied();
            h.own = v.team_own.get(&key).map(OwnLoginView::from);
            h.forward_agent = v.team_forward_agent.contains(&key);
        }
        view.layout = v.layouts.get(id).cloned();
        if let Some(r) = v.teams.iter().find(|t| t.id == id)
            && r.mirrored()
        {
            let (error, synced_at) = self.git_status.get(id).cloned().unwrap_or_default();
            view.remote = Some(RemoteView { url: r.location(), kind: r.storage().into(), pending: pending_of(r), error, synced_at });
        }
        Ok(view)
    }

    /// Remembers what was read from a team, then returns its view.
    fn after_team(&mut self, id: &str) -> Result<TeamView> {
        if let Some(t) = self.teams.remove(id) {
            self.save_cache(id, &t);
            self.teams.insert(id.to_string(), t);
        }
        let heads = self.team(id)?.known_heads().clone();
        let v = self.vault_mut()?;
        if v.team_heads.get(id) != Some(&heads) {
            v.team_heads.insert(id.to_string(), heads);
            self.save()?;
        }
        self.team_view(id)
    }

    pub fn teams(&self) -> Vec<TeamEntry> {
        let Ok(s) = self.unlocked() else { return vec![] };
        s.vault
            .teams
            .iter()
            .map(|r| TeamEntry { id: r.id.clone(), path: r.path.clone(), view: self.team_view(&r.id).ok(), error: self.team_errors.get(&r.id).cloned() })
            .collect()
    }

    pub fn create_team(&mut self, path: &str, name: &str, my_name: &str, recovery_key: &str) -> Result<TeamView> {
        let recovery = Identity::from_recovery_key(recovery_key)?;
        let (me, device) = self.member()?;
        let t = TeamVault::create(path, name, &me, my_name, &device, &recovery)?;
        self.add_team(t, None)
    }

    pub fn inspect_team(path: &str) -> Result<VaultInfo> {
        TeamVault::inspect(path)
    }

    /// Joins a vault after the user confirmed its creator (`root`, a Detour ID).
    pub fn join_team(&mut self, path: &str, root: &str) -> Result<TeamView> {
        let (me, device) = self.member()?;
        let id = TeamVault::inspect(path)?.id;
        if self.teams.contains_key(&id) {
            return Err(Error::AlreadyMember);
        }
        let t = TeamVault::open(path, root, &me, &device, self.known_heads(&id))?;
        self.add_team(t, None)
    }

    /// Uses a recovery key to make this person an owner of the vault at `path` again, then joins it.
    pub fn recover_team(&mut self, path: &str, recovery_key: &str, my_name: &str) -> Result<TeamView> {
        let me = self.my_id()?;
        TeamVault::recover(path, recovery_key, &me.detour_id, my_name)?;
        let info = TeamVault::inspect(path)?;
        match self.join_team(path, &info.root) {
            Err(Error::AlreadyMember) => self.sync_team(&info.id),
            other => other,
        }
    }

    fn add_team(&mut self, t: TeamVault, place: Option<Place>) -> Result<TeamView> {
        let id = t.id().to_string();
        let (remote, drive) = Place::split(place);
        let r = TeamRef { id: id.clone(), path: t.path().display().to_string(), root: t.root().to_string(), remote, drive };
        self.vault_mut()?.teams.push(r);
        self.teams.insert(id.clone(), t);
        self.save()?;
        self.after_team(&id)
    }

    /// Leaves a team on this device and, with sync on, on this person's other devices
    /// (the shared vault is untouched).
    pub fn leave_team(&mut self, id: &str) -> Result<()> {
        self.vault_mut()?.team_links.retain(|l| l.team_id != id);
        self.link_errors.remove(id);
        self.forget_team(id)?;
        self.save()
    }

    /// Takes a team off this device only.
    fn forget_team(&mut self, id: &str) -> Result<()> {
        let gone: Vec<TeamRef> = self.unlocked()?.vault.teams.iter().filter(|t| t.id == id).cloned().collect();
        self.vault_mut()?.teams.retain(|t| t.id != id);
        self.teams.remove(id);
        self.team_errors.remove(id);
        self.git_status.remove(id);
        self.drop_cache(id);
        // A Git or drive team's local copy is only a cache of where it is kept.
        for r in gone.iter().filter(|r| r.mirrored()) {
            let _ = std::fs::remove_dir_all(mirror_of(r).files().parent().unwrap_or(Path::new(&r.path)));
        }
        Ok(())
    }

    /// Re-reads the team's storage (or retries opening it), then does the housekeeping it calls for.
    pub fn sync_team(&mut self, id: &str) -> Result<TeamView> {
        if !self.teams.contains_key(id) {
            self.open_teams();
        }
        let t = self.team_mut(id)?;
        t.reload()?;
        // Housekeeping is best effort: a read must not fail because a write could not happen.
        let _ = t.maintain();
        self.after_team(id)
    }

    pub fn team_put(&mut self, id: &str, update: VaultUpdate) -> Result<TeamView> {
        // Agent forwarding is this person's choice, kept here; the team never sees it.
        let forward: Vec<(String, bool)> = update.hosts.iter().map(|h| (format!("{id}/{}", h.id), h.forward_agent)).collect();
        self.team_mut(id)?.put(update)?;
        let v = self.vault_mut()?;
        let before = v.team_forward_agent.clone();
        for (key, on) in forward {
            if on {
                v.team_forward_agent.insert(key);
            } else {
                v.team_forward_agent.remove(&key);
            }
        }
        if v.team_forward_agent != before {
            self.save()?;
        }
        self.after_team(id)
    }

    /// A client's edit of a team as it saw it (`base`): only what it changed is applied.
    pub fn team_put_from(&mut self, id: &str, base: &VaultUpdate, next: VaultUpdate) -> Result<TeamView> {
        let current = self.team_view(id)?.data.into();
        self.team_put(id, crate::merge::merge(base, next, current))
    }

    pub fn team_rename(&mut self, id: &str, name: &str) -> Result<TeamView> {
        self.team_mut(id)?.rename(name)?;
        self.after_team(id)
    }

    pub fn team_add_member(&mut self, id: &str, detour_id: &str, name: &str, role: Role, access: Level) -> Result<TeamView> {
        self.team_mut(id)?.add_member(detour_id, name, role, access)?;
        self.after_team(id)
    }

    pub fn team_update_member(&mut self, id: &str, member: &str, role: Role) -> Result<TeamView> {
        self.team_mut(id)?.update_member(member, role)?;
        self.after_team(id)
    }

    pub fn team_remove_member(&mut self, id: &str, member: &str) -> Result<TeamView> {
        self.team_mut(id)?.remove_member(member)?;
        self.after_team(id)
    }

    pub fn team_set_access(&mut self, id: &str, changes: &[AccessChange]) -> Result<TeamView> {
        self.team_mut(id)?.set_access(changes)?;
        self.after_team(id)
    }

    pub fn team_resolve_field(&mut self, id: &str, coll: Coll, record: &str, field: &str, pick: usize) -> Result<TeamView> {
        self.team_mut(id)?.resolve_field(coll, record, field, pick)?;
        self.after_team(id)
    }

    pub fn team_resolve_deleted(&mut self, id: &str, coll: Coll, record: &str, keep: bool) -> Result<TeamView> {
        self.team_mut(id)?.resolve_deleted(coll, record, keep)?;
        self.after_team(id)
    }

    /// Sets (or with `None` removes) this person's own login for a team host.
    /// This person's own grouping of a team's hosts; `None` drops it.
    pub fn layout_set(&mut self, id: &str, layout: Option<Layout>) -> Result<TeamView> {
        self.team(id)?;
        let v = self.vault_mut()?;
        match layout {
            Some(l) => v.layouts.insert(id.to_string(), l),
            None => v.layouts.remove(id),
        };
        self.save()?;
        self.team_view(id)
    }

    /// This person's own settings of groups, without passwords.
    pub fn group_prefs(&self) -> Result<Vec<GroupPrefsView>> {
        Ok(self.unlocked()?.vault.group_prefs.iter().map(|(k, p)| p.view(k)).collect())
    }

    /// Sets (or with `None` drops) this person's own settings of a group.
    pub fn group_prefs_set(&mut self, prefs: GroupPrefsView, drop: bool) -> Result<Vec<GroupPrefsView>> {
        let vault = prefs.key.split('/').next().unwrap_or_default().to_string();
        if !(prefs.key.contains("/team:") || prefs.key.contains("/mine:")) || (vault != PERSONAL && self.team(&vault).is_err()) {
            return Err(not_found("group"));
        }
        if let Some(Auth::Key { key_id }) = &prefs.auth
            && !self.unlocked()?.vault.keys.iter().any(|k| &k.id == key_id)
        {
            return Err(not_found("key"));
        }
        // A proxy of this person's own or of the group's vault.
        if let Some(r) = &prefs.proxy
            && r.vault != PERSONAL
            && r.vault != vault
        {
            return Err(Error::Forbidden);
        }
        let v = self.vault_mut()?;
        if drop {
            v.group_prefs.remove(&prefs.key);
        } else {
            let kept = v.group_prefs.get(&prefs.key).and_then(|p| p.password.clone());
            let password = match prefs.new_password {
                Some(p) if p.is_empty() => None,
                Some(p) => Some(p),
                None => kept,
            };
            let password = if prefs.auth == Some(Auth::Password) { password } else { None };
            let p = GroupPrefs { proxy: prefs.proxy, user: prefs.user.filter(|u| !u.is_empty()), auth: prefs.auth, password };
            if p.is_empty() {
                v.group_prefs.remove(&prefs.key);
            } else {
                v.group_prefs.insert(prefs.key, p);
            }
        }
        self.save()?;
        self.group_prefs()
    }

    /// This person's own settings that hold for a host: those of the group it is in in their
    /// own grouping, else those of its group (each setting on its own).
    fn prefs_for(&self, vault: &str, host: &Host) -> Result<GroupPrefs> {
        let v = &self.unlocked()?.vault;
        let mine = v.layouts.get(vault).and_then(|l| l.groups.iter().find(|g| g.hosts.contains(&host.id))).and_then(|g| v.group_prefs.get(&format!("{vault}/mine:{}", g.id)));
        let team = v.group_prefs.get(&format!("{vault}/team:{}", host.group));
        let pick = |f: &dyn Fn(&GroupPrefs) -> bool| mine.filter(|p| f(p)).or(team.filter(|p| f(p))).cloned();
        let login = pick(&|p| p.auth.is_some() || p.user.is_some());
        Ok(GroupPrefs {
            proxy: pick(&|p| p.proxy.is_some()).and_then(|p| p.proxy),
            user: login.as_ref().and_then(|p| p.user.clone()),
            auth: login.as_ref().and_then(|p| p.auth.clone()),
            password: login.and_then(|p| p.password),
        })
    }

    pub fn team_own_login_set(&mut self, id: &str, host_id: &str, login: Option<OwnLoginView>) -> Result<TeamView> {
        if self.team(id)?.level(host_id) < Level::View {
            return Err(not_found("host"));
        }
        let key = format!("{id}/{host_id}");
        let v = self.vault_mut()?;
        match login {
            None => {
                v.team_own.remove(&key);
            }
            Some(l) => {
                if let Auth::Key { key_id } = &l.auth
                    && !v.keys.iter().any(|k| &k.id == key_id)
                {
                    return Err(not_found("key"));
                }
                let kept = v.team_own.get(&key).and_then(|o| o.password.clone());
                let password = l.new_password.filter(|p| !p.is_empty()).or(kept);
                v.team_own.insert(key, OwnLogin { user: l.user, password: if l.auth == Auth::Password { password } else { None }, auth: l.auth, active: l.active });
            }
        }
        self.save()?;
        self.team_view(id)
    }

    /* ---------- the same operations for any vault ---------- */

    pub fn set_host_password_in(&mut self, vault: &str, host_id: &str, password: Option<String>) -> Result<()> {
        if vault == PERSONAL { self.set_host_password(host_id, password) } else { self.team_mut(vault)?.set_host_password(host_id, password) }
    }

    /// A saved password, for showing or copying: the shared one, or with `own` this person's own login.
    pub fn host_password_in(&self, vault: &str, host_id: &str, own: bool) -> Result<Option<String>> {
        if vault == PERSONAL {
            return Ok(self.unlocked()?.vault.hosts.iter().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?.password.clone());
        }
        if own {
            return Ok(self.unlocked()?.vault.team_own.get(&format!("{vault}/{host_id}")).and_then(|o| o.password.clone()));
        }
        self.team(vault)?.host_password(host_id)
    }

    pub fn generate_key_in(&mut self, vault: &str, name: &str, algorithm: KeyAlgorithm, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        if vault == PERSONAL { self.generate_key(name, algorithm, passphrase, remember) } else { self.team_mut(vault)?.generate_key(name, algorithm, passphrase, remember) }
    }

    pub fn import_key_in(&mut self, vault: &str, name: &str, text: &str, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        if vault == PERSONAL {
            return self.import_key(name, text, passphrase, remember);
        }
        let entry = keys::import(name, text, passphrase, remember)?;
        self.team_mut(vault)?.add_key(entry)
    }

    pub fn export_key_in(&self, vault: &str, key_id: &str, file_passphrase: Option<&str>) -> Result<String> {
        if vault == PERSONAL {
            return self.export_key(key_id, file_passphrase);
        }
        keys::export(&self.team(vault)?.secret_key(key_id)?, file_passphrase)
    }

    pub fn set_key_passphrase_in(&mut self, vault: &str, key_id: &str, current: Option<&str>, new: Option<&str>, remember: bool) -> Result<KeyView> {
        if vault == PERSONAL {
            return self.set_key_passphrase(key_id, current, new, remember);
        }
        let t = self.team_mut(vault)?;
        let entry = t.secret_key(key_id)?;
        let updated = keys::change_passphrase(&entry, current, new, remember)?;
        t.replace_key(updated)
    }

    /// The login a team host is opened with: this person's own one if set,
    /// otherwise the shared one if they may have it, otherwise asking for a password.
    fn team_login(&self, vault: &str, host_id: &str) -> Result<(Host, Option<KeyEntry>)> {
        let t = self.team(vault)?;
        let mut host = t.host(host_id).ok_or_else(|| not_found("host"))?;
        let v = &self.unlocked()?.vault;
        host.forward_agent = v.team_forward_agent.contains(&format!("{vault}/{host_id}"));
        if let Some(own) = v.team_own.get(&format!("{vault}/{host_id}")).filter(|o| o.active) {
            let key = match &own.auth {
                Auth::Key { key_id } => Some(v.keys.iter().find(|k| &k.id == key_id).ok_or_else(|| not_found("key"))?.clone()),
                _ => None,
            };
            host.user = own.user.clone();
            host.auth = own.auth.clone();
            host.password = own.password.clone();
            return Ok((host, key));
        }
        if t.level(host_id) < Level::Secret {
            host.auth = Auth::Password;
            host.password = None;
            return Ok((host, None));
        }
        let key = match &host.auth {
            Auth::Key { key_id } => Some(t.secret_key(key_id)?),
            _ => None,
        };
        Ok((host, key))
    }

    /// A host as this person logs in to it: its login (their own, a group's of theirs, or the
    /// shared one), the key for it, and their settings of its group.
    fn login_in(&self, vault: &str, host_id: &str) -> Result<(Host, Option<KeyEntry>, GroupPrefs)> {
        let (mut host, mut key) = if vault == PERSONAL { self.personal_login(host_id)? } else { self.team_login(vault, host_id)? };
        // First this person's own settings of the host's group, then the host's.
        let prefs = self.prefs_for(vault, &host)?;
        if let Some(auth) = prefs.auth.clone() {
            key = match &auth {
                Auth::Key { key_id } => Some(self.unlocked()?.vault.keys.iter().find(|k| &k.id == key_id).cloned().ok_or_else(|| not_found("key"))?),
                _ => None,
            };
            host.password = prefs.password.clone();
            host.auth = auth;
        }
        if let Some(user) = prefs.user.clone() {
            host.user = user;
        }
        Ok((host, key, prefs))
    }

    /// The proxy a group's hosts share when they have none of their own.
    fn shared_proxy(&self, vault: &str, group: &str) -> Option<String> {
        let groups = if vault == PERSONAL { self.unlocked().ok()?.vault.groups.clone() } else { self.team(vault).ok()?.data().groups };
        groups.into_iter().find(|g| g.name == group).and_then(|g| g.proxy_id)
    }

    /// Everything a connection to a host needs: its login, its pinned key, and the way
    /// to it (a jump host of the same vault, with its own login, or a proxy).
    /// The user and login a host is reached with: this person's group settings, then their own
    /// login while it is on, then the host's (as `ssh` and exports should show it).
    pub fn login_as(&self, vault: &str, host_id: &str) -> Result<(String, Auth)> {
        let (host, _, _) = self.login_in(vault, host_id)?;
        Ok((host.user, host.auth))
    }

    pub fn connect_request_in(&self, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<ssh::ConnectRequest> {
        self.request_via(vault, host_id, secret, trust_fingerprint, &mut Vec::new())
    }

    fn request_via(&self, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>, seen: &mut Vec<String>) -> Result<ssh::ConnectRequest> {
        if seen.iter().any(|s| s == host_id) || seen.len() > 4 {
            return Err(Error::JumpLoop);
        }
        seen.push(host_id.to_string());
        let (host, key, prefs) = self.login_in(vault, host_id)?;
        let ep = endpoint(&host);
        let known = self.unlocked()?.vault.known_hosts.iter().find(|k| k.endpoint == ep).cloned();
        let shared = || self.shared_proxy(vault, &host.group);
        let route = match (prefs.proxy.clone(), &host.proxy_id, host.proxy.clone()) {
            (Some(r), _, _) => ssh::Route::Proxy(Box::new(self.proxy_for_connect(&r.vault, &r.id)?)),
            (None, Some(id), _) => ssh::Route::Proxy(Box::new(self.proxy_for_connect(vault, id)?)),
            (None, None, None) => match shared() {
                Some(id) => ssh::Route::Proxy(Box::new(self.proxy_for_connect(vault, &id)?)),
                None => ssh::Route::Direct,
            },
            (None, None, Some(Proxy::Jump { host_id: jump })) => ssh::Route::Jump(Box::new(self.request_via(vault, &jump, None, trust_fingerprint.clone(), seen)?)),
            // Written into the host by an older version.
            (None, None, Some(legacy)) => {
                let kind = legacy.as_kind().ok_or_else(|| not_found("proxy"))?;
                ssh::Route::Proxy(Box::new(ProxyEntry { id: String::new(), name: String::new(), kind, password: host.proxy_password.clone(), private_key: None, preshared_key: None, uuid: None }))
            }
        };
        Ok(ssh::ConnectRequest { host, key, secret, known, trust_fingerprint, route })
    }

    /// A saved proxy of a vault with its secrets, to connect through.
    pub fn proxy_for_connect(&self, vault: &str, id: &str) -> Result<ProxyEntry> {
        let missing = || Error::Proxy { reason: "missing".into() };
        if vault == PERSONAL {
            return self.unlocked()?.vault.proxies.iter().find(|p| p.id == id).cloned().ok_or_else(missing);
        }
        let t = self.team(vault)?;
        let p = t.proxy(id).ok_or_else(missing)?;
        // Without its secret a member can't use it; a proxy without one needs none.
        if !p.kind.secrets().is_empty() && t.level(id) < Level::Secret {
            return Err(Error::Proxy { reason: "noAccess".into() });
        }
        Ok(p)
    }

    /// A proxy as a UI is editing it, with the secrets saved for it filled in.
    pub fn proxy_draft(&self, vault: &str, view: ProxyView) -> Result<ProxyEntry> {
        let saved = if vault == PERSONAL { self.unlocked()?.vault.proxies.iter().find(|p| p.id == view.id).cloned() } else { self.team(vault)?.proxy(&view.id) };
        Ok(ProxyEntry::from_view(view, saved.as_ref()))
    }

    pub fn record_connection_in(&mut self, vault: &str, host_id: &str, new_keys: ssh::NewKeys) -> Result<()> {
        if vault == PERSONAL {
            return self.record_connection(host_id, new_keys);
        }
        let v = self.vault_mut()?;
        for k in new_keys {
            v.known_hosts.retain(|x| x.endpoint != k.endpoint);
            v.known_hosts.push(k);
        }
        v.team_last_connected.insert(format!("{vault}/{host_id}"), now_ms());
        self.save()
    }

    /// Saves a secret typed in during a login to wherever that login comes from.
    pub fn save_secret_in(&mut self, vault: &str, host_id: &str, kind: SecretKind, value: String) -> Result<()> {
        if vault == PERSONAL {
            return self.save_secret(host_id, kind, value);
        }
        let key = format!("{vault}/{host_id}");
        let (host, _) = self.team_login(vault, host_id)?;
        // An own login switched off isn't what this login came from.
        let own = self.unlocked()?.vault.team_own.get(&key).filter(|o| o.active).cloned();
        let level = self.team(vault)?.level(host_id);
        match (kind, own) {
            (SecretKind::Password, Some(mut o)) => {
                o.password = Some(value);
                self.vault_mut()?.team_own.insert(key, o);
                self.save()
            }
            (SecretKind::Passphrase, Some(OwnLogin { auth: Auth::Key { key_id }, .. })) => {
                let k = self.vault_mut()?.keys.iter_mut().find(|k| k.id == key_id).ok_or_else(|| not_found("key"))?;
                k.passphrase = Some(value);
                self.save()
            }
            (SecretKind::Password, None) if level >= Level::Edit => self.team_mut(vault)?.set_host_password(host_id, Some(value)),
            // Without the right to change the shared password, a typed one becomes this person's own login.
            (SecretKind::Password, None) => {
                self.vault_mut()?.team_own.insert(key, OwnLogin { user: host.user, auth: Auth::Password, password: Some(value), active: true });
                self.save()
            }
            (SecretKind::Passphrase, None) if level >= Level::Edit => {
                let Auth::Key { key_id } = &host.auth else { return Err(not_found("key")) };
                let t = self.team_mut(vault)?;
                let mut k = t.secret_key(key_id)?;
                k.passphrase = Some(value);
                t.replace_key(k).map(|_| ())
            }
            _ => Err(Error::Forbidden),
        }
    }

    /// A tunnel of any vault and the host it goes through.
    pub fn tunnel_in(&self, vault: &str, tunnel_id: &str) -> Result<Tunnel> {
        if vault == PERSONAL {
            return self.unlocked()?.vault.tunnels.iter().find(|t| t.id == tunnel_id).cloned().ok_or_else(|| not_found("tunnel"));
        }
        self.team(vault)?.tunnel(tunnel_id).ok_or_else(|| not_found("tunnel"))
    }

    pub fn forget_host_key_in(&mut self, vault: &str, host_id: &str) -> Result<()> {
        if vault == PERSONAL {
            return self.forget_host_key(host_id);
        }
        let host = self.team(vault)?.host(host_id).ok_or_else(|| not_found("host"))?;
        let ep = endpoint(&host);
        self.vault_mut()?.known_hosts.retain(|k| k.endpoint != ep);
        self.save()
    }
}

/* ---------- syncing the personal vault between this person's devices ---------- */

impl Store {
    /// Where a synced personal vault kept in Git has its local copy.
    pub fn sync_dir(&self) -> PathBuf {
        self.dir.join("sync")
    }

    fn sync_ref(&self) -> Option<TeamRef> {
        self.state.as_ref().and_then(|s| s.vault.sync.clone())
    }

    fn sync_is_git(&self) -> bool {
        self.sync_ref().is_some_and(|r| r.mirrored())
    }

    fn open_sync(&mut self) {
        let Some(r) = self.sync_ref() else { return };
        let opened = self.member().and_then(|(me, device)| TeamVault::open_cached(&r.path, &r.root, &me, &device, self.known_heads(&r.id), self.load_cache(&r.id)));
        match opened {
            Ok(t) => {
                self.save_cache(&r.id, &t);
                self.personal = Some(t);
            }
            Err(e) => self.sync_state.0 = serde_json::to_value(&e).ok(),
        }
    }

    /// Records changed here since the last sync.
    fn has_unsynced(&self) -> bool {
        let (Ok(s), Ok((me, _))) = (self.unlocked(), self.member()) else { return false };
        personal::records_of(&s.vault, &me) != s.vault.sync_base
    }

    /// Writes what this device changed to the synced copy, then takes the synced
    /// records as this device's own. Returns true if changes came from elsewhere.
    fn merge_personal(&mut self) -> Result<bool> {
        let me = self.member()?.0;
        let sv = self.personal.as_mut().ok_or_else(|| not_found("sync"))?;
        let s = self.state.as_mut().ok_or(Error::Locked)?;
        sv.reload()?;
        let local = personal::records_of(&s.vault, &me);
        sv.apply(personal::changes(&s.vault.sync_base, &local))?;
        // Acknowledgements are best effort.
        let _ = sv.maintain();
        let synced: BTreeMap<String, SyncRecord> = sv.all_records().into_iter().map(|(id, (coll, fields))| (id, SyncRecord { coll, fields })).collect();
        let came_in = synced != local;
        // The key this device reaches its repositories with stays, even deleted elsewhere: without
        // it, this device can't sync again. It goes back to the others with the next sync.
        let needed: Vec<KeyEntry> = repo_keys(&s.vault).iter().filter_map(|id| s.vault.keys.iter().find(|k| &k.id == id).cloned()).collect();
        let gone = personal::adopt(&mut s.vault, &synced);
        for k in needed {
            if !s.vault.keys.iter().any(|x| x.id == k.id) {
                s.vault.keys.push(k);
            }
        }
        s.vault.sync_base = synced;
        if let Some(r) = &s.vault.sync {
            s.vault.team_heads.insert(r.id.clone(), sv.known_heads().clone());
        }
        if let (Some(t), Some(id)) = (self.personal.take(), self.sync_ref().map(|r| r.id)) {
            self.save_cache(&id, &t);
            self.personal = Some(t);
        }
        // Left on another device: leave here too.
        for id in gone {
            self.forget_team(&id)?;
        }
        self.write_vault()?;
        Ok(came_in)
    }

    /// Remembers how the last sync went.
    pub fn note_sync(&mut self, outcome: &Result<()>) {
        match outcome {
            Ok(()) => self.sync_state = (None, Some(now_ms())),
            Err(e) => self.sync_state.0 = serde_json::to_value(e).ok(),
        }
    }

    pub fn sync_status(&self) -> Result<SyncStatus> {
        let r = self.unlocked()?.vault.sync.clone();
        let (error, synced_at) = self.sync_state.clone();
        Ok(SyncStatus {
            enabled: r.is_some(),
            location: r.as_ref().map(TeamRef::location),
            git: r.as_ref().is_some_and(|r| r.remote.is_some()),
            storage: r.as_ref().map(|r| r.storage().to_string()),
            drive: r.as_ref().and_then(|r| r.drive.clone()),
            pending: r.as_ref().is_some_and(pending_of),
            error: if r.is_some() { error } else { None },
            synced_at,
            problems: self.personal.as_ref().map(|t| t.view().problems).unwrap_or_default(),
        })
    }

    /// Re-reads the synced copy (opening it if it could not be opened before) and merges.
    /// Returns true if this device's records changed.
    pub fn sync_personal(&mut self) -> Result<bool> {
        if self.sync_ref().is_none() {
            return Err(not_found("sync"));
        }
        if self.personal.is_none() {
            self.open_sync();
        }
        let merged = if self.personal.is_some() { self.merge_personal() } else { Err(self.sync_unavailable()) };
        if merged.is_ok()
            && let Some(r) = self.sync_ref()
        {
            personal::remove_expired(Path::new(&r.path));
        }
        match &merged {
            // A repository counts as reached only once it was pulled and pushed.
            Ok(_) if self.sync_is_git() => {}
            Ok(_) => self.note_sync(&Ok(())),
            Err(e) => self.sync_state.0 = serde_json::to_value(e).ok(),
        }
        merged
    }

    fn sync_unavailable(&self) -> Error {
        self.sync_ref().map_or_else(|| not_found("sync"), |r| Error::Unavailable { path: r.path })
    }

    /// Starts syncing through an empty folder (or a new repository's local copy).
    pub fn sync_enable(&mut self, path: &str, place: Option<Place>) -> Result<()> {
        if self.unlocked()?.vault.sync.is_some() {
            return Err(Error::AlreadyInitialized);
        }
        if TeamVault::inspect_personal(path).is_ok() {
            return Err(Error::SyncExists);
        }
        let (me, device) = self.member()?;
        let t = TeamVault::create_personal(path, &me, &device)?;
        self.attach(t, place)
    }

    /// Creates a synced copy in `files` (a staging copy of a repository), keeping nothing open.
    pub fn sync_create_files(&self, files: &Path) -> Result<()> {
        let (me, device) = self.member()?;
        TeamVault::create_personal(files, &me, &device)?;
        Ok(())
    }

    /// Starts syncing with a vault this person's other devices use. `identity`
    /// came from a pairing code or the recovery key and is the vault's creator.
    /// This device's records join the ones already there.
    pub fn sync_join(&mut self, path: &str, identity: &Identity, place: Option<Place>) -> Result<()> {
        let v = &self.unlocked()?.vault;
        if v.sync.is_some() {
            return Err(Error::AlreadyInitialized);
        }
        let current = v.member_identity.as_ref().map(Identity::from_stored).transpose()?;
        // Teams know this device by its identity: taking another one would lock it out of them.
        if current.is_none_or(|c| c.public() != identity.public()) && !v.teams.is_empty() {
            return Err(Error::SyncHasTeams);
        }
        let device = v.device_id.clone().ok_or(Error::Locked)?;
        let meta = TeamVault::inspect_personal(path)?;
        let t = TeamVault::open(path, &meta.root, identity, &device, self.known_heads(&meta.id))?;
        self.vault_mut()?.member_identity = Some(identity.to_stored());
        self.attach(t, place)
    }

    /// Starts syncing with a vault this device just created elsewhere (a repository's or drive's staging copy).
    pub fn sync_open(&mut self, path: &str, place: Place) -> Result<()> {
        let me = self.member()?.0;
        self.sync_join(path, &me, Some(place))
    }

    fn attach(&mut self, t: TeamVault, place: Option<Place>) -> Result<()> {
        let (remote, drive) = Place::split(place);
        let r = TeamRef { id: t.id().to_string(), path: t.path().display().to_string(), root: t.root().to_string(), remote, drive };
        let v = self.vault_mut()?;
        v.sync = Some(r);
        v.sync_base.clear();
        v.sync_unread.clear();
        self.personal = Some(t);
        let merged = self.merge_personal().map(|_| ());
        let reached = merged.is_ok();
        self.note_sync(&merged);
        if !reached {
            self.write_vault()?;
        }
        merged
    }

    /// The folder, vault id and identity a pairing offer is made from.
    pub fn sync_offer_parts(&self) -> Result<(PathBuf, String, Identity)> {
        let r = self.sync_ref().ok_or_else(|| not_found("sync"))?;
        Ok((PathBuf::from(&r.path), r.id, self.member()?.0))
    }

    /// This person's identity as a recovery key: joins a synced vault without another device.
    pub fn sync_recovery_key(&self) -> Result<String> {
        Ok(self.member()?.0.to_recovery_key())
    }

    /// Stops syncing on this device. Its records stay; the synced copy is untouched.
    pub fn sync_disable(&mut self) -> Result<()> {
        let v = self.vault_mut()?;
        let Some(r) = v.sync.take() else { return Ok(()) };
        v.sync_base.clear();
        v.sync_unread.clear();
        v.team_heads.remove(&r.id);
        self.personal = None;
        self.sync_state = (None, None);
        self.drop_cache(&r.id);
        self.write_vault()?;
        if r.mirrored() {
            let _ = std::fs::remove_dir_all(mirror_of(&r).files().parent().unwrap_or(Path::new(&r.path)));
        }
        Ok(())
    }

    /// Where the synced vault is kept and its local copy's directory; `None` for a folder.
    pub fn sync_place(&self) -> Result<Option<(PathBuf, Place)>> {
        let r = self.sync_ref().ok_or_else(|| not_found("sync"))?;
        Ok(place_of(&r))
    }

    /// Forgets the pinned key of the sync repository's server, so a new one can be trusted.
    pub fn sync_forget_host_key(&mut self) -> Result<()> {
        let remote = self.sync_ref().and_then(|r| r.remote).ok_or_else(|| not_found("repository"))?;
        let url = SshUrl::parse(&remote.url)?;
        let ep = format!("{}:{}", url.host, url.port);
        self.vault_mut()?.known_hosts.retain(|k| k.endpoint != ep);
        ssh::close_pooled();
        self.save()
    }

    /// This device's GitHub sign-in.
    pub fn github_login(&self) -> Result<Option<crate::github::GithubLogin>> {
        Ok(self.unlocked()?.vault.github.clone())
    }

    /// This device's sign-ins to drives.
    pub fn drive_logins(&self) -> Result<Vec<crate::drive::DriveLogin>> {
        Ok(self.unlocked()?.vault.drives.clone())
    }

    /// This device's sign-in to a drive with `account`, or (with `None`) to any account of
    /// that drive, a working one first.
    pub fn drive_login(&self, p: crate::drive::Provider, account: Option<&str>) -> Result<Option<crate::drive::DriveLogin>> {
        let drives = &self.unlocked()?.vault.drives;
        let mut of = drives.iter().filter(|l| l.provider == p && account.is_none_or(|a| l.account == a));
        Ok(of.clone().find(|l| !l.access_token.is_empty()).or_else(|| of.next()).cloned())
    }

    /// Keeps this device's sign-in to a drive's account, in place of an earlier one to it.
    pub fn set_drive_login(&mut self, login: crate::drive::DriveLogin) -> Result<()> {
        let v = self.vault_mut()?;
        v.drives.retain(|l| l.provider != login.provider || l.account != login.account);
        v.drives.push(login);
        self.write_vault()
    }

    /// Forgets this device's sign-in to a drive's account (with `None`, to all its accounts).
    pub fn drive_sign_out(&mut self, p: crate::drive::Provider, account: Option<&str>) -> Result<()> {
        self.vault_mut()?.drives.retain(|l| l.provider != p || account.is_some_and(|a| l.account != a));
        self.write_vault()
    }

    /// Keeps (or with `None` forgets) this device's GitHub sign-in.
    pub fn set_github_login(&mut self, login: Option<crate::github::GithubLogin>) -> Result<()> {
        self.vault_mut()?.github = login;
        self.write_vault()
    }

    /// Replaces the HTTPS token of the sync repository.
    pub fn sync_set_token(&mut self, token: &str) -> Result<()> {
        let v = self.vault_mut()?;
        let remote = v.sync.as_mut().and_then(|r| r.remote.as_mut()).filter(|r| r.is_https()).ok_or_else(|| not_found("repository"))?;
        remote.token = Some(token.trim().to_string()).filter(|t| !t.is_empty());
        remote.github = false;
        self.write_vault()
    }

    /// Reaches the sync repository with this device's GitHub sign-in from now on.
    pub fn sync_use_github(&mut self) -> Result<()> {
        let v = self.vault_mut()?;
        let remote = v.sync.as_mut().and_then(|r| r.remote.as_mut()).filter(|r| r.is_https()).ok_or_else(|| not_found("repository"))?;
        remote.token = None;
        remote.github = true;
        self.write_vault()
    }
}

/* ---------- teams in Git: what the network operations in `sync` need ---------- */

/// The local copy of a Git team: `path` is its `files` directory.
fn mirror_of(r: &TeamRef) -> Mirror {
    Mirror::new(copy_dir(r))
}

/// The directory of a Git or drive team's local copy (`path` is its `files`).
fn copy_dir(r: &TeamRef) -> PathBuf {
    Path::new(&r.path).parent().unwrap_or(Path::new(&r.path)).to_path_buf()
}

/// Where a team (or the synced vault) is kept, with its local copy's directory; `None` for a folder.
fn place_of(r: &TeamRef) -> Option<(PathBuf, Place)> {
    match (&r.remote, &r.drive) {
        (Some(g), _) => Some((copy_dir(r), Place::Git(g.clone()))),
        (_, Some(d)) => Some((copy_dir(r), Place::Drive(d.clone()))),
        _ => None,
    }
}

/// Local changes the repository or drive doesn't have yet.
fn pending_of(r: &TeamRef) -> bool {
    match (&r.remote, &r.drive) {
        (Some(_), _) => mirror_of(r).pending().unwrap_or(false),
        (_, Some(_)) => crate::drive::Mirror::new(copy_dir(r)).pending().unwrap_or(false),
        _ => false,
    }
}

impl Store {
    fn teams_dir(&self) -> PathBuf {
        self.dir.join("teams")
    }

    /// A fresh directory for a repository being cloned or a team being created.
    pub fn staging_dir(&self) -> (String, PathBuf) {
        let token = format!("stage-{}", crate::new_id());
        let dir = self.teams_dir().join(&token);
        (token, dir)
    }

    /// The staging directory named by a token a UI handed back.
    pub fn staged(&self, token: &str) -> Result<PathBuf> {
        let ok = token.starts_with("stage-") && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let dir = self.teams_dir().join(token);
        if !ok || !dir.is_dir() {
            return Err(not_found("staged repository"));
        }
        Ok(dir)
    }

    /// How to reach a repository with this person's keys and pinned server keys.
    pub fn git_remote(&self, remote: &GitRemote, trust_fingerprint: Option<String>) -> Result<Remote> {
        if remote.is_https() {
            let url = HttpUrl::parse(&remote.url)?;
            let token = if remote.github {
                self.unlocked()?.vault.github.as_ref().map(|l| l.access_token.clone()).filter(|t| !t.is_empty())
            } else {
                remote.token.clone().filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string())
            };
            return Ok(Remote::Http(HttpRemote { url, token, github: remote.github }));
        }
        let url = SshUrl::parse(&remote.url)?;
        let v = &self.unlocked()?.vault;
        let key = match &remote.auth {
            Auth::Key { key_id } => Some(v.keys.iter().find(|k| &k.id == key_id).ok_or(Error::RepoKeyMissing)?.clone()),
            Auth::Agent => None,
            Auth::Password => return Err(Error::Git { message: "repositories need a key or ssh-agent".into() }),
        };
        let host = Host {
            id: "git".into(),
            group: String::new(),
            label: url.host.clone(),
            address: url.host.clone(),
            port: url.port,
            user: url.user.clone(),
            auth: remote.auth.clone(),
            tags: vec![],
            last_connected: None,
            password: None,
            proxy: None,
            proxy_password: None, proxy_id: None,
            forward_agent: false, order: None,
        };
        let known = v.known_hosts.iter().find(|k| k.endpoint == endpoint(&host)).cloned();
        Ok(Remote::Ssh { request: ssh::ConnectRequest { host, key, secret: None, known, trust_fingerprint, route: ssh::Route::Direct }, path: url.path })
    }

    /// Where a team is kept and its local copy's directory; `None` for a team in a folder.
    pub fn team_place(&self, id: &str) -> Result<Option<(PathBuf, Place)>> {
        let r = self.unlocked()?.vault.teams.iter().find(|t| t.id == id).cloned().ok_or_else(|| not_found("team"))?;
        Ok(place_of(&r))
    }

    /// Pins a server key the user agreed to trust.
    pub fn pin_known_host(&mut self, k: Option<KnownHost>) -> Result<()> {
        let Some(k) = k else { return Ok(()) };
        let v = self.vault_mut()?;
        v.known_hosts.retain(|x| x.endpoint != k.endpoint);
        v.known_hosts.push(k);
        self.save()
    }

    /// Replaces the HTTPS token of a team's repository (it expired or was revoked).
    pub fn set_git_token(&mut self, id: &str, token: &str) -> Result<()> {
        let v = self.vault_mut()?;
        let r = v.teams.iter_mut().find(|t| t.id == id).ok_or_else(|| not_found("team"))?;
        let remote = r.remote.as_mut().filter(|r| r.is_https()).ok_or_else(|| not_found("repository"))?;
        remote.token = Some(token.trim().to_string()).filter(|t| !t.is_empty());
        remote.github = false;
        self.save()
    }

    /// Reaches a team's repository with this device's GitHub sign-in from now on.
    pub fn team_use_github(&mut self, id: &str) -> Result<()> {
        let v = self.vault_mut()?;
        let r = v.teams.iter_mut().find(|t| t.id == id).ok_or_else(|| not_found("team"))?;
        let remote = r.remote.as_mut().filter(|r| r.is_https()).ok_or_else(|| not_found("repository"))?;
        remote.token = None;
        remote.github = true;
        self.save()
    }

    /// Forgets the pinned server key of a team's repository, so a new one can be trusted.
    pub fn forget_git_host_key(&mut self, id: &str) -> Result<()> {
        let r = self.unlocked()?.vault.teams.iter().find(|t| t.id == id).cloned().ok_or_else(|| not_found("team"))?;
        let remote = r.remote.ok_or_else(|| not_found("repository"))?;
        let url = SshUrl::parse(&remote.url)?;
        let ep = format!("{}:{}", url.host, url.port);
        self.vault_mut()?.known_hosts.retain(|k| k.endpoint != ep);
        ssh::close_pooled();
        self.save()
    }

    /// Remembers how the last attempt to reach a team's repository went.
    pub fn record_git(&mut self, id: &str, outcome: &Result<()>) {
        let entry = self.git_status.entry(id.to_string()).or_default();
        match outcome {
            Ok(()) => *entry = (None, Some(now_ms())),
            Err(e) => entry.0 = serde_json::to_value(e).ok(),
        }
    }

    /// Creates a team vault in `files` (a staging copy), returning nothing to keep open.
    pub fn create_team_files(&self, files: &Path, name: &str, my_name: &str, recovery_key: &str) -> Result<()> {
        let recovery = Identity::from_recovery_key(recovery_key)?;
        let (me, device) = self.member()?;
        TeamVault::create(files, name, &me, my_name, &device, &recovery)?;
        Ok(())
    }

    /// Makes a staged copy of a repository or drive folder a joined team. `root` is the creator the user confirmed.
    pub fn add_remote_team(&mut self, staged: &Path, root: &str, place: Place) -> Result<TeamView> {
        let info = TeamVault::inspect(staged.join("files"))?;
        if self.teams.contains_key(&info.id) || self.unlocked()?.vault.teams.iter().any(|t| t.id == info.id) {
            return Err(Error::AlreadyMember);
        }
        let dir = self.teams_dir().join(&info.id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::rename(staged, &dir)?;
        let files = dir.join("files");
        let (me, device) = self.member()?;
        let t = TeamVault::open(&files, root, &me, &device, self.known_heads(&info.id))?;
        self.add_team(t, Some(place))
    }
}

/// The keys this device reaches its repositories with: its sync's and its teams' in Git.
fn repo_keys(v: &VaultData) -> Vec<String> {
    let remotes = v.sync.iter().chain(&v.teams).filter_map(|t| t.remote.as_ref());
    remotes.filter_map(|r| if let Auth::Key { key_id } = &r.auth { Some(key_id.clone()) } else { None }).collect()
}

pub fn endpoint(host: &Host) -> String {
    format!("{}:{}", host.address, host.port)
}

fn not_found(what: &str) -> Error {
    Error::NotFound { what: what.to_string() }
}

/// Writes through a temporary file so a crash never leaves a half-written vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("detour-test-{}", crate::new_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn host(id: &str, auth: Auth) -> HostView {
        HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None, proxy_id: None, forward_agent: false, order: None }
    }

    fn update(hosts: Vec<HostView>, keys: Vec<KeyView>) -> VaultUpdate {
        VaultUpdate { hosts, keys, snippets: vec![], tunnels: vec![], proxies: None, groups: None }
    }

    #[test]
    fn create_lock_unlock_roundtrip() {
        let dir = temp_dir();
        let mut s = Store::open(&dir);
        assert!(!s.is_initialized());
        s.create("master").unwrap();
        s.put(update(vec![host("h1", Auth::Agent)], vec![])).unwrap();
        s.lock();
        assert!(matches!(s.view(), Err(Error::Locked)));
        assert!(matches!(s.unlock("wrong"), Err(Error::WrongMasterPassword)));

        let mut s2 = Store::open(&dir);
        s2.unlock("master").unwrap();
        assert_eq!(s2.view().unwrap().hosts.len(), 1);
        assert!(matches!(s2.create("x"), Err(Error::AlreadyInitialized)));
    }

    #[test]
    fn files_on_disk_do_not_contain_secrets() {
        let dir = temp_dir();
        let mut s = Store::open(&dir);
        s.create("master").unwrap();
        s.put(update(vec![host("h1", Auth::Password)], vec![])).unwrap();
        s.set_host_password("h1", Some("very-secret-pw".into())).unwrap();
        let raw = std::fs::read(dir.join(PERSONAL_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("very-secret-pw"));
        assert!(!s.view().unwrap().hosts.iter().any(|h| !h.has_password));
    }

    #[test]
    fn put_keeps_secrets_and_undo_restores_them() {
        let dir = temp_dir();
        let mut s = Store::open(&dir);
        s.create("m").unwrap();
        s.put(update(vec![host("h1", Auth::Password)], vec![])).unwrap();
        s.set_host_password("h1", Some("pw".into())).unwrap();
        let key = s.generate_key("k", KeyAlgorithm::Ed25519, None, false).unwrap();

        // Rename via put: password and private key survive.
        let mut h = host("h1", Auth::Password);
        h.label = "renamed".into();
        let mut kv = key.clone();
        kv.name = "k2".into();
        let view = s.put(update(vec![h.clone()], vec![kv.clone()])).unwrap();
        assert!(view.hosts[0].has_password);
        assert_eq!(view.keys[0].name, "k2");

        // Delete, then undo by putting them back.
        s.put(update(vec![], vec![])).unwrap();
        let view = s.put(update(vec![h], vec![kv])).unwrap();
        assert!(view.hosts[0].has_password);
        assert_eq!(view.keys.len(), 1);
        assert!(s.export_key(&key.id, None).unwrap().contains("OPENSSH PRIVATE KEY"));
    }

    #[test]
    fn new_password_is_saved_with_the_host_in_one_put() {
        let mut s = Store::open(temp_dir());
        s.create("m").unwrap();
        let mut h = host("h1", Auth::Password);
        h.new_password = Some("typed".into());
        let view = s.put(update(vec![h], vec![])).unwrap();
        assert!(view.hosts[0].has_password);
        let req = s.connect_request("h1", None, None).unwrap();
        assert_eq!(req.host.password.as_deref(), Some("typed"));
    }

    #[test]
    fn change_master_password() {
        let dir = temp_dir();
        let mut s = Store::open(&dir);
        s.create("old").unwrap();
        s.change_master_password("old", "new").unwrap();
        s.lock();
        assert!(s.unlock("old").is_err());
        s.unlock("new").unwrap();
    }
}

/* ---------- moving over from OpenSSH ---------- */

/// What importing from `~/.ssh` would bring.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub config_path: String,
    pub config_found: bool,
    pub hosts: Vec<ImportHost>,
    /// Server keys not pinned here yet.
    pub known_new: usize,
    /// Pinned here with a different key: left as they are.
    pub known_different: usize,
    /// Hashed lines, which can't be read.
    pub known_hashed: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHost {
    #[serde(flatten)]
    pub host: crate::import::ConfigHost,
    /// The same host (address, port and user, or name) is in the vault already.
    pub exists: bool,
    /// Its `IdentityFile` can't be read: it would log in with ssh-agent.
    pub key_missing: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub hosts: usize,
    pub keys: usize,
    pub known: usize,
}

const IMPORT_GROUP: &str = "SSH config";

impl Store {
    fn same_host(v: &VaultData, alias: &str, address: &str, port: u16, user: &str) -> Option<String> {
        v.hosts
            .iter()
            .find(|h| (h.address.eq_ignore_ascii_case(address) && h.port == port && h.user == user) || h.label == alias)
            .map(|h| h.id.clone())
    }

    /// What `config` and `known_hosts` in `ssh` (normally `~/.ssh`) hold, against this vault.
    pub fn ssh_import_preview_from(&self, ssh: &Path) -> Result<ImportPreview> {
        let v = &self.unlocked()?.vault;
        let home = ssh.parent().unwrap_or(ssh);
        let config = ssh.join("config");
        let hosts = crate::import::read_config(&config, home)
            .into_iter()
            .map(|h| ImportHost {
                exists: Self::same_host(v, &h.alias, &h.address, h.port, &h.user).is_some(),
                key_missing: h.key_file.as_ref().is_some_and(|f| !Path::new(f).is_file()),
                host: h,
            })
            .collect();
        let known = crate::import::read_known_hosts(&ssh.join("known_hosts"));
        let pinned = |e: &crate::import::KnownEntry| v.known_hosts.iter().find(|k| k.endpoint.eq_ignore_ascii_case(&e.endpoint)).map(|k| k.fingerprint == e.fingerprint);
        Ok(ImportPreview {
            config_path: config.display().to_string(),
            config_found: config.is_file(),
            hosts,
            known_new: known.entries.iter().filter(|e| pinned(e).is_none()).count(),
            known_different: known.entries.iter().filter(|e| pinned(e) == Some(false)).count(),
            known_hashed: known.hashed,
        })
    }

    pub fn ssh_import_preview(&self) -> Result<ImportPreview> {
        self.ssh_import_preview_from(&crate::import::ssh_dir().ok_or_else(|| not_found("home folder"))?)
    }

    /// Imports the chosen hosts (with their keys and the hosts they go through)
    /// and, if asked, server keys not pinned yet. Hosts already here are skipped.
    pub fn ssh_import_from(&mut self, ssh: &Path, aliases: &[String], known_hosts: bool) -> Result<ImportResult> {
        let home = ssh.parent().unwrap_or(ssh).to_path_buf();
        let config = crate::import::read_config(&ssh.join("config"), &home);
        let mut result = ImportResult { hosts: 0, keys: 0, known: 0 };
        // Alias -> host id, for hosts imported now or already here.
        let mut ids: HashMap<String, String> = HashMap::new();
        // Hosts this import created, which may still get a route.
        let mut created: Vec<String> = Vec::new();
        let mut queue: Vec<String> = aliases.to_vec();
        let mut done: Vec<String> = Vec::new();
        while let Some(alias) = queue.pop() {
            if done.contains(&alias) {
                continue;
            }
            done.push(alias.clone());
            let Some(h) = config.iter().find(|h| h.alias == alias).cloned() else { continue };
            if let Some(id) = Self::same_host(&self.unlocked()?.vault, &h.alias, &h.address, h.port, &h.user) {
                ids.insert(alias, id);
                continue;
            }
            // Hosts a chosen one goes through come along.
            for hop in &h.jumps {
                if config.iter().any(|c| &c.alias == hop) && !done.contains(hop) {
                    queue.push(hop.clone());
                }
            }
            let auth = match h.key_file.as_deref().and_then(|f| std::fs::read_to_string(f).ok().map(|text| (f.to_string(), text))) {
                Some((file, text)) => {
                    let name = Path::new(&file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "imported-key".into());
                    match keys::import_locked(&name, &text) {
                        Ok(entry) => {
                            let v = self.vault_mut()?;
                            let id = match v.keys.iter().find(|k| k.fingerprint == entry.fingerprint) {
                                Some(k) => k.id.clone(),
                                None => {
                                    let id = entry.id.clone();
                                    v.keys.push(entry);
                                    result.keys += 1;
                                    id
                                }
                            };
                            Auth::Key { key_id: id }
                        }
                        Err(_) => Auth::Agent,
                    }
                }
                None => Auth::Agent,
            };
            let id = crate::new_id();
            ids.insert(alias.clone(), id.clone());
            created.push(id.clone());
            self.vault_mut()?.hosts.push(Host {
                id,
                group: IMPORT_GROUP.into(),
                label: h.alias.clone(),
                address: h.address.clone(),
                port: h.port,
                user: h.user.clone(),
                auth,
                tags: vec![],
                last_connected: None,
                password: None,
                proxy: None,
                proxy_password: None, proxy_id: None,
                forward_agent: h.forward_agent,
                order: None,
            });
            result.hosts += 1;
        }
        // Routes, once every host has an id: ProxyJump a,b means a, then b, then the host.
        for h in config.iter().filter(|h| done.contains(&h.alias) && !h.jumps.is_empty()) {
            let Some(target) = ids.get(&h.alias).cloned() else { continue };
            if !created.contains(&target) {
                continue;
            }
            let mut previous: Option<String> = None;
            for hop in &h.jumps {
                let hop_id = match ids.get(hop) {
                    Some(id) => id.clone(),
                    None => {
                        let (user, address, port) = crate::import::parse_hop(hop, &h.user);
                        let v = self.vault_mut()?;
                        match Self::same_host(v, hop, &address, port, &user) {
                            Some(id) => id,
                            None => {
                                let id = crate::new_id();
                                v.hosts.push(Host { id: id.clone(), group: IMPORT_GROUP.into(), label: hop.clone(), address, port, user, auth: Auth::Agent, tags: vec![], last_connected: None, password: None, proxy: None, proxy_password: None, proxy_id: None, forward_agent: false, order: None });
                                result.hosts += 1;
                                // A hop written out in full goes through the hop before it.
                                if let Some(prev) = &previous
                                    && let Some(x) = v.hosts.last_mut()
                                {
                                    x.proxy = Some(Proxy::Jump { host_id: prev.clone() });
                                }
                                id
                            }
                        }
                    }
                };
                ids.entry(hop.clone()).or_insert_with(|| hop_id.clone());
                previous = Some(hop_id);
            }
            if let Some(last) = previous
                && let Some(x) = self.vault_mut()?.hosts.iter_mut().find(|x| x.id == target)
            {
                x.proxy = Some(Proxy::Jump { host_id: last });
            }
        }
        if known_hosts {
            let known = crate::import::read_known_hosts(&ssh.join("known_hosts"));
            let v = self.vault_mut()?;
            for e in known.entries {
                if !v.known_hosts.iter().any(|k| k.endpoint.eq_ignore_ascii_case(&e.endpoint)) {
                    v.known_hosts.push(e.to_known());
                    result.known += 1;
                }
            }
        }
        self.save()?;
        Ok(result)
    }

    pub fn ssh_import(&mut self, aliases: &[String], known_hosts: bool) -> Result<ImportResult> {
        let ssh = crate::import::ssh_dir().ok_or_else(|| not_found("home folder"))?;
        self.ssh_import_from(&ssh, aliases, known_hosts)
    }
}

/* ---------- Detour files ---------- */

/// A host in a Detour file, as the import shows it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleHost {
    pub id: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    pub group: String,
    /// The host it goes through, by its name in the file.
    pub via: Option<String>,
    /// The same host (address, port and user, or name) is in the vault already.
    pub exists: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub hosts: Vec<BundleHost>,
    pub counts: crate::bundle::Counts,
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleResult {
    pub hosts: usize,
    pub keys: usize,
    pub proxies: usize,
    pub known: usize,
}

impl Store {
    /// The chosen hosts of a vault, with the hosts they go through, as a Detour file carries
    /// them: each with the login and the way this person reaches it by.
    pub fn bundle_of(&self, vault: &str, host_ids: &[String], include: crate::bundle::Include) -> Result<crate::bundle::Bundle> {
        let mut b = crate::bundle::Bundle::new();
        let v = &self.unlocked()?.vault;
        let groups = if vault == PERSONAL { v.groups.clone() } else { self.team(vault)?.data().groups };
        let mut queue: Vec<String> = host_ids.iter().rev().cloned().collect();
        while let Some(id) = queue.pop() {
            if b.hosts.iter().any(|h| h.id == id) {
                continue;
            }
            let (mut host, key, prefs) = self.login_in(vault, &id)?;
            // The way to it: a saved proxy (this person's choice for the group, the host's, the
            // group's shared one), or a jump host, which comes along.
            let proxy = match (&prefs.proxy, &host.proxy_id, &host.proxy) {
                (Some(r), _, _) => self.proxy_for_connect(&r.vault, &r.id).ok(),
                (None, Some(p), _) => self.proxy_for_connect(vault, p).ok(),
                (None, None, None) => self.shared_proxy(vault, &host.group).and_then(|p| self.proxy_for_connect(vault, &p).ok()),
                _ => None,
            };
            host.proxy_id = None;
            if let Some(mut p) = proxy.filter(|_| include.proxies) {
                if matches!(host.proxy, Some(Proxy::Jump { .. })) {
                    host.proxy = None;
                }
                host.proxy_id = Some(p.id.clone());
                if !include.passwords {
                    p.password = None;
                }
                if !b.proxies.iter().any(|x| x.id == p.id) {
                    b.proxies.push(p);
                }
            }
            if let Some(Proxy::Jump { host_id }) = &host.proxy {
                queue.push(host_id.clone());
            }
            match key.filter(|_| include.keys) {
                Some(mut k) => {
                    if !include.passwords {
                        k.passphrase = None;
                    }
                    if !b.keys.iter().any(|x| x.id == k.id) {
                        b.keys.push(k);
                    }
                }
                None if matches!(host.auth, Auth::Key { .. }) => host.auth = Auth::Agent,
                None => {}
            }
            if !include.passwords {
                (host.password, host.proxy_password) = (None, None);
            }
            if include.known_hosts {
                let ep = endpoint(&host);
                if let Some(k) = v.known_hosts.iter().find(|k| k.endpoint == ep)
                    && !b.known_hosts.iter().any(|x| x.endpoint == ep)
                {
                    b.known_hosts.push(k.clone());
                }
            }
            if include.groups {
                if let Some(g) = groups.iter().find(|g| g.name == host.group && !g.name.is_empty())
                    && !b.groups.iter().any(|x| x.name == g.name)
                {
                    // Its proxy is already the hosts' own.
                    b.groups.push(Group { proxy_id: None, ..g.clone() });
                }
            } else {
                (host.group, host.order) = (String::new(), None);
            }
            host.last_connected = None;
            b.hosts.push(host);
        }
        Ok(b)
    }

    /// What a Detour file holds, against this person's vault.
    pub fn bundle_preview(&self, b: &crate::bundle::Bundle) -> Result<BundlePreview> {
        let v = &self.unlocked()?.vault;
        let hosts = b
            .hosts
            .iter()
            .map(|h| BundleHost {
                id: h.id.clone(),
                label: h.label.clone(),
                address: h.address.clone(),
                port: h.port,
                user: h.user.clone(),
                group: h.group.clone(),
                via: match &h.proxy {
                    Some(Proxy::Jump { host_id }) => b.hosts.iter().find(|x| &x.id == host_id).map(|x| x.label.clone()),
                    _ => None,
                },
                exists: Self::same_host(v, &h.label, &h.address, h.port, &h.user).is_some(),
            })
            .collect();
        Ok(BundlePreview { hosts, counts: crate::bundle::Counts::of(b) })
    }

    /// Takes the chosen hosts of a Detour file into this person's vault, with what they log in
    /// with and the hosts they go through. A key or proxy already here is used, not added again;
    /// a host they go through that is here already is gone through.
    pub fn bundle_import(&mut self, b: &crate::bundle::Bundle, host_ids: &[String]) -> Result<BundleResult> {
        let mut result = BundleResult::default();
        // Host ids in the file -> here.
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut take: Vec<&Host> = Vec::new();
        let mut queue: Vec<(String, bool)> = host_ids.iter().rev().map(|id| (id.clone(), true)).collect();
        while let Some((id, chosen)) = queue.pop() {
            if ids.contains_key(&id) {
                continue;
            }
            let Some(h) = b.hosts.iter().find(|h| h.id == id) else { continue };
            // A host it goes through comes along unless it is here already.
            if !chosen && let Some(here) = Self::same_host(self.vault()?, &h.label, &h.address, h.port, &h.user) {
                ids.insert(id, here);
                continue;
            }
            ids.insert(id, crate::new_id());
            take.push(h);
            if let Some(Proxy::Jump { host_id }) = &h.proxy {
                queue.push((host_id.clone(), false));
            }
        }
        let v = self.vault_mut()?;
        let mut keys: HashMap<String, String> = HashMap::new();
        let mut proxies: HashMap<String, String> = HashMap::new();
        for h in &take {
            if let Auth::Key { key_id } = &h.auth
                && !keys.contains_key(key_id)
                && let Some(k) = b.keys.iter().find(|k| &k.id == key_id)
            {
                let id = match v.keys.iter().find(|x| x.fingerprint == k.fingerprint) {
                    Some(x) => x.id.clone(),
                    None => {
                        let id = crate::new_id();
                        v.keys.push(KeyEntry { id: id.clone(), ..k.clone() });
                        result.keys += 1;
                        id
                    }
                };
                keys.insert(key_id.clone(), id);
            }
            if let Some(pid) = &h.proxy_id
                && !proxies.contains_key(pid)
                && let Some(p) = b.proxies.iter().find(|p| &p.id == pid)
            {
                let same = |x: &&ProxyEntry| x.kind == p.kind && x.password == p.password && x.private_key == p.private_key && x.preshared_key == p.preshared_key && x.uuid == p.uuid;
                let id = match v.proxies.iter().find(same) {
                    Some(x) => x.id.clone(),
                    None => {
                        let id = crate::new_id();
                        v.proxies.push(ProxyEntry { id: id.clone(), ..p.clone() });
                        result.proxies += 1;
                        id
                    }
                };
                proxies.insert(pid.clone(), id);
            }
            if !h.group.is_empty()
                && !v.groups.iter().any(|g| g.name == h.group)
                && let Some(g) = b.groups.iter().find(|g| g.name == h.group)
            {
                v.groups.push(Group { id: crate::new_id(), proxy_id: None, ..g.clone() });
            }
            let mut host = (*h).clone();
            host.id = ids[&h.id].clone();
            host.auth = match &h.auth {
                Auth::Key { key_id } => keys.get(key_id).map(|id| Auth::Key { key_id: id.clone() }).unwrap_or(Auth::Agent),
                other => other.clone(),
            };
            host.proxy_id = h.proxy_id.as_ref().and_then(|p| proxies.get(p).cloned());
            if let Some(Proxy::Jump { host_id }) = &h.proxy {
                host.proxy = ids.get(host_id).map(|id| Proxy::Jump { host_id: id.clone() });
            }
            host.last_connected = None;
            let ep = endpoint(&host);
            if let Some(k) = b.known_hosts.iter().find(|k| k.endpoint == ep)
                && !v.known_hosts.iter().any(|x| x.endpoint.eq_ignore_ascii_case(&ep))
            {
                v.known_hosts.push(k.clone());
                result.known += 1;
            }
            v.hosts.push(host);
            result.hosts += 1;
        }
        self.save()?;
        Ok(result)
    }
}

/* ---------- teams on this person's other devices ---------- */

/// A team joined on another device of this person and not here (yet).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTeam {
    pub team_id: String,
    pub name: String,
    pub root: String,
    /// The repository of a team in Git, which is joined here by itself; otherwise a folder is asked for.
    pub url: Option<String>,
    /// The drive folder of a team in a cloud drive: joined here by itself once this device is signed in to the drive.
    pub drive: Option<crate::drive::DriveRemote>,
    /// Why joining it here failed (a serialized `Error`).
    pub error: Option<serde_json::Value>,
}

impl Store {
    /// Keeps the shared list of teams in step with the teams joined here: each team's
    /// name, creator and repository. Links of teams not joined here stay as they are.
    fn refresh_team_links(&mut self) {
        let Some(s) = self.state.as_mut() else { return };
        let v = &mut s.vault;
        for t in &v.teams {
            let name = self.teams.get(&t.id).map(|tv| tv.name().to_string()).filter(|n| !n.is_empty());
            let old = v.team_links.iter().position(|l| l.team_id == t.id);
            let link = TeamLink {
                team_id: t.id.clone(),
                name: name.or_else(|| old.map(|i| v.team_links[i].name.clone())).unwrap_or_default(),
                root: t.root.clone(),
                url: t.remote.as_ref().map(|r| r.url.clone()),
                auth: t.remote.as_ref().map(|r| r.auth.clone()),
                token: t.remote.as_ref().and_then(|r| r.token.clone()),
                github: t.remote.as_ref().is_some_and(|r| r.github),
                drive: t.drive.clone(),
            };
            match old {
                Some(i) if v.team_links[i] != link => v.team_links[i] = link,
                Some(_) => {}
                None => v.team_links.push(link),
            }
        }
    }

    /// Teams on this person's other devices that are not joined here.
    pub fn pending_teams(&self) -> Result<Vec<PendingTeam>> {
        let v = &self.unlocked()?.vault;
        Ok(v.team_links
            .iter()
            .filter(|l| !v.teams.iter().any(|t| t.id == l.team_id))
            .map(|l| PendingTeam { team_id: l.team_id.clone(), name: l.name.clone(), root: l.root.clone(), url: l.url.clone(), drive: l.drive.clone(), error: self.link_errors.get(&l.team_id).cloned() })
            .collect())
    }

    /// The remote and creator of a team in Git to join here, from its link.
    pub fn pending_git_teams(&self) -> Result<Vec<(String, String, GitRemote)>> {
        let v = &self.unlocked()?.vault;
        Ok(v.team_links
            .iter()
            .filter(|l| !v.teams.iter().any(|t| t.id == l.team_id))
            .filter_map(|l| Some((l.team_id.clone(), l.root.clone(), GitRemote { url: l.url.clone()?, auth: l.auth.clone().unwrap_or(Auth::Agent), token: l.token.clone(), github: l.github })))
            .collect())
    }

    /// The drive folder and creator of a team in a drive to join here, from its link, when
    /// this device is signed in to that drive.
    pub fn pending_drive_teams(&self) -> Result<Vec<(String, String, crate::drive::DriveRemote)>> {
        let v = &self.unlocked()?.vault;
        Ok(v.team_links
            .iter()
            .filter(|l| !v.teams.iter().any(|t| t.id == l.team_id))
            .filter_map(|l| Some((l.team_id.clone(), l.root.clone(), l.drive.clone()?)))
            .filter(|(_, _, d)| v.drives.iter().any(|x| x.provider == d.provider && d.account.as_ref().is_none_or(|a| &x.account == a) && !x.access_token.is_empty()))
            .collect())
    }

    /// A team joined on another device, its link with this device's creator to check against.
    pub fn team_link(&self, team_id: &str) -> Result<TeamLink> {
        self.unlocked()?.vault.team_links.iter().find(|l| l.team_id == team_id).cloned().ok_or_else(|| not_found("team"))
    }

    pub fn note_link_error(&mut self, team_id: &str, e: Option<&Error>) {
        match e {
            Some(e) => {
                self.link_errors.insert(team_id.to_string(), serde_json::to_value(e).unwrap_or_default());
            }
            None => {
                self.link_errors.remove(team_id);
            }
        }
    }

    /// Joins, in a folder of this device, a team joined on another device.
    pub fn join_linked_team(&mut self, team_id: &str, path: &str) -> Result<TeamView> {
        let link = self.unlocked()?.vault.team_links.iter().find(|l| l.team_id == team_id).cloned().ok_or_else(|| not_found("team"))?;
        if TeamVault::inspect(path)?.id != team_id {
            return Err(Error::NotATeamVault);
        }
        let view = self.join_team(path, &link.root)?;
        self.link_errors.remove(team_id);
        Ok(view)
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    use crate::bundle::{self, Include};

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("detour-test-{}", crate::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(dir);
        s.create("m").unwrap();
        s
    }

    fn host(id: &str, address: &str, auth: Auth) -> Host {
        Host { id: id.into(), group: "Web".into(), label: id.into(), address: address.into(), port: 22, user: "root".into(), auth, tags: vec![], last_connected: Some(1), password: None, proxy: None, proxy_password: None, proxy_id: None, forward_agent: false, order: Some(2.0) }
    }

    /// web (a password, a saved SOCKS proxy with a password, a pinned key) and db, through web with a key.
    fn source() -> (Store, String) {
        let mut s = store();
        let key = keys::generate("deploy", KeyAlgorithm::Ed25519, None, false).unwrap();
        let key_id = key.id.clone();
        let v = s.vault_mut().unwrap();
        v.keys.push(key);
        v.proxies.push(ProxyEntry { id: "p1".into(), name: "office".into(), kind: ProxyKind::Socks5 { address: "10.9.9.9".into(), port: 1080, user: "me".into() }, password: Some("sockspw".into()), private_key: None, preshared_key: None, uuid: None });
        v.groups.push(Group { id: "g1".into(), name: "Web".into(), order: 1.0, proxy_id: None });
        let mut web = host("web", "10.0.0.1", Auth::Password);
        web.password = Some("webpw".into());
        web.proxy_id = Some("p1".into());
        let mut db = host("db", "10.0.0.2", Auth::Key { key_id: key_id.clone() });
        db.proxy = Some(Proxy::Jump { host_id: "web".into() });
        v.hosts.extend([web, db]);
        v.known_hosts.push(KnownHost { endpoint: "10.0.0.1:22".into(), key: "ssh-ed25519 AAAA".into(), fingerprint: "SHA256:x".into(), added_at: 1 });
        s.save().unwrap();
        (s, key_id)
    }

    #[test]
    fn hosts_go_over_to_another_detour_with_what_logs_in_to_them() {
        let (a, _) = source();
        // db alone takes web along, the host it goes through.
        let b = a.bundle_of(PERSONAL, &["db".into()], Include::ALL).unwrap();
        assert_eq!(bundle::Counts::of(&b), bundle::Counts { hosts: 2, passwords: 2, keys: 1, proxies: 1, known_hosts: 1, groups: 1 });
        let b = bundle::open(bundle::seal(&b, "pw").unwrap().as_bytes(), "pw").unwrap();

        let mut other = store();
        let preview = other.bundle_preview(&b).unwrap();
        assert_eq!(preview.hosts.iter().map(|h| (h.label.as_str(), h.via.as_deref(), h.exists)).collect::<Vec<_>>(), [("db", Some("web"), false), ("web", None, false)]);
        let r = other.bundle_import(&b, &["db".into()]).unwrap();
        assert_eq!((r.hosts, r.keys, r.proxies, r.known), (2, 1, 1, 1));
        let v = other.vault().unwrap();
        let web = v.hosts.iter().find(|h| h.label == "web").unwrap();
        let db = v.hosts.iter().find(|h| h.label == "db").unwrap();
        assert_ne!(web.id, "web");
        assert_eq!(db.proxy, Some(Proxy::Jump { host_id: web.id.clone() }));
        assert!(matches!(&db.auth, Auth::Key { key_id } if v.keys.iter().any(|k| &k.id == key_id && !k.private_key.is_empty())));
        assert_eq!(web.password.as_deref(), Some("webpw"));
        let p = v.proxies.iter().find(|p| Some(&p.id) == web.proxy_id.as_ref()).unwrap();
        assert_eq!((p.name.as_str(), p.password.as_deref()), ("office", Some("sockspw")));
        assert_eq!((web.group.as_str(), web.order, web.last_connected), ("Web", Some(2.0), None));
        assert!(v.groups.iter().any(|g| g.name == "Web"));
        assert!(v.known_hosts.iter().any(|k| k.endpoint == "10.0.0.1:22"));
        // It connects as the source did.
        let req = other.connect_request(&db.id, None, None).unwrap();
        assert!(req.key.is_some() && matches!(req.route, ssh::Route::Jump(_)));

        // Again: the hosts are here now, and the key and proxy aren't added twice.
        let preview = other.bundle_preview(&b).unwrap();
        assert!(preview.hosts.iter().all(|h| h.exists));
        let r = other.bundle_import(&b, &["web".into()]).unwrap();
        assert_eq!((r.hosts, r.keys, r.proxies, r.known), (1, 0, 0, 0));
    }

    #[test]
    fn what_is_left_out_stays_out() {
        let (a, _) = source();
        let none = Include { passwords: false, keys: false, proxies: false, known_hosts: false, groups: false };
        let b = a.bundle_of(PERSONAL, &["db".into(), "web".into()], none).unwrap();
        assert_eq!(bundle::Counts::of(&b), bundle::Counts { hosts: 2, ..Default::default() });
        let text = serde_json::to_string(&b).unwrap();
        assert!(!text.contains("webpw") && !text.contains("sockspw") && !text.contains("PRIVATE KEY"), "{text}");
        let db = b.hosts.iter().find(|h| h.id == "db").unwrap();
        assert_eq!((&db.auth, db.group.as_str(), db.order), (&Auth::Agent, "", None));
        let web = b.hosts.iter().find(|h| h.id == "web").unwrap();
        assert_eq!(web.proxy_id, None);
        // A host already here that a chosen one goes through is gone through, not copied.
        let mut other = store();
        other.bundle_import(&b, &["web".into()]).unwrap();
        let r = other.bundle_import(&b, &["db".into()]).unwrap();
        assert_eq!(r.hosts, 1);
        let v = other.vault().unwrap();
        let web = v.hosts.iter().find(|h| h.label == "web").unwrap();
        assert_eq!(v.hosts.iter().find(|h| h.label == "db").unwrap().proxy, Some(Proxy::Jump { host_id: web.id.clone() }));
    }
}

#[cfg(test)]
mod repo_key_tests {
    use super::*;
    use std::sync::Mutex;

    fn device(tag: &str) -> Mutex<Store> {
        let dir = std::env::temp_dir().join(format!("detour-test-{tag}-{}", crate::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(dir);
        s.create("m").unwrap();
        Mutex::new(s)
    }

    fn without_key(s: &Mutex<Store>, id: &str) -> Result<VaultView> {
        let mut st = s.lock().unwrap();
        let v = st.view().unwrap();
        let keys = v.keys.into_iter().filter(|k| k.id != id).collect();
        st.put(VaultUpdate { hosts: v.hosts, keys, snippets: v.snippets, tunnels: v.tunnels, proxies: None, groups: Some(v.groups) })
    }

    /// As if this device reached its synced vault in Git with this key.
    fn reach_with(s: &Mutex<Store>, key_id: &str) {
        let mut st = s.lock().unwrap();
        let r = st.vault_mut().unwrap().sync.as_mut().unwrap();
        r.remote = Some(GitRemote { url: "git@example.com:me/vault.git".into(), auth: Auth::Key { key_id: key_id.into() }, token: None, github: false });
    }

    #[tokio::test]
    async fn a_device_keeps_the_key_it_reaches_its_repository_with() {
        let laptop = device("laptop");
        let folder = std::env::temp_dir().join(format!("detour-test-sync-{}", crate::new_id())).join("vault");
        laptop.lock().unwrap().sync_enable(&folder.display().to_string(), None).unwrap();
        let code = crate::sync::add_device(&laptop).await.unwrap();
        let desktop = device("desktop");
        let git = desktop.lock().unwrap().generate_key("git", KeyAlgorithm::Ed25519, None, false).unwrap();
        crate::sync::join_folder_sync(&desktop, folder.display().to_string(), code.code).await.unwrap();
        laptop.lock().unwrap().sync_personal().unwrap();
        reach_with(&desktop, &git.id);

        // Deleting it here is refused.
        assert!(matches!(without_key(&desktop, &git.id), Err(Error::KeyInUse)));
        // Deleted on the laptop, where nothing uses it: the desktop keeps it, and it comes back.
        without_key(&laptop, &git.id).unwrap();
        laptop.lock().unwrap().sync_personal().unwrap();
        desktop.lock().unwrap().sync_personal().unwrap();
        assert!(desktop.lock().unwrap().view().unwrap().keys.iter().any(|k| k.id == git.id));
        desktop.lock().unwrap().sync_personal().unwrap();
        laptop.lock().unwrap().sync_personal().unwrap();
        assert!(laptop.lock().unwrap().view().unwrap().keys.iter().any(|k| k.id == git.id));
        // A key nothing reaches a repository with still goes.
        let other = laptop.lock().unwrap().generate_key("other", KeyAlgorithm::Ed25519, None, false).unwrap();
        without_key(&laptop, &other.id).unwrap();
    }

    #[test]
    fn a_repository_whose_key_is_gone_says_so() {
        let s = device("gone");
        let mut st = s.lock().unwrap();
        let remote = GitRemote { url: "git@example.com:me/vault.git".into(), auth: Auth::Key { key_id: "missing".into() }, token: None, github: false };
        assert!(matches!(st.git_remote(&remote, None), Err(Error::RepoKeyMissing)));
        let _ = st.vault_mut();
    }
}
