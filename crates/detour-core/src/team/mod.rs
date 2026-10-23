//! Team vaults: shared, end-to-end encrypted, no server. See `docs/teams.md`.

pub mod log;
pub mod state;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use self::log::{Body, EpochKeys, Folder, Hlc, Kind, VaultMeta};
use self::state::{Activity, Coll, Member, Op, Problem, Role, State};
use crate::identity::{Identity, PublicIdentity};
use crate::model::*;
use crate::store::VaultUpdate;
use crate::{Error, Result, keys, new_id};

/// Fields that hold secrets: never shown to members without "can reveal".
const SECRET_FIELDS: [&str; 3] = ["password", "privateKey", "passphrase"];

pub struct TeamVault {
    folder: Folder,
    meta: VaultMeta,
    device: String,
    me: Identity,
    state: State,
}

/// What a joining member is asked to confirm before trusting a vault.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInfo {
    pub id: String,
    pub root: String,
    pub root_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub can_reveal: bool,
    pub recovery: bool,
    pub fingerprint: String,
    pub is_you: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictValue {
    pub value: Value,
    pub author: String,
    pub author_name: String,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConflictView {
    Field { coll: Coll, record: String, field: String, values: Vec<ConflictValue> },
    Deleted { coll: Coll, record: String, by: String, by_name: String, at: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub id: String,
    pub name: String,
    pub path: String,
    /// You are in the member list (otherwise ask an admin to add your Detour ID).
    pub joined: bool,
    /// You were a member and got removed.
    pub removed: bool,
    pub my_role: Option<Role>,
    pub can_reveal: bool,
    pub members: Vec<MemberView>,
    pub data: VaultView,
    pub conflicts: Vec<ConflictView>,
    pub activity: Vec<ActivityView>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    pub author_name: String,
    pub is_you: bool,
    #[serde(flatten)]
    pub activity: Activity,
}

fn denied() -> Error {
    Error::Forbidden
}

fn fields_of<T: Serialize>(t: &T, skip: &[&str]) -> BTreeMap<String, Value> {
    match serde_json::to_value(t) {
        Ok(Value::Object(m)) => m.into_iter().filter(|(k, _)| k != "id" && !skip.contains(&k.as_str())).collect(),
        _ => BTreeMap::new(),
    }
}

fn from_fields<T: DeserializeOwned>(id: &str, fields: BTreeMap<String, Value>) -> Option<T> {
    let mut m: serde_json::Map<String, Value> = fields.into_iter().collect();
    m.insert("id".into(), Value::String(id.to_string()));
    serde_json::from_value(Value::Object(m)).ok()
}

impl TeamVault {
    /// Reads `vault.json` of a folder the user wants to join.
    pub fn inspect(path: impl AsRef<Path>) -> Result<VaultInfo> {
        let meta = Folder::new(path).read_meta()?.ok_or(Error::NotATeamVault)?;
        let root = PublicIdentity::parse(&meta.root)?;
        Ok(VaultInfo { id: meta.id, root: meta.root, root_fingerprint: root.fingerprint() })
    }

    /// Creates a new team vault in an empty folder. `recovery` becomes a second owner.
    pub fn create(path: impl AsRef<Path>, name: &str, me: &Identity, my_name: &str, device: &str, recovery: &Identity) -> Result<Self> {
        let folder = Folder::new(&path);
        if folder.read_meta()?.is_some() {
            return Err(Error::AlreadyInitialized);
        }
        if folder.exists() && std::fs::read_dir(folder.path())?.next().is_some() {
            return Err(Error::FolderNotEmpty);
        }
        let meta = VaultMeta { format: log::FORMAT, id: new_id(), root: me.public().detour_id() };
        folder.write_meta(&meta)?;

        let mut v = Self { folder, meta, device: device.to_string(), me: me.clone(), state: State::default() };
        let mut keys = EpochKeys::new();
        keys.insert(1, log::new_epoch_key());
        v.append_keys(1, &keys, &[me.public(), recovery.public()])?;
        v.state.keys = keys;
        let members = vec![
            member(me, my_name, Role::Owner, true, false),
            member(recovery, "Recovery key", Role::Owner, true, true),
        ];
        v.append_ops(vec![Op::Genesis { name: name.to_string(), members }])?;
        Ok(v)
    }

    /// Opens a vault whose creator key `root` the user trusts.
    pub fn open(path: impl AsRef<Path>, root: &str, me: &Identity, device: &str) -> Result<Self> {
        let folder = Folder::new(&path);
        if !folder.exists() {
            return Err(Error::Unavailable { path: path.as_ref().display().to_string() });
        }
        let meta = folder.read_meta()?.ok_or(Error::NotATeamVault)?;
        if meta.root != root {
            return Err(Error::RootChanged);
        }
        let mut v = Self { folder, meta, device: device.to_string(), me: me.clone(), state: State::default() };
        v.reload()?;
        Ok(v)
    }

    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn root(&self) -> &str {
        &self.meta.root
    }

    pub fn name(&self) -> &str {
        &self.state.name
    }

    pub fn path(&self) -> &Path {
        self.folder.path()
    }

    /// Re-reads every log from storage.
    pub fn reload(&mut self) -> Result<()> {
        if !self.folder.exists() {
            return Err(Error::Unavailable { path: self.folder.path().display().to_string() });
        }
        let root = PublicIdentity::parse(&self.meta.root)?.key_id();
        self.state = state::replay(self.folder.read_logs()?, &root, &self.me);
        Ok(())
    }

    fn my_id(&self) -> String {
        self.me.public().key_id()
    }

    fn my_role(&self) -> Option<Role> {
        self.state.members.get(&self.my_id()).map(|m| m.role)
    }

    fn i_can_reveal(&self) -> bool {
        self.state.members.get(&self.my_id()).is_some_and(|m| m.can_reveal)
    }

    fn require(&self, ok: impl Fn(Role) -> bool) -> Result<()> {
        if self.my_role().is_some_and(ok) { Ok(()) } else { Err(denied()) }
    }

    /* ---------- writing ---------- */

    fn next_body(&self, kind: Kind, epoch: u32, payload: String) -> Body {
        let (seq, prev) = match self.state.heads.get(&self.device) {
            Some((s, h)) => (s + 1, Some(h.clone())),
            None => (1, None),
        };
        Body {
            v: log::FORMAT,
            vault: self.meta.id.clone(),
            device: self.device.clone(),
            author: self.my_id(),
            seq,
            prev,
            hlc: Hlc::tick(self.state.latest),
            seen: self.state.heads.iter().map(|(d, (s, _))| (d.clone(), *s)).collect(),
            kind,
            epoch,
            payload,
        }
    }

    fn append(&mut self, body: Body) -> Result<()> {
        let bytes = log::sign(&body, &self.me)?;
        self.folder.write_log(&self.device, body.seq, &bytes)?;
        self.reload()
    }

    fn append_keys(&mut self, epoch: u32, keys: &EpochKeys, to: &[PublicIdentity]) -> Result<()> {
        let payload = log::seal_keys(keys, to)?;
        let body = self.next_body(Kind::Keys, epoch, payload);
        self.append(body)
    }

    fn append_ops(&mut self, ops: Vec<Op>) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let (epoch, key) = self.state.current_epoch().ok_or(Error::Locked)?;
        let payload = log::seal_data(&key, &serde_json::to_vec(&ops)?)?;
        let body = self.next_body(Kind::Data, epoch, payload);
        self.append(body)
    }

    /* ---------- reading ---------- */

    /// A record's latest values even if it is deleted.
    fn any_record<T: DeserializeOwned>(&self, coll: Coll, id: &str) -> Option<T> {
        self.state.records.get(&(coll, id.to_string())).and_then(|r| from_fields(id, r.values()))
    }

    fn records<T: DeserializeOwned>(&self, coll: Coll) -> Vec<T> {
        self.state
            .records
            .iter()
            .filter(|((c, _), r)| *c == coll && !r.is_deleted() && !r.fields.is_empty())
            .filter_map(|((_, id), r)| from_fields(id, r.values()))
            .collect()
    }

    pub fn host(&self, id: &str) -> Option<Host> {
        self.records::<Host>(Coll::Host).into_iter().find(|h| h.id == id)
    }

    pub fn key(&self, id: &str) -> Option<KeyEntry> {
        self.records::<KeyEntry>(Coll::Key).into_iter().find(|k| k.id == id)
    }

    pub fn data(&self) -> VaultView {
        VaultView {
            hosts: self.records::<Host>(Coll::Host).iter().map(HostView::from).collect(),
            keys: self.records::<KeyEntry>(Coll::Key).iter().map(KeyView::from).collect(),
            snippets: self.records(Coll::Snippet),
            tunnels: self.records(Coll::Tunnel),
        }
    }

    pub fn view(&self) -> TeamView {
        let me = self.my_id();
        let reveal = self.i_can_reveal();
        let members = self
            .state
            .members
            .values()
            .map(|m| MemberView {
                id: m.id.clone(),
                name: m.name.clone(),
                role: m.role,
                can_reveal: m.can_reveal,
                recovery: m.recovery,
                fingerprint: PublicIdentity::parse(&m.detour_id).map(|p| p.fingerprint()).unwrap_or_default(),
                is_you: m.id == me,
            })
            .collect();
        let mut conflicts = Vec::new();
        for ((coll, id), r) in &self.state.records {
            if r.is_deleted() {
                continue;
            }
            if let Some(d) = &r.deleted
                && !r.is_deleted()
            {
                conflicts.push(ConflictView::Deleted { coll: *coll, record: id.clone(), by: d.author.clone(), by_name: self.state.member_name(&d.author), at: d.hlc.ms });
            }
            for (field, cs) in &r.fields {
                let distinct: BTreeSet<String> = cs.iter().map(|c| c.value.to_string()).collect();
                if distinct.len() < 2 {
                    continue;
                }
                let secret = SECRET_FIELDS.contains(&field.as_str());
                let mut values: Vec<ConflictValue> = cs
                    .iter()
                    .map(|c| ConflictValue {
                        value: if secret { Value::String("••••••".into()) } else { c.value.clone() },
                        author: c.version.author.clone(),
                        author_name: self.state.member_name(&c.version.author),
                        at: c.version.hlc.ms,
                    })
                    .collect();
                values.sort_by_key(|v| v.at);
                conflicts.push(ConflictView::Field { coll: *coll, record: id.clone(), field: field.clone(), values });
            }
        }
        let activity = self
            .state
            .activity
            .iter()
            .rev()
            .take(50)
            .map(|a| ActivityView { author_name: self.state.member_name(&a.author), is_you: a.author == me, activity: a.clone() })
            .collect();
        TeamView {
            id: self.meta.id.clone(),
            name: self.state.name.clone(),
            path: self.folder.path().display().to_string(),
            joined: self.my_role().is_some(),
            removed: self.my_role().is_none() && self.state.former.contains_key(&me),
            my_role: self.my_role(),
            can_reveal: reveal,
            members,
            data: self.data(),
            conflicts,
            activity,
            problems: self.state.problems.clone(),
        }
    }

    /* ---------- editing ---------- */

    /// Applies a UI edit: writes only the fields that changed. Secrets stay as they are.
    pub fn put(&mut self, update: VaultUpdate) -> Result<()> {
        self.require(Role::can_write)?;
        let mut ops = Vec::new();

        let old_hosts: BTreeMap<String, Host> = self.records::<Host>(Coll::Host).into_iter().map(|h| (h.id.clone(), h)).collect();
        for v in &update.hosts {
            let old = old_hosts.get(&v.id);
            // Also look at deleted records, so undoing a delete keeps the password.
            let kept = self.any_record::<Host>(Coll::Host, &v.id).and_then(|h| h.password);
            let password = v.new_password.clone().filter(|p| !p.is_empty()).or(kept);
            let host = Host {
                id: v.id.clone(),
                group: v.group.clone(),
                label: v.label.clone(),
                address: v.address.clone(),
                port: v.port,
                user: v.user.clone(),
                auth: v.auth.clone(),
                tags: v.tags.clone(),
                last_connected: None,
                password: if v.auth == Auth::Password { password } else { None },
            };
            diff_into(&mut ops, Coll::Host, &v.id, old.map(|h| fields_of(h, &["lastConnected"])), fields_of(&host, &["lastConnected"]));
        }
        for id in old_hosts.keys().filter(|id| !update.hosts.iter().any(|h| &h.id == *id)) {
            ops.push(Op::Delete { coll: Coll::Host, id: id.clone() });
        }

        // Keys: rename and delete here; new keys come from generate/import.
        let old_keys: BTreeMap<String, KeyEntry> = self.records::<KeyEntry>(Coll::Key).into_iter().map(|k| (k.id.clone(), k)).collect();
        for v in &update.keys {
            if let Some(old) = old_keys.get(&v.id)
                && old.name != v.name
            {
                ops.push(Op::Set { coll: Coll::Key, id: v.id.clone(), fields: BTreeMap::from([("name".into(), Value::String(v.name.clone()))]) });
            }
        }
        for id in old_keys.keys().filter(|id| !update.keys.iter().any(|k| &k.id == *id)) {
            ops.push(Op::Delete { coll: Coll::Key, id: id.clone() });
        }
        // Restoring a deleted key (undo) re-sets it from the values the logs still hold.
        for v in update.keys.iter().filter(|v| !old_keys.contains_key(&v.id)) {
            if let Some(r) = self.state.records.get(&(Coll::Key, v.id.clone()))
                && !r.fields.is_empty()
            {
                ops.push(Op::Set { coll: Coll::Key, id: v.id.clone(), fields: r.values() });
            }
        }

        let old_snippets: BTreeMap<String, Snippet> = self.records::<Snippet>(Coll::Snippet).into_iter().map(|s| (s.id.clone(), s)).collect();
        for s in &update.snippets {
            diff_into(&mut ops, Coll::Snippet, &s.id, old_snippets.get(&s.id).map(|o| fields_of(o, &[])), fields_of(s, &[]));
        }
        for id in old_snippets.keys().filter(|id| !update.snippets.iter().any(|s| &s.id == *id)) {
            ops.push(Op::Delete { coll: Coll::Snippet, id: id.clone() });
        }

        let old_tunnels: BTreeMap<String, Tunnel> = self.records::<Tunnel>(Coll::Tunnel).into_iter().map(|t| (t.id.clone(), t)).collect();
        for t in &update.tunnels {
            diff_into(&mut ops, Coll::Tunnel, &t.id, old_tunnels.get(&t.id).map(|o| fields_of(o, &[])), fields_of(t, &[]));
        }
        for id in old_tunnels.keys().filter(|id| !update.tunnels.iter().any(|t| &t.id == *id)) {
            ops.push(Op::Delete { coll: Coll::Tunnel, id: id.clone() });
        }

        self.append_ops(ops)
    }

    pub fn set_host_password(&mut self, host_id: &str, password: Option<String>) -> Result<()> {
        self.require(Role::can_write)?;
        self.host(host_id).ok_or(Error::NotFound { what: "host".into() })?;
        let value = password.filter(|p| !p.is_empty()).map(Value::String).unwrap_or(Value::Null);
        self.append_ops(vec![Op::Set { coll: Coll::Host, id: host_id.into(), fields: BTreeMap::from([("password".into(), value)]) }])
    }

    pub fn add_key(&mut self, entry: KeyEntry) -> Result<KeyView> {
        self.require(Role::can_write)?;
        let view = KeyView::from(&entry);
        let fields = fields_of(&entry, &[]);
        self.append_ops(vec![Op::Set { coll: Coll::Key, id: entry.id.clone(), fields }])?;
        Ok(view)
    }

    pub fn replace_key(&mut self, entry: KeyEntry) -> Result<KeyView> {
        self.require(Role::can_write)?;
        let view = KeyView::from(&entry);
        let old = self.key(&entry.id).map(|k| fields_of(&k, &[]));
        let mut ops = Vec::new();
        diff_into(&mut ops, Coll::Key, &entry.id, old, fields_of(&entry, &[]));
        self.append_ops(ops)?;
        Ok(view)
    }

    /// A private key for export: only for members allowed to reveal secrets.
    pub fn revealable_key(&self, key_id: &str) -> Result<KeyEntry> {
        if !self.i_can_reveal() {
            return Err(denied());
        }
        self.key(key_id).ok_or(Error::NotFound { what: "key".into() })
    }

    pub fn can_reveal(&self) -> bool {
        self.i_can_reveal()
    }

    /// Resolves a field conflict by writing the chosen value again, having seen all others.
    pub fn resolve_field(&mut self, coll: Coll, record: &str, field: &str, pick: usize) -> Result<()> {
        self.require(Role::can_write)?;
        let r = self.state.records.get(&(coll, record.to_string())).ok_or(Error::NotFound { what: "conflict".into() })?;
        let mut cs = r.fields.get(field).cloned().unwrap_or_default();
        cs.sort_by_key(|c| c.version.hlc.ms);
        let chosen = cs.get(pick).ok_or(Error::NotFound { what: "conflict".into() })?.value.clone();
        self.append_ops(vec![Op::Set { coll, id: record.into(), fields: BTreeMap::from([(field.to_string(), chosen)]) }])
    }

    /// Resolves an edit-vs-delete conflict.
    pub fn resolve_deleted(&mut self, coll: Coll, record: &str, keep: bool) -> Result<()> {
        self.require(Role::can_write)?;
        let r = self.state.records.get(&(coll, record.to_string())).ok_or(Error::NotFound { what: "conflict".into() })?;
        let op = if keep { Op::Set { coll, id: record.into(), fields: r.values() } } else { Op::Delete { coll, id: record.into() } };
        self.append_ops(vec![op])
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        self.require(Role::can_manage)?;
        self.append_ops(vec![Op::Rename { name: name.into() }])
    }

    /* ---------- members ---------- */

    pub fn add_member(&mut self, detour_id: &str, name: &str, role: Role, can_reveal: bool) -> Result<()> {
        self.require(Role::can_manage)?;
        let who = PublicIdentity::parse(detour_id)?;
        if self.state.members.contains_key(&who.key_id()) {
            return Err(Error::AlreadyMember);
        }
        let m = Member { id: who.key_id(), detour_id: who.detour_id(), name: name.into(), role, can_reveal, recovery: false };
        self.append_ops(vec![Op::MemberPut { member: m }])?;
        // Share every epoch key so the newcomer can read the whole history.
        let (epoch, _) = self.state.current_epoch().ok_or(Error::Locked)?;
        let keys = self.state.keys.clone();
        self.append_keys(epoch, &keys, &[who])
    }

    pub fn update_member(&mut self, id: &str, role: Role, can_reveal: bool) -> Result<()> {
        self.require(Role::can_manage)?;
        let mut m = self.state.members.get(id).cloned().ok_or(Error::NotFound { what: "member".into() })?;
        m.role = role;
        m.can_reveal = can_reveal;
        self.append_ops(vec![Op::MemberPut { member: m }])
    }

    /// Removes a member and starts a new epoch they can't read.
    pub fn remove_member(&mut self, id: &str) -> Result<()> {
        self.require(Role::can_manage)?;
        if id == self.my_id() {
            return Err(denied());
        }
        self.append_ops(vec![Op::MemberRemove { id: id.into() }])?;
        if self.state.members.contains_key(id) {
            // The log rejected the removal (e.g. not allowed): no new epoch.
            return Err(denied());
        }
        let (epoch, _) = self.state.current_epoch().ok_or(Error::Locked)?;
        let mut keys = self.state.keys.clone();
        keys.insert(epoch + 1, log::new_epoch_key());
        let recipients: Vec<PublicIdentity> = self.state.members.values().filter_map(|m| PublicIdentity::parse(&m.detour_id).ok()).collect();
        self.append_keys(epoch + 1, &keys, &recipients)
    }

    /// Uses a recovery key to make `new_owner` an owner again (e.g. all owners lost their devices).
    pub fn recover(path: impl AsRef<Path>, recovery_key: &str, new_owner_detour_id: &str, name: &str) -> Result<()> {
        let recovery = Identity::from_recovery_key(recovery_key)?;
        let meta = Folder::new(&path).read_meta()?.ok_or(Error::NotATeamVault)?;
        let mut v = Self::open(&path, &meta.root, &recovery, &format!("recovery-{}", new_id()))?;
        let rec = v.state.members.get(&recovery.public().key_id()).ok_or(Error::Forbidden)?;
        if !rec.recovery {
            return Err(Error::Forbidden);
        }
        let who = PublicIdentity::parse(new_owner_detour_id)?;
        let existing = v.state.members.get(&who.key_id()).cloned();
        let m = Member { id: who.key_id(), detour_id: who.detour_id(), name: existing.map(|m| m.name).unwrap_or_else(|| name.into()), role: Role::Owner, can_reveal: true, recovery: false };
        v.append_ops(vec![Op::MemberPut { member: m }])?;
        let (epoch, _) = v.state.current_epoch().ok_or(Error::Locked)?;
        let keys = v.state.keys.clone();
        v.append_keys(epoch, &keys, &[who])
    }

    /// Generates a key in this vault.
    pub fn generate_key(&mut self, name: &str, algorithm: KeyAlgorithm, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        self.require(Role::can_write)?;
        self.add_key(keys::generate(name, algorithm, passphrase, remember)?)
    }
}

fn member(id: &Identity, name: &str, role: Role, can_reveal: bool, recovery: bool) -> Member {
    let p = id.public();
    Member { id: p.key_id(), detour_id: p.detour_id(), name: name.into(), role, can_reveal, recovery }
}

fn diff_into(ops: &mut Vec<Op>, coll: Coll, id: &str, old: Option<BTreeMap<String, Value>>, new: BTreeMap<String, Value>) {
    // Every field is always present (absent options serialize as null), so a plain comparison finds removals too.
    let changed: BTreeMap<String, Value> = match old {
        None => new,
        Some(old) => new.into_iter().filter(|(k, v)| old.get(k) != Some(v)).collect(),
    };
    if !changed.is_empty() {
        ops.push(Op::Set { coll, id: id.to_string(), fields: changed });
    }
}
