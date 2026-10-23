//! Team vaults: shared, end-to-end encrypted, no server. See `docs/teams.md`.

pub mod git;
pub mod log;
pub mod state;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::log::{Body, Folder, Head, Hlc, Key, Part, VaultMeta};
use self::state::{Activity, Coll, Cutoff, DropCache, Level, Member, Payload, Problem, Record, RecordOp, Role, Section, State, TeamOp};
use crate::identity::{Identity, PublicIdentity};
use crate::model::*;
use crate::store::VaultUpdate;
use crate::{Error, Result, keys, new_id};

/// An acknowledgement is written when this device wrote nothing for this long.
const ACK_AFTER_MS: u64 = 24 * 60 * 60 * 1000;

pub struct TeamVault {
    folder: Folder,
    meta: VaultMeta,
    device: String,
    me: Identity,
    state: State,
    drops: DropCache,
    /// Heads this person had read, merged with the storage after every read.
    known: BTreeMap<String, Head>,
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
    pub recovery: bool,
    pub fingerprint: String,
    pub is_you: bool,
}

/// One member's access to one record the reader can see.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessView {
    pub coll: Coll,
    pub record: String,
    pub member: String,
    pub level: Level,
}

/// A change to the access matrix.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessChange {
    pub record: String,
    pub member: String,
    pub level: Level,
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
pub struct ProblemView {
    /// Name of the member whose device the problem is about, if known.
    pub author_name: String,
    #[serde(flatten)]
    pub problem: Problem,
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
    pub members: Vec<MemberView>,
    pub data: VaultView,
    /// Who has which access to the records you can see.
    pub access: Vec<AccessView>,
    pub conflicts: Vec<ConflictView>,
    pub activity: Vec<ActivityView>,
    pub problems: Vec<ProblemView>,
    /// For a team in a Git repository: how mirroring it goes. Filled in by the store.
    pub remote: Option<RemoteView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteView {
    pub url: String,
    /// Local changes the repository doesn't have yet.
    pub pending: bool,
    /// Why the last attempt to reach the repository failed (a serialized `Error`).
    pub error: Option<Value>,
    /// Last time the repository was reached.
    pub synced_at: Option<i64>,
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

/// Splits a record's fields into what goes under the meta key and under the secret key.
fn split(coll: Coll, fields: BTreeMap<String, Value>) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    fields.into_iter().partition(|(k, _)| !coll.is_secret_field(k))
}

/// Everything one entry will carry, assembled before it is encrypted and signed.
#[derive(Default)]
struct Draft {
    team: Vec<TeamOp>,
    records: Vec<(String, Section, Vec<RecordOp>)>,
    /// Member id -> (identity, keys to hand over).
    drops: BTreeMap<String, (PublicIdentity, Vec<Key>)>,
    /// Keys created in this entry, used instead of the current ones.
    new_team_key: Option<Key>,
    new_keys: HashMap<(String, Section), Key>,
}

impl Draft {
    fn rec(&mut self, rid: &str, section: Section, op: RecordOp) {
        match self.records.iter_mut().find(|(r, s, _)| r == rid && *s == section) {
            Some((_, _, ops)) => ops.push(op),
            None => self.records.push((rid.to_string(), section, vec![op])),
        }
    }

