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
    /// The folder, or the repository's URL.
    pub location: Option<String>,
    pub git: bool,
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
        if self.ensure_member()? {
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
        self.teams.clear();
        self.team_errors.clear();
        self.git_status.clear();
        self.personal = None;
        self.sync_state = (None, None);
        ssh::close_pooled();
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

    fn vault_mut(&mut self) -> Result<&mut VaultData> {
        Ok(&mut self.state.as_mut().ok_or(Error::Locked)?.vault)
    }

    /// Writes the vault file; with sync on, what changed goes to the synced copy first.
    fn save(&mut self) -> Result<()> {
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
        })
    }

    /// Applies a UI's edit of the vault while keeping every secret it can't see.
    pub fn put(&mut self, update: VaultUpdate) -> Result<VaultView> {
        let mut trash_keys = std::mem::take(&mut self.trash_keys);
        let mut trash_passwords = std::mem::take(&mut self.trash_passwords);
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

        self.trash_keys = trash_keys;
        self.trash_passwords = trash_passwords;
        self.save()?;
        self.view()
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

    fn team(&self, id: &str) -> Result<&TeamVault> {
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
        }
        if let Some(r) = v.teams.iter().find(|t| t.id == id)
            && let Some(remote) = &r.remote
        {
            let (error, synced_at) = self.git_status.get(id).cloned().unwrap_or_default();
            view.remote = Some(RemoteView { url: remote.url.clone(), pending: mirror_of(r).pending().unwrap_or(false), error, synced_at });
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

    fn add_team(&mut self, t: TeamVault, remote: Option<GitRemote>) -> Result<TeamView> {
        let id = t.id().to_string();
        let r = TeamRef { id: id.clone(), path: t.path().display().to_string(), root: t.root().to_string(), remote };
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
        // A Git team's local copy is only a cache of the repository.
        for r in gone.iter().filter(|r| r.remote.is_some()) {
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
        self.team_mut(id)?.put(update)?;
        self.after_team(id)
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
                v.team_own.insert(key, OwnLogin { user: l.user, password: if l.auth == Auth::Password { password } else { None }, auth: l.auth });
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
        if let Some(own) = v.team_own.get(&format!("{vault}/{host_id}")) {
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

    /// Everything a connection to a host needs: its login, its pinned key, and the way
    /// to it (a jump host of the same vault, with its own login, or a proxy).
    pub fn connect_request_in(&self, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<ssh::ConnectRequest> {
        self.request_via(vault, host_id, secret, trust_fingerprint, &mut Vec::new())
    }

    fn request_via(&self, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>, seen: &mut Vec<String>) -> Result<ssh::ConnectRequest> {
        if seen.iter().any(|s| s == host_id) || seen.len() > 4 {
            return Err(Error::JumpLoop);
        }
        seen.push(host_id.to_string());
        let (host, key) = if vault == PERSONAL { self.personal_login(host_id)? } else { self.team_login(vault, host_id)? };
        let ep = endpoint(&host);
        let known = self.unlocked()?.vault.known_hosts.iter().find(|k| k.endpoint == ep).cloned();
        let route = match host.proxy.clone() {
            None => ssh::Route::Direct,
            Some(Proxy::Jump { host_id: jump }) => ssh::Route::Jump(Box::new(self.request_via(vault, &jump, None, trust_fingerprint.clone(), seen)?)),
            Some(Proxy::Socks5 { address, port, user }) => ssh::Route::Socks5 { address, port, user, password: host.proxy_password.clone() },
            Some(Proxy::Http { address, port, user }) => ssh::Route::Http { address, port, user, password: host.proxy_password.clone() },
        };
        Ok(ssh::ConnectRequest { host, key, secret, known, trust_fingerprint, route })
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
        let own = self.unlocked()?.vault.team_own.get(&key).cloned();
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
                self.vault_mut()?.team_own.insert(key, OwnLogin { user: host.user, auth: Auth::Password, password: Some(value) });
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
        self.sync_ref().is_some_and(|r| r.remote.is_some())
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
        let gone = personal::adopt(&mut s.vault, &synced);
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
            location: r.as_ref().map(|r| r.remote.as_ref().map_or_else(|| r.path.clone(), |g| g.url.clone())),
            git: r.as_ref().is_some_and(|r| r.remote.is_some()),
            pending: r.as_ref().filter(|r| r.remote.is_some()).is_some_and(|r| mirror_of(r).pending().unwrap_or(false)),
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
    pub fn sync_enable(&mut self, path: &str, remote: Option<GitRemote>) -> Result<()> {
        if self.unlocked()?.vault.sync.is_some() {
            return Err(Error::AlreadyInitialized);
        }
        let (me, device) = self.member()?;
        let t = TeamVault::create_personal(path, &me, &device)?;
        self.attach(t, remote)
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
    pub fn sync_join(&mut self, path: &str, identity: &Identity, remote: Option<GitRemote>) -> Result<()> {
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
        self.attach(t, remote)
    }

    /// Starts syncing with a vault this device just created elsewhere (a repository's staging copy).
    pub fn sync_open(&mut self, path: &str, remote: GitRemote) -> Result<()> {
        let me = self.member()?.0;
        self.sync_join(path, &me, Some(remote))
    }

    fn attach(&mut self, t: TeamVault, remote: Option<GitRemote>) -> Result<()> {
        let r = TeamRef { id: t.id().to_string(), path: t.path().display().to_string(), root: t.root().to_string(), remote };
        let v = self.vault_mut()?;
        v.sync = Some(r);
        v.sync_base.clear();
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
        v.team_heads.remove(&r.id);
        self.personal = None;
        self.sync_state = (None, None);
        self.drop_cache(&r.id);
        self.write_vault()?;
        if r.remote.is_some() {
            let _ = std::fs::remove_dir_all(mirror_of(&r).files().parent().unwrap_or(Path::new(&r.path)));
        }
        Ok(())
    }

    /// The local copy and remote of a synced vault kept in Git; `None` for a folder.
    pub fn sync_git_job(&self, trust_fingerprint: Option<String>) -> Result<Option<(Mirror, Remote)>> {
        let r = self.sync_ref().ok_or_else(|| not_found("sync"))?;
        match &r.remote {
            None => Ok(None),
            Some(remote) => Ok(Some((mirror_of(&r), self.git_remote(remote, trust_fingerprint)?))),
        }
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

    /// Replaces the HTTPS token of the sync repository.
    pub fn sync_set_token(&mut self, token: &str) -> Result<()> {
        let v = self.vault_mut()?;
        let remote = v.sync.as_mut().and_then(|r| r.remote.as_mut()).filter(|r| r.is_https()).ok_or_else(|| not_found("repository"))?;
        remote.token = Some(token.trim().to_string()).filter(|t| !t.is_empty());
        self.write_vault()
    }
}

/* ---------- teams in Git: what the network operations in `sync` need ---------- */

/// The local copy of a Git team: `path` is its `files` directory.
fn mirror_of(r: &TeamRef) -> Mirror {
    Mirror::new(Path::new(&r.path).parent().unwrap_or(Path::new(&r.path)))
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
            let token = remote.token.clone().filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string());
            return Ok(Remote::Http(HttpRemote { url, token }));
        }
        let url = SshUrl::parse(&remote.url)?;
        let v = &self.unlocked()?.vault;
        let key = match &remote.auth {
            Auth::Key { key_id } => Some(v.keys.iter().find(|k| &k.id == key_id).ok_or_else(|| not_found("key"))?.clone()),
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
            proxy_password: None,
        };
        let known = v.known_hosts.iter().find(|k| k.endpoint == endpoint(&host)).cloned();
        Ok(Remote::Ssh { request: ssh::ConnectRequest { host, key, secret: None, known, trust_fingerprint, route: ssh::Route::Direct }, path: url.path })
    }

    /// The local copy and remote of a Git team; `None` for a team in a folder.
    pub fn git_job(&self, id: &str, trust_fingerprint: Option<String>) -> Result<Option<(Mirror, Remote)>> {
        let r = self.unlocked()?.vault.teams.iter().find(|t| t.id == id).cloned().ok_or_else(|| not_found("team"))?;
        match &r.remote {
            None => Ok(None),
            Some(remote) => Ok(Some((mirror_of(&r), self.git_remote(remote, trust_fingerprint)?))),
        }
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

    /// Makes a staged copy of a repository a joined team. `root` is the creator the user confirmed.
    pub fn add_git_team(&mut self, staged: &Path, root: &str, remote: GitRemote) -> Result<TeamView> {
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
        self.add_team(t, Some(remote))
    }
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
        HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None }
    }

    fn update(hosts: Vec<HostView>, keys: Vec<KeyView>) -> VaultUpdate {
        VaultUpdate { hosts, keys, snippets: vec![], tunnels: vec![] }
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
                proxy_password: None,
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
                                v.hosts.push(Host { id: id.clone(), group: IMPORT_GROUP.into(), label: hop.clone(), address, port, user, auth: Auth::Agent, tags: vec![], last_connected: None, password: None, proxy: None, proxy_password: None });
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
            .map(|l| PendingTeam { team_id: l.team_id.clone(), name: l.name.clone(), root: l.root.clone(), url: l.url.clone(), error: self.link_errors.get(&l.team_id).cloned() })
            .collect())
    }

    /// The remote and creator of a team in Git to join here, from its link.
    pub fn pending_git_teams(&self) -> Result<Vec<(String, String, GitRemote)>> {
        let v = &self.unlocked()?.vault;
        Ok(v.team_links
            .iter()
            .filter(|l| !v.teams.iter().any(|t| t.id == l.team_id))
            .filter_map(|l| Some((l.team_id.clone(), l.root.clone(), GitRemote { url: l.url.clone()?, auth: l.auth.clone().unwrap_or(Auth::Agent), token: l.token.clone() })))
            .collect())
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
