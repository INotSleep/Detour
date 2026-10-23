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
use crate::team::state::{Coll, Role};
use crate::team::{TeamVault, TeamView, VaultInfo};
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
        Self { dir: dir.as_ref().to_path_buf(), state: None, trash_keys: HashMap::new(), trash_passwords: HashMap::new(), teams: BTreeMap::new(), team_errors: BTreeMap::new() }
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
        Ok(())
    }

    /// Forgets every decrypted secret held in memory.
    pub fn lock(&mut self) {
        self.state = None;
        self.trash_keys.clear();
        self.trash_passwords.clear();
        self.teams.clear();
        self.team_errors.clear();
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

    fn save(&self) -> Result<()> {
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
                let kept = v.new_password.filter(|p| !p.is_empty()).or_else(|| old.and_then(|h| h.password)).or_else(|| trash_passwords.remove(&v.id));
                Host {
                    password: if v.auth == Auth::Password { kept } else { None },
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
        let v = &self.unlocked()?.vault;
        let host = v.hosts.iter().find(|h| h.id == host_id).ok_or_else(|| not_found("host"))?.clone();
        let key = match &host.auth {
            Auth::Key { key_id } => Some(v.keys.iter().find(|k| &k.id == key_id).ok_or_else(|| not_found("key"))?.clone()),
            _ => None,
        };
        let endpoint = endpoint(&host);
        let known = v.known_hosts.iter().find(|k| k.endpoint == endpoint).cloned();
        Ok(ssh::ConnectRequest { host, key, secret, known, trust_fingerprint })
    }

    /// Records a successful login: pins a newly trusted host key and the time.
    pub fn record_connection(&mut self, host_id: &str, new_known: Option<KnownHost>) -> Result<()> {
        let vault = self.vault_mut()?;
        if let Some(k) = new_known {
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

    fn open_teams(&mut self) {
        let Ok((me, device)) = self.member() else { return };
        let refs = self.state.as_ref().map(|s| s.vault.teams.clone()).unwrap_or_default();
        for r in refs {
            match TeamVault::open(&r.path, &r.root, &me, &device) {
                Ok(t) => {
                    self.team_errors.remove(&r.id);
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

    /// Team view with this device's last-login times filled in.
    fn team_view(&self, id: &str) -> Result<TeamView> {
        let mut view = self.team(id)?.view();
        let last = &self.unlocked()?.vault.team_last_connected;
        for h in &mut view.data.hosts {
            h.last_connected = last.get(&format!("{id}/{}", h.id)).copied();
        }
        Ok(view)
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
        self.add_team(t)
    }

    pub fn inspect_team(path: &str) -> Result<VaultInfo> {
        TeamVault::inspect(path)
    }

    /// Joins a vault after the user confirmed its creator (`root`, a Detour ID).
    pub fn join_team(&mut self, path: &str, root: &str) -> Result<TeamView> {
        let (me, device) = self.member()?;
        let t = TeamVault::open(path, root, &me, &device)?;
        if self.teams.contains_key(t.id()) {
            return Err(Error::AlreadyMember);
        }
        self.add_team(t)
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

    fn add_team(&mut self, t: TeamVault) -> Result<TeamView> {
        let id = t.id().to_string();
        let r = TeamRef { id: id.clone(), path: t.path().display().to_string(), root: t.root().to_string() };
        self.vault_mut()?.teams.push(r);
        self.teams.insert(id.clone(), t);
        self.save()?;
        self.team_view(&id)
    }

    /// Forgets a team on this device (the shared vault is untouched).
    pub fn leave_team(&mut self, id: &str) -> Result<()> {
        self.vault_mut()?.teams.retain(|t| t.id != id);
        self.teams.remove(id);
        self.team_errors.remove(id);
        self.save()
    }

    /// Re-reads the team's storage (or retries opening it).
    pub fn sync_team(&mut self, id: &str) -> Result<TeamView> {
        if !self.teams.contains_key(id) {
            self.open_teams();
        }
        self.team_mut(id)?.reload()?;
        self.team_view(id)
    }

    pub fn team_put(&mut self, id: &str, update: VaultUpdate) -> Result<TeamView> {
        self.team_mut(id)?.put(update)?;
        self.team_view(id)
    }

    pub fn team_rename(&mut self, id: &str, name: &str) -> Result<TeamView> {
        self.team_mut(id)?.rename(name)?;
        self.team_view(id)
    }

    pub fn team_add_member(&mut self, id: &str, detour_id: &str, name: &str, role: Role, can_reveal: bool) -> Result<TeamView> {
        self.team_mut(id)?.add_member(detour_id, name, role, can_reveal)?;
        self.team_view(id)
    }

    pub fn team_update_member(&mut self, id: &str, member: &str, role: Role, can_reveal: bool) -> Result<TeamView> {
        self.team_mut(id)?.update_member(member, role, can_reveal)?;
        self.team_view(id)
    }

    pub fn team_remove_member(&mut self, id: &str, member: &str) -> Result<TeamView> {
        self.team_mut(id)?.remove_member(member)?;
        self.team_view(id)
    }

    pub fn team_resolve_field(&mut self, id: &str, coll: Coll, record: &str, field: &str, pick: usize) -> Result<TeamView> {
        self.team_mut(id)?.resolve_field(coll, record, field, pick)?;
        self.team_view(id)
    }

    pub fn team_resolve_deleted(&mut self, id: &str, coll: Coll, record: &str, keep: bool) -> Result<TeamView> {
        self.team_mut(id)?.resolve_deleted(coll, record, keep)?;
        self.team_view(id)
    }

    /* ---------- the same operations for any vault ---------- */

    pub fn set_host_password_in(&mut self, vault: &str, host_id: &str, password: Option<String>) -> Result<()> {
        if vault == PERSONAL { self.set_host_password(host_id, password) } else { self.team_mut(vault)?.set_host_password(host_id, password) }
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
        keys::export(&self.team(vault)?.revealable_key(key_id)?, file_passphrase)
    }

    pub fn set_key_passphrase_in(&mut self, vault: &str, key_id: &str, current: Option<&str>, new: Option<&str>, remember: bool) -> Result<KeyView> {
        if vault == PERSONAL {
            return self.set_key_passphrase(key_id, current, new, remember);
        }
        let t = self.team_mut(vault)?;
        let entry = t.key(key_id).ok_or_else(|| not_found("key"))?;
        let updated = keys::change_passphrase(&entry, current, new, remember)?;
        t.replace_key(updated)
    }

    pub fn connect_request_in(&self, vault: &str, host_id: &str, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<ssh::ConnectRequest> {
        if vault == PERSONAL {
            return self.connect_request(host_id, secret, trust_fingerprint);
        }
        let t = self.team(vault)?;
        let host = t.host(host_id).ok_or_else(|| not_found("host"))?;
        let key = match &host.auth {
            Auth::Key { key_id } => Some(t.key(key_id).ok_or_else(|| not_found("key"))?),
            _ => None,
        };
        let ep = endpoint(&host);
        let known = self.unlocked()?.vault.known_hosts.iter().find(|k| k.endpoint == ep).cloned();
        Ok(ssh::ConnectRequest { host, key, secret, known, trust_fingerprint })
    }

    pub fn record_connection_in(&mut self, vault: &str, host_id: &str, new_known: Option<KnownHost>) -> Result<()> {
        if vault == PERSONAL {
            return self.record_connection(host_id, new_known);
        }
        let v = self.vault_mut()?;
        if let Some(k) = new_known {
            v.known_hosts.retain(|x| x.endpoint != k.endpoint);
            v.known_hosts.push(k);
        }
        v.team_last_connected.insert(format!("{vault}/{host_id}"), now_ms());
        self.save()
    }

    pub fn save_secret_in(&mut self, vault: &str, host_id: &str, kind: SecretKind, value: String) -> Result<()> {
        if vault == PERSONAL {
            return self.save_secret(host_id, kind, value);
        }
        let t = self.team_mut(vault)?;
        let host = t.host(host_id).ok_or_else(|| not_found("host"))?;
        match (kind, &host.auth) {
            (SecretKind::Password, _) => t.set_host_password(host_id, Some(value)),
            (SecretKind::Passphrase, Auth::Key { key_id }) => {
                let mut k = t.key(key_id).ok_or_else(|| not_found("key"))?;
                k.passphrase = Some(value);
                t.replace_key(k).map(|_| ())
            }
            (SecretKind::Passphrase, _) => Err(not_found("key")),
        }
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
        HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth, tags: vec![], last_connected: None, has_password: false, new_password: None }
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