    fn give(&mut self, to: &Member, keys: impl IntoIterator<Item = Key>) {
        let Ok(p) = PublicIdentity::parse(&to.detour_id) else { return };
        let slot = &mut self.drops.entry(to.id.clone()).or_insert_with(|| (p, Vec::new())).1;
        for k in keys {
            if !slot.contains(&k) {
                slot.push(k);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.team.is_empty() && self.records.is_empty() && self.drops.is_empty()
    }
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
        if folder.exists() && std::fs::read_dir(folder.path())?.next().is_some() {
            return Err(if folder.path().join("vault.json").exists() { Error::AlreadyInitialized } else { Error::FolderNotEmpty });
        }
        let meta = VaultMeta { format: log::FORMAT, id: new_id(), root: me.public().detour_id() };
        folder.write_meta(&meta)?;

        let mut v = Self { folder, meta, device: device.to_string(), me: me.clone(), state: State::default(), drops: DropCache::new(), known: BTreeMap::new() };
        let members = vec![member(me, my_name, Role::Owner, false), member(recovery, "Recovery key", Role::Owner, true)];
        let key = log::new_key();
        let mut d = Draft { new_team_key: Some(key), ..Default::default() };
        d.team.push(TeamOp::Epoch { epoch: 1, kid: log::key_id(&key), holders: members.iter().map(|m| m.id.clone()).collect() });
        d.team.push(TeamOp::Genesis { name: name.to_string(), members: members.clone() });
        for m in &members {
            d.give(m, [key]);
        }
        v.commit(d)?;
        Ok(v)
    }

    /// Opens a vault whose creator key `root` the user trusts. `known` are the heads read before.
    pub fn open(path: impl AsRef<Path>, root: &str, me: &Identity, device: &str, known: BTreeMap<String, Head>) -> Result<Self> {
        let folder = Folder::new(&path);
        if !folder.exists() {
            return Err(Error::Unavailable { path: path.as_ref().display().to_string() });
        }
        let meta = folder.read_meta()?.ok_or(Error::NotATeamVault)?;
        if meta.root != root {
            return Err(Error::RootChanged);
        }
        let mut v = Self { folder, meta, device: device.to_string(), me: me.clone(), state: State::default(), drops: DropCache::new(), known };
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

    /// Heads of every device this person has read, to keep in the personal vault.
    pub fn known_heads(&self) -> &BTreeMap<String, Head> {
        &self.known
    }

    /// Re-reads and re-checks everything in storage.
    pub fn reload(&mut self) -> Result<()> {
        if !self.folder.exists() {
            return Err(Error::Unavailable { path: self.folder.path().display().to_string() });
        }
        let root = PublicIdentity::parse(&self.meta.root)?.key_id();
        let verified = state::verify_all(self.folder.scan()?, &self.known);
        self.state = state::replay(&verified, &root, &self.me, &mut self.drops);
        // Never forget a head: a storage showing less later is a rollback.
        for (d, h) in &self.state.heads {
            if self.known.get(d).is_none_or(|k| k.seq < h.seq) {
                self.known.insert(d.clone(), h.clone());
            }
        }
        Ok(())
    }

    fn my_id(&self) -> String {
        self.me.public().key_id()
    }

    fn my_role(&self) -> Option<Role> {
        self.state.role_of(&self.my_id())
    }

    /// Your access to a record.
    pub fn level(&self, record: &str) -> Level {
        self.state.level_of(&self.my_id(), record)
    }

    fn require_role(&self, min: Role) -> Result<()> {
        if self.my_role().is_some_and(|r| r.at_least(min)) { Ok(()) } else { Err(denied()) }
    }

    fn require_level(&self, record: &str, min: Level) -> Result<()> {
        if self.level(record) >= min { Ok(()) } else { Err(denied()) }
    }

    fn me_member(&self) -> Result<Member> {
        self.state.members.get(&self.my_id()).cloned().ok_or_else(denied)
    }

    /* ---------- writing ---------- */

    fn key_of(&self, kid: &str) -> Result<Key> {
        self.state.keys.get(kid).copied().ok_or_else(denied)
    }

    fn team_key(&self) -> Result<Key> {
        self.key_of(&self.state.epoch.as_ref().ok_or(Error::Locked)?.kid)
    }

    fn section_key(&self, rid: &str, s: Section) -> Result<Key> {
        let g = self.state.records.get(rid).and_then(|r| r.gens.get(&s)).ok_or_else(denied)?;
        self.key_of(&g.kid)
    }

    /// Every key of a record this member holds, for sections up to `level`.
    fn record_keys_up_to(&self, rid: &str, level: Level) -> Vec<Key> {
        let Some(r) = self.state.records.get(rid) else { return vec![] };
        r.history.iter().filter(|(s, _)| level >= s.level()).flat_map(|(_, kids)| kids.iter().filter_map(|k| self.state.keys.get(k).copied())).collect()
    }

    /// Holds the current key of every section up to `level`: a grant hands them over.
    fn holds_current(&self, rid: &str, level: Level) -> bool {
        let Some(r) = self.state.records.get(rid) else { return false };
        r.gens.iter().filter(|(s, _)| level >= s.level()).all(|(_, g)| self.state.keys.contains_key(&g.kid))
    }

    fn team_keys(&self) -> Vec<Key> {
        self.state.epoch_history.iter().filter_map(|k| self.state.keys.get(k).copied()).collect()
    }

    /// Encrypts, signs and writes one entry, then re-reads the vault.
    fn commit(&mut self, d: Draft) -> Result<()> {
        let mut parts = Vec::new();
        for (to, keys) in d.drops.values() {
            parts.push(log::seal_drop(keys, to)?);
        }
        if !d.team.is_empty() {
            let key = match d.new_team_key {
                Some(k) => k,
                None => self.team_key()?,
            };
            parts.push(log::seal_data(&key, &serde_json::to_vec(&Payload::Team { ops: d.team })?)?);
        }
        for (rid, section, ops) in d.records {
            let key = match d.new_keys.get(&(rid.clone(), section)) {
                Some(k) => *k,
                None => self.section_key(&rid, section)?,
            };
            parts.push(log::seal_data(&key, &serde_json::to_vec(&Payload::Record { record: rid, section, ops })?)?);
        }
        self.append(parts)
    }

    fn append(&mut self, parts: Vec<Part>) -> Result<()> {
        let (seq, prev) = match self.state.heads.get(&self.device) {
            Some(h) => (h.seq + 1, Some(h.hash.clone())),
            None => (1, None),
        };
        let body = Body {
            v: log::FORMAT,
            vault: self.meta.id.clone(),
            device: self.device.clone(),
            author: self.my_id(),
            seq,
            prev,
            hlc: Hlc::tick(self.state.latest),
            seen: self.state.heads.clone(),
            parts,
        };
        let bytes = log::sign(&body, &self.me)?;
        self.folder.write_log(&self.device, seq, &bytes)?;
        self.reload()
    }

    /// Who gets what when a record is created: owners, admins and the recovery key may edit, members see it.
    fn default_level(m: &Member, coll: Coll) -> Level {
        if m.recovery || m.role.can_manage() { Level::Edit } else { Level::View.normalize(coll) }
    }

    fn create_record(&self, d: &mut Draft, coll: Coll, rid: &str, fields: BTreeMap<String, Value>) {
        let me = self.my_id();
        let (meta_fields, secret_fields) = split(coll, fields);
        let km = log::new_key();
        let ks = log::new_key();
        d.new_keys.insert((rid.to_string(), Section::Meta), km);
        d.new_keys.insert((rid.to_string(), Section::Secret), ks);

        let levels: Vec<(Member, Level)> = self.state.members.values().map(|m| (m.clone(), if m.id == me { Level::Edit } else { Self::default_level(m, coll) })).collect();
        let holders = |min: Level| levels.iter().filter(|(_, l)| *l >= min).map(|(m, _)| m.id.clone()).collect::<Vec<_>>();
        d.rec(rid, Section::Meta, RecordOp::Set { coll, fields: meta_fields });
        d.rec(rid, Section::Meta, RecordOp::Rotate { section: Section::Meta, generation: 1, kid: log::key_id(&km), holders: holders(Level::View) });
        if coll.has_secret() {
            d.rec(rid, Section::Meta, RecordOp::Rotate { section: Section::Secret, generation: 1, kid: log::key_id(&ks), holders: holders(Level::Secret) });
        }
        for (m, level) in &levels {
            if m.id != me {
                d.rec(rid, Section::Meta, RecordOp::Grant { member: m.id.clone(), level: *level, cutoff: Cutoff::new() });
            }
            let keys = if *level >= Level::Secret && coll.has_secret() { vec![km, ks] } else { vec![km] };
            d.give(m, keys);
        }
        if coll.has_secret() && !secret_fields.is_empty() {
            d.rec(rid, Section::Secret, RecordOp::Set { coll, fields: secret_fields });
        }
    }

    /* ---------- reading ---------- */

    fn visible(&self, rid: &str, r: &Record) -> bool {
        r.coll.is_some() && !r.fields.is_empty() && self.level(rid) >= Level::View
    }

    /// A record's latest values even if it is deleted.
    fn any_record<T: DeserializeOwned>(&self, rid: &str) -> Option<T> {
        self.state.records.get(rid).filter(|r| self.visible(rid, r)).and_then(|r| from_fields(rid, r.values()))
    }

    fn records<T: DeserializeOwned>(&self, coll: Coll) -> Vec<T> {
        self.state
            .records
            .iter()
            .filter(|(id, r)| r.coll == Some(coll) && !r.is_deleted() && self.visible(id, r))
            .filter_map(|(id, r)| from_fields(id, r.values()))
            .collect()
    }

    /// A host with its shared password when this member may have it.
    pub fn host(&self, id: &str) -> Option<Host> {
        self.records::<Host>(Coll::Host).into_iter().find(|h| h.id == id)
    }

    /// A key; its private part is empty unless this member has Secret access to it.
    pub fn key(&self, id: &str) -> Option<KeyEntry> {
        self.records::<KeyEntry>(Coll::Key).into_iter().find(|k| k.id == id)
    }

    /// The private key, for connecting or exporting.
    pub fn secret_key(&self, id: &str) -> Result<KeyEntry> {
        self.require_level(id, Level::Secret)?;
        self.key(id).filter(|k| !k.private_key.is_empty()).ok_or_else(denied)
    }

    /// The shared password of a host.
    pub fn host_password(&self, id: &str) -> Result<Option<String>> {
        self.require_level(id, Level::Secret)?;
        Ok(self.host(id).ok_or(Error::NotFound { what: "host".into() })?.password)
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
        let members = self
            .state
            .members
            .values()
            .map(|m| MemberView {
                id: m.id.clone(),
                name: m.name.clone(),
                role: m.role,
                recovery: m.recovery,
                fingerprint: PublicIdentity::parse(&m.detour_id).map(|p| p.fingerprint()).unwrap_or_default(),
                is_you: m.id == me,
            })
            .collect();
        let shown = |id: &str, r: &Record| self.visible(id, r) && !r.is_deleted();
        let mut access = Vec::new();
        let mut conflicts = Vec::new();
        for (id, r) in &self.state.records {
            if !shown(id, r) {
                continue;
            }
            let coll = r.coll.unwrap();
            for (m, level) in &r.acl {
                if self.state.members.contains_key(m) {
                    access.push(AccessView { coll, record: id.clone(), member: m.clone(), level: *level });
                }
            }
            if let Some(d) = &r.deleted {
                conflicts.push(ConflictView::Deleted { coll, record: id.clone(), by: d.author.clone(), by_name: self.state.member_name(&d.author), at: d.hlc.ms });
            }
            for (field, cs) in &r.fields {
                let distinct: BTreeSet<String> = cs.iter().map(|c| c.value.to_string()).collect();
                if distinct.len() < 2 {
                    continue;
                }
                let secret = coll.is_secret_field(field);
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
                conflicts.push(ConflictView::Field { coll, record: id.clone(), field: field.clone(), values });
            }
        }
        let about_visible = |a: &Activity| {
            use state::ActivityKind::*;
            match &a.what {
                Added { id, .. } | Changed { id, .. } | SecretChanged { id, .. } | Deleted { id, .. } | AccessChanged { id, .. } => self.level(id) >= Level::View,
                _ => true,
            }
        };
        let activity = self
            .state
            .activity
            .iter()
            .rev()
            .filter(|a| about_visible(a))
            .take(50)
            .map(|a| ActivityView { author_name: self.state.member_name(&a.author), is_you: a.author == me, activity: a.clone() })
            .collect();
        let problems = self
            .state
            .problems
            .iter()
            .map(|p| {
                let author = match p {
                    Problem::Rejected { author, .. } | Problem::Missing { author, .. } | Problem::Forked { author, .. } | Problem::RolledBack { author, .. } => author.as_str(),
                    Problem::UnknownFile { .. } => "",
                };
                ProblemView { author_name: self.state.member_name(author), problem: p.clone() }
            })
            .collect();
        TeamView {
            id: self.meta.id.clone(),
            name: self.state.name.clone(),
            path: self.folder.path().display().to_string(),
            joined: self.my_role().is_some(),
            removed: self.my_role().is_none() && self.state.former.contains_key(&me),
            my_role: self.my_role(),
            members,
            data: self.data(),
            access,
            conflicts,
            activity,
            problems,
            remote: None,
        }
    }

    /* ---------- editing ---------- */

    /// Writes a record the UI changed: creates it, or writes the fields that changed.
    fn put_record(&self, d: &mut Draft, coll: Coll, rid: &str, old: Option<BTreeMap<String, Value>>, new: BTreeMap<String, Value>) -> Result<()> {
        let exists = self.state.records.get(rid).is_some_and(|r| r.coll.is_some());
        if !exists {
            self.require_role(Role::Member)?;
            self.create_record(d, coll, rid, new);
            return Ok(());
        }
        // Every field is always present (absent options serialize as null), so a plain comparison finds removals too.
        let changed: BTreeMap<String, Value> = match old {
            None => new,
            Some(old) => new.into_iter().filter(|(k, v)| old.get(k) != Some(v)).collect(),
        };
        if changed.is_empty() {
            return Ok(());
        }
        self.require_level(rid, Level::Edit)?;
        let (meta, secret) = split(coll, changed);
        if !meta.is_empty() {
            d.rec(rid, Section::Meta, RecordOp::Set { coll, fields: meta });
        }
        if !secret.is_empty() {
            d.rec(rid, Section::Secret, RecordOp::Set { coll, fields: secret });
        }
        Ok(())
    }

    fn delete_record(&self, d: &mut Draft, rid: &str) -> Result<()> {
        self.require_level(rid, Level::Edit)?;
        d.rec(rid, Section::Meta, RecordOp::Delete);
        Ok(())
    }

    /// Applies a UI edit of the records this member can see. Secrets the UI never had stay as they are.
    pub fn put(&mut self, update: VaultUpdate) -> Result<()> {
        self.require_role(Role::Member)?;
        let mut d = Draft::default();

        let old_hosts: BTreeMap<String, Host> = self.records::<Host>(Coll::Host).into_iter().map(|h| (h.id.clone(), h)).collect();
        for v in &update.hosts {
            let old = old_hosts.get(&v.id);
            // Also look at deleted records, so undoing a delete keeps the password.
            let kept = self.any_record::<Host>(&v.id).and_then(|h| h.password);
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
            let mut new = fields_of(&host, &["lastConnected"]);
            // Members without the secret don't know the password: never overwrite it blindly.
            if self.level(&v.id) < Level::Secret && self.state.records.contains_key(&v.id) {
                new.remove("password");
            }
            let old_fields = old.map(|h| {
                let mut f = fields_of(h, &["lastConnected"]);
                if !new.contains_key("password") {
                    f.remove("password");
                }
                f
            });
            self.put_record(&mut d, Coll::Host, &v.id, old_fields, new)?;
        }
        for id in old_hosts.keys().filter(|id| !update.hosts.iter().any(|h| &h.id == *id)) {
            self.delete_record(&mut d, id)?;
        }

        // Keys: rename and delete here; new keys come from generate/import.
        let old_keys: BTreeMap<String, KeyEntry> = self.records::<KeyEntry>(Coll::Key).into_iter().map(|k| (k.id.clone(), k)).collect();
        for v in &update.keys {
            match old_keys.get(&v.id) {
                Some(old) if old.name != v.name => {
                    self.require_level(&v.id, Level::Edit)?;
                    d.rec(&v.id, Section::Meta, RecordOp::Set { coll: Coll::Key, fields: BTreeMap::from([("name".into(), Value::String(v.name.clone()))]) });
                }
                Some(_) => {}
                // Restoring a deleted key (undo) re-sets it from the values the logs still hold.
                None => {
                    if let Some(r) = self.state.records.get(&v.id).filter(|r| r.coll == Some(Coll::Key) && self.visible(&v.id, r)) {
                        self.require_level(&v.id, Level::Edit)?;
                        let (meta, _) = split(Coll::Key, r.values());
                        d.rec(&v.id, Section::Meta, RecordOp::Set { coll: Coll::Key, fields: meta });
                    }
                }
            }
        }
        for id in old_keys.keys().filter(|id| !update.keys.iter().any(|k| &k.id == *id)) {
            self.delete_record(&mut d, id)?;
        }

        let old_snippets: BTreeMap<String, Snippet> = self.records::<Snippet>(Coll::Snippet).into_iter().map(|s| (s.id.clone(), s)).collect();
        for s in &update.snippets {
            self.put_record(&mut d, Coll::Snippet, &s.id, old_snippets.get(&s.id).map(|o| fields_of(o, &[])), fields_of(s, &[]))?;
        }
        for id in old_snippets.keys().filter(|id| !update.snippets.iter().any(|s| &s.id == *id)) {
            self.delete_record(&mut d, id)?;
        }

        let old_tunnels: BTreeMap<String, Tunnel> = self.records::<Tunnel>(Coll::Tunnel).into_iter().map(|t| (t.id.clone(), t)).collect();
        for t in &update.tunnels {
            self.put_record(&mut d, Coll::Tunnel, &t.id, old_tunnels.get(&t.id).map(|o| fields_of(o, &[])), fields_of(t, &[]))?;
        }
        for id in old_tunnels.keys().filter(|id| !update.tunnels.iter().any(|t| &t.id == *id)) {
            self.delete_record(&mut d, id)?;
        }

        if d.is_empty() {
            return Ok(());
        }
        self.write(d)
    }

    /// Commits after rotating keys that people who lost access still hold.
    fn write(&mut self, mut d: Draft) -> Result<()> {
        let touched: BTreeSet<(String, Section)> = d.records.iter().map(|(r, s, _)| (r.clone(), *s)).collect();
        for (rid, s) in touched {
            if !d.new_keys.contains_key(&(rid.clone(), s)) && self.state.section_stale(&rid, s) {
                self.rotate(&mut d, &rid, s)?;
            }
        }
        if !d.team.is_empty() && d.new_team_key.is_none() && self.state.team_key_stale() && self.my_role().is_some_and(Role::can_manage) {
            self.rotate_team(&mut d)?;
        }
        self.commit(d)
    }

    /// Starts a new key generation for a record section, handed to everyone who has that access now.
    fn rotate(&self, d: &mut Draft, rid: &str, s: Section) -> Result<()> {
        let generation = self.state.records.get(rid).and_then(|r| r.gens.get(&s)).map_or(1, |g| g.generation + 1);
        let key = log::new_key();
        let holders: Vec<Member> = self.state.members.values().filter(|m| self.state.level_of(&m.id, rid) >= s.level()).cloned().collect();
        for m in &holders {
            d.give(m, [key]);
        }
        d.new_keys.insert((rid.to_string(), s), key);
        let op = RecordOp::Rotate { section: s, generation, kid: log::key_id(&key), holders: holders.iter().map(|m| m.id.clone()).collect() };
        // The announcement travels in the meta section: under the new key when that is the one rotating.
        if s == Section::Secret && !d.new_keys.contains_key(&(rid.to_string(), Section::Meta)) {
            self.section_key(rid, Section::Meta)?;
        }
        d.records.insert(0, (rid.to_string(), Section::Meta, vec![op]));
        Ok(())
    }

    fn rotate_team(&self, d: &mut Draft) -> Result<()> {
        let epoch = self.state.epoch.as_ref().map_or(1, |g| g.generation + 1);
        let key = log::new_key();
        let members: Vec<Member> = self.state.members.values().cloned().collect();
        for m in &members {
            d.give(m, [key]);
        }
        d.new_team_key = Some(key);
        d.team.insert(0, TeamOp::Epoch { epoch, kid: log::key_id(&key), holders: members.iter().map(|m| m.id.clone()).collect() });
        Ok(())
    }

    /// Housekeeping after a read: rotate keys people who lost access still hold,
    /// hand current keys to members who missed them, acknowledge others' entries.
    /// Returns true if an entry was written.
    pub fn maintain(&mut self) -> Result<bool> {
        let me = self.my_id();
        let Some(role) = self.my_role() else { return Ok(false) };
        let mut d = Draft::default();

        if role.can_manage() && self.state.team_key_stale() {
            self.rotate_team(&mut d)?;
        } else if let Some(g) = &self.state.epoch
            && let Ok(key) = self.team_key()
        {
            for m in self.state.members.values().filter(|m| !g.holders.contains(&m.id)) {
                d.give(m, [key]);
                d.team.push(TeamOp::Share { member: m.id.clone(), epoch: g.generation });
            }
        }

        for (rid, r) in &self.state.records {
            let Some(_) = r.coll else { continue };
            let mine = self.level(rid);
            for (s, g) in &r.gens {
                if mine < s.level() {
                    continue;
                }
                if self.state.section_stale(rid, *s) {
                    if mine >= Level::Edit || role.can_manage() {
                        self.rotate(&mut d, rid, *s)?;
                    }
                    continue;
                }
                let Ok(key) = self.key_of(&g.kid) else { continue };
                for m in self.state.members.values().filter(|m| !g.holders.contains(&m.id) && self.state.level_of(&m.id, rid) >= s.level()) {
                    d.give(m, [key]);
                    d.rec(rid, Section::Meta, RecordOp::Share { member: m.id.clone(), section: *s, generation: g.generation });
                }
            }
        }

        if !d.is_empty() {
            self.commit(d)?;
            return Ok(true);
        }

        // Acknowledge what others wrote, so the storage can't drop it unnoticed.
        let mine = self.state.last_at.get(&self.device).copied();
        let newer_from_others = self.state.last_at.iter().any(|(dev, at)| dev != &self.device && mine.is_none_or(|m| *at > m));
        let quiet = mine.is_none_or(|m| Hlc::tick(Hlc::default()).ms.saturating_sub(m.ms) >= ACK_AFTER_MS);
        if newer_from_others && quiet && self.state.members.contains_key(&me) {
            self.append(Vec::new())?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn set_host_password(&mut self, host_id: &str, password: Option<String>) -> Result<()> {
        self.require_level(host_id, Level::Edit)?;
        self.host(host_id).ok_or(Error::NotFound { what: "host".into() })?;
        let value = password.filter(|p| !p.is_empty()).map(Value::String).unwrap_or(Value::Null);
        let mut d = Draft::default();
        d.rec(host_id, Section::Secret, RecordOp::Set { coll: Coll::Host, fields: BTreeMap::from([("password".into(), value)]) });
        self.write(d)
    }

    pub fn add_key(&mut self, entry: KeyEntry) -> Result<KeyView> {
        self.require_role(Role::Member)?;
        let view = KeyView::from(&entry);
        let mut d = Draft::default();
        self.create_record(&mut d, Coll::Key, &entry.id.clone(), fields_of(&entry, &[]));
        self.commit(d)?;
        Ok(view)
    }

    pub fn replace_key(&mut self, entry: KeyEntry) -> Result<KeyView> {
        self.require_level(&entry.id, Level::Edit)?;
        let view = KeyView::from(&entry);
        let old = self.key(&entry.id).map(|k| fields_of(&k, &[]));
        let mut d = Draft::default();
        self.put_record(&mut d, Coll::Key, &entry.id, old, fields_of(&entry, &[]))?;
        self.write(d)?;
        Ok(view)
    }

    /// Resolves a field conflict by writing the chosen value again, having seen all others.
    pub fn resolve_field(&mut self, coll: Coll, record: &str, field: &str, pick: usize) -> Result<()> {
        self.require_level(record, Level::Edit)?;
        let r = self.state.records.get(record).ok_or(Error::NotFound { what: "conflict".into() })?;
        let mut cs = r.fields.get(field).cloned().unwrap_or_default();
        cs.sort_by_key(|c| c.version.hlc.ms);
        let chosen = cs.get(pick).ok_or(Error::NotFound { what: "conflict".into() })?.value.clone();
        let section = if coll.is_secret_field(field) { Section::Secret } else { Section::Meta };
        let mut d = Draft::default();
        d.rec(record, section, RecordOp::Set { coll, fields: BTreeMap::from([(field.to_string(), chosen)]) });
        self.write(d)
    }

    /// Resolves an edit-vs-delete conflict.
    pub fn resolve_deleted(&mut self, coll: Coll, record: &str, keep: bool) -> Result<()> {
        self.require_level(record, Level::Edit)?;
        let r = self.state.records.get(record).ok_or(Error::NotFound { what: "conflict".into() })?;
        let mut d = Draft::default();
        let op = if keep { RecordOp::Set { coll, fields: split(coll, r.values()).0 } } else { RecordOp::Delete };
        d.rec(record, Section::Meta, op);
        self.write(d)
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        self.require_role(Role::Admin)?;
        let mut d = Draft::default();
        d.team.push(TeamOp::Rename { name: name.into() });
        self.write(d)
    }

    /* ---------- members and access ---------- */

    /// Adds a member with `access` to every record this admin holds at least that level of.
    pub fn add_member(&mut self, detour_id: &str, name: &str, role: Role, access: Level) -> Result<()> {
        self.require_role(if role.can_manage() { Role::Owner } else { Role::Admin })?;
        let who = PublicIdentity::parse(detour_id)?;
        if self.state.members.contains_key(&who.key_id()) {
            return Err(Error::AlreadyMember);
        }
        let m = Member { id: who.key_id(), detour_id: who.detour_id(), name: name.into(), role, recovery: false };
        let mut d = Draft::default();
        d.team.push(TeamOp::MemberPut { member: m.clone(), cutoff: Cutoff::new() });
        // Every team key, so the newcomer can read the whole member history.
        d.give(&m, self.team_keys());
        if access > Level::None {
            for (rid, r) in &self.state.records {
                let Some(coll) = r.coll else { continue };
                let level = access.normalize(coll);
                if self.level(rid) >= level && self.holds_current(rid, level) {
                    d.rec(rid, Section::Meta, RecordOp::Grant { member: m.id.clone(), level, cutoff: Cutoff::new() });
                    d.give(&m, self.record_keys_up_to(rid, level));
                }
            }
        }
        self.write(d)
    }

    pub fn update_member(&mut self, id: &str, role: Role) -> Result<()> {
        let mut m = self.state.members.get(id).cloned().ok_or(Error::NotFound { what: "member".into() })?;
        self.require_role(if role.can_manage() || m.role.can_manage() { Role::Owner } else { Role::Admin })?;
        m.role = role;
        let mut d = Draft::default();
        d.team.push(TeamOp::MemberPut { member: m, cutoff: self.state.cutoff_for(id) });
        self.write(d)?;
        self.maintain().map(|_| ())
    }

    /// Removes a member, then gives everyone else new keys the removed person never sees.
    pub fn remove_member(&mut self, id: &str) -> Result<()> {
        let target = self.state.members.get(id).cloned().ok_or(Error::NotFound { what: "member".into() })?;
        self.require_role(if target.role.can_manage() { Role::Owner } else { Role::Admin })?;
        if id == self.my_id() {
            return Err(denied());
        }
        let mut d = Draft::default();
        d.team.push(TeamOp::MemberRemove { id: id.into(), cutoff: self.state.cutoff_for(id) });
        self.commit(d)?;
        if self.state.members.contains_key(id) {
            // The log rejected the removal.
            return Err(denied());
        }
        self.maintain().map(|_| ())
    }

    /// Sets access levels. A host's Secret also brings Secret on the team key it uses.
    pub fn set_access(&mut self, changes: &[AccessChange]) -> Result<()> {
        self.require_role(Role::Admin)?;
        let mut wanted: BTreeMap<(String, String), Level> = BTreeMap::new();
        for c in changes {
            let coll = self.state.records.get(&c.record).and_then(|r| r.coll).ok_or(Error::NotFound { what: "record".into() })?;
            wanted.insert((c.record.clone(), c.member.clone()), c.level.normalize(coll));
        }
        // Keys follow the hosts that use them.
        let hosts = self.records::<Host>(Coll::Host);
        let level_after = |w: &BTreeMap<(String, String), Level>, rid: &str, m: &str| w.get(&(rid.to_string(), m.to_string())).copied().unwrap_or_else(|| self.state.level_of(m, rid));
        for c in changes {
            let Some(Auth::Key { key_id }) = hosts.iter().find(|h| h.id == c.record).map(|h| h.auth.clone()) else { continue };
            if self.state.records.get(&key_id).and_then(|r| r.coll) != Some(Coll::Key) {
                continue;
            }
            let key_now = level_after(&wanted, &key_id, &c.member);
            let need = hosts
                .iter()
                .filter(|h| h.auth == Auth::Key { key_id: key_id.clone() })
                .map(|h| level_after(&wanted, &h.id, &c.member).min(Level::Secret))
                .max()
                .unwrap_or(Level::None);
            let target = if key_now < need { need } else if key_now == Level::Secret && need < Level::Secret { need.max(Level::View) } else { key_now };
            if target != key_now && self.level(&key_id) >= target.max(key_now) {
                wanted.insert((key_id.clone(), c.member.clone()), target);
            }
        }

        let mut d = Draft::default();
        for ((rid, mid), level) in &wanted {
            let current = self.state.level_of(mid, rid);
            if *level == current {
                continue;
            }
            if self.level(rid) < (*level).max(current) || !self.holds_current(rid, *level) {
                return Err(denied());
            }
            let m = self.state.members.get(mid).cloned().ok_or(Error::NotFound { what: "member".into() })?;
            let cutoff = if *level < current { self.state.cutoff_for(mid) } else { Cutoff::new() };
            d.rec(rid, Section::Meta, RecordOp::Grant { member: mid.clone(), level: *level, cutoff });
            if *level > current {
                d.give(&m, self.record_keys_up_to(rid, *level));
            }
        }
        if d.is_empty() {
            return Ok(());
        }
        self.commit(d)?;
        self.maintain().map(|_| ())
    }

    /// Uses a recovery key to make `new_owner` an owner again (e.g. all owners lost their devices).
    pub fn recover(path: impl AsRef<Path>, recovery_key: &str, new_owner_detour_id: &str, name: &str) -> Result<()> {
        let recovery = Identity::from_recovery_key(recovery_key)?;
        let meta = Folder::new(&path).read_meta()?.ok_or(Error::NotATeamVault)?;
        let mut v = Self::open(&path, &meta.root, &recovery, &format!("recovery-{}", new_id()), BTreeMap::new())?;
        if !v.me_member()?.recovery {
            return Err(Error::Forbidden);
        }
        let who = PublicIdentity::parse(new_owner_detour_id)?;
        let existing = v.state.members.get(&who.key_id()).cloned();
        let m = Member { id: who.key_id(), detour_id: who.detour_id(), name: existing.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| name.into()), role: Role::Owner, recovery: false };
        let mut d = Draft::default();
        d.team.push(TeamOp::MemberPut { member: m.clone(), cutoff: Cutoff::new() });
        d.give(&m, v.team_keys());
        for (rid, r) in &v.state.records {
            if r.coll.is_some() && v.level(rid) >= Level::Edit {
                d.rec(rid, Section::Meta, RecordOp::Grant { member: m.id.clone(), level: Level::Edit, cutoff: Cutoff::new() });
                d.give(&m, v.record_keys_up_to(rid, Level::Edit));
            }
        }
        v.write(d)
    }

    /// Generates a key in this vault.
    pub fn generate_key(&mut self, name: &str, algorithm: KeyAlgorithm, passphrase: Option<&str>, remember: bool) -> Result<KeyView> {
        self.require_role(Role::Member)?;
        self.add_key(keys::generate(name, algorithm, passphrase, remember)?)
    }
}

fn member(id: &Identity, name: &str, role: Role, recovery: bool) -> Member {
    let p = id.public();
    Member { id: p.key_id(), detour_id: p.detour_id(), name: name.into(), role, recovery }
}
