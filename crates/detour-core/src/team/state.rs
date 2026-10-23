//! Rebuilding a team vault's state from its logs.
//!
//! Verification first: signatures, each device's chain, cross references
//! between devices, the heads this client already knows, and stray files.
//! Then replay: entries in a total order (HLC, author, device, seq), every
//! change checked against its author's role and access at that point. Record
//! fields merge last-writer-wins; when two writers changed the same field
//! without having seen each other's change, both values are kept as a conflict
//! until someone resolves it.
//!
//! A change that takes rights away carries a cutoff: how far the person making
//! it had read each of the target's devices. Entries of the target beyond the
//! cutoff are judged with the reduced rights wherever their timestamp places
//! them; replay is repeated without the ones that fail.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::log::{self, Entry, Head, Hlc, Key, Part, Scan};
use crate::identity::{Identity, PublicIdentity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coll {
    Host,
    Key,
    Snippet,
    Tunnel,
}

impl Coll {
    pub fn has_secret(self) -> bool {
        matches!(self, Coll::Host | Coll::Key)
    }

    /// Fields kept under the record's secret key.
    pub fn secret_fields(self) -> &'static [&'static str] {
        match self {
            Coll::Host => &["password"],
            Coll::Key => &["privateKey", "passphrase"],
            _ => &[],
        }
    }

    pub fn is_secret_field(self, field: &str) -> bool {
        self.secret_fields().contains(&field)
    }

    fn name_field(self) -> &'static str {
        if self == Coll::Host { "label" } else { "name" }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    fn rank(self) -> u8 {
        match self {
            Role::Owner => 2,
            Role::Admin => 1,
            Role::Member => 0,
        }
    }
    pub fn can_manage(self) -> bool {
        self.rank() >= 1
    }
    pub fn at_least(self, other: Role) -> bool {
        self.rank() >= other.rank()
    }
}

/// Access to one record. Each level includes the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    None,
    /// Metadata key: name, address, login. Connect with own credentials.
    View,
    /// Secret key too: the shared password or private key.
    Secret,
    /// Same keys as Secret, and changes are accepted.
    Edit,
}

impl Level {
    /// Snippets and tunnels have no secret part.
    pub fn normalize(self, coll: Coll) -> Level {
        if self == Level::Secret && !coll.has_secret() { Level::View } else { self }
    }
}

/// The two keys of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    Meta,
    Secret,
}

impl Section {
    pub fn level(self) -> Level {
        match self {
            Section::Meta => Level::View,
            Section::Secret => Level::Secret,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    /// Signing key id.
    pub id: String,
    pub detour_id: String,
    pub name: String,
    pub role: Role,
    #[serde(default)]
    pub recovery: bool,
}

/// Device -> last sequence number the author of a change had read from it.
pub type Cutoff = BTreeMap<String, u64>;

/// Plaintext of a data part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Payload {
    Team { ops: Vec<TeamOp> },
    Record { record: String, section: Section, ops: Vec<RecordOp> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TeamOp {
    Genesis { name: String, members: Vec<Member> },
    /// A new team key generation, handed to `holders`.
    Epoch { epoch: u32, kid: String, holders: Vec<String> },
    Rename { name: String },
    MemberPut { member: Member, cutoff: Cutoff },
    MemberRemove { id: String, cutoff: Cutoff },
    /// The current team key was handed to a member who missed it.
    Share { member: String, epoch: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RecordOp {
    /// Metadata fields in the meta section, secret fields in the secret section.
    Set { coll: Coll, fields: BTreeMap<String, Value> },
    Delete,
    Grant { member: String, level: Level, cutoff: Cutoff },
    /// A new key generation for a section, handed to `holders`.
    Rotate { section: Section, generation: u32, kid: String, holders: Vec<String> },
    /// The current key of a section was handed to a member who missed it.
    Share { member: String, section: Section, generation: u32 },
}

/// Who wrote a value and what they had seen at the time.
#[derive(Debug, Clone)]
pub struct Version {
    pub hlc: Hlc,
    pub author: String,
    pub device: String,
    pub seq: u64,
    pub seen: BTreeMap<String, u64>,
}

impl Version {
    fn from(b: &log::Body) -> Self {
        Self { hlc: b.hlc, author: b.author.clone(), device: b.device.clone(), seq: b.seq, seen: b.seen.iter().map(|(d, h)| (d.clone(), h.seq)).collect() }
    }
    /// Had the writer of `self` seen `other` when writing?
    pub fn has_seen(&self, other: &Version) -> bool {
        if self.device == other.device {
            return other.seq < self.seq;
        }
        self.seen.get(&other.device).is_some_and(|&s| s >= other.seq)
    }
}

#[derive(Debug, Clone)]
pub struct Contender {
    pub value: Value,
    pub version: Version,
}

/// A key generation of the team or of a record section.
#[derive(Debug, Clone)]
pub struct Gen {
    pub generation: u32,
    pub kid: String,
    /// Members it was handed to.
    pub holders: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub coll: Option<Coll>,
    pub creator: Option<String>,
    /// Concurrent values per field; one element unless the field is in conflict.
    pub fields: BTreeMap<String, Vec<Contender>>,
    pub deleted: Option<Version>,
    /// Member -> access. Absent means none.
    pub acl: BTreeMap<String, Level>,
    pub gens: BTreeMap<Section, Gen>,
    /// Every announced key id per section, oldest first: history stays readable.
    pub history: BTreeMap<Section, Vec<String>>,
}

impl Record {
    /// Current value of each field (the latest contender).
    pub fn values(&self) -> BTreeMap<String, Value> {
        self.fields.iter().filter_map(|(k, cs)| cs.iter().max_by_key(|c| c.version.hlc).map(|c| (k.clone(), c.value.clone()))).collect()
    }

    /// Deleted, and no edit happened without the deleter having seen it.
    pub fn is_deleted(&self) -> bool {
        self.deleted.as_ref().is_some_and(|d| self.fields.values().flatten().all(|c| d.has_seen(&c.version)))
    }

    pub fn name(&self) -> String {
        let Some(coll) = self.coll else { return String::new() };
        match self.values().get(coll.name_field()) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        }
    }
}

/// Something wrong with the storage or a change in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Problem {
    /// A change was ignored: bad signature, broken chain, or its author lacked the right.
    Rejected { device: String, author: String, seq: u64, reason: String },
    /// An entry is absent: a gap in a device's log, or one another entry refers to.
    Missing { device: String, author: String, seq: u64 },
    /// Two different versions of one entry exist.
    Forked { device: String, author: String, seq: u64 },
    /// The storage shows less of a device's log than this client had already read.
    RolledBack { device: String, author: String, seq: u64 },
    /// A file that is neither an entry nor known sync-tool debris.
    UnknownFile { path: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub author: String,
    pub at: u64,
    #[serde(flatten)]
    pub what: ActivityKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ActivityKind {
    Created,
    Renamed { name: String },
    Added { coll: Coll, id: String, name: String },
    Changed { coll: Coll, id: String, name: String },
    SecretChanged { coll: Coll, id: String, name: String },
    Deleted { coll: Coll, id: String, name: String },
    AccessChanged { coll: Coll, id: String, name: String, member: String, level: Level },
    MemberAdded { member: String },
    MemberChanged { member: String },
    MemberRemoved { member: String },
}

/// A right an applied entry relied on.
#[derive(Debug, Clone)]
enum Need {
    Member,
    Role(Role),
    Level(String, Level),
}

#[derive(Debug)]
struct Applied {
    hash: String,
    author: String,
    device: String,
    seq: u64,
    needs: Vec<Need>,
}

#[derive(Debug, Default)]
pub struct State {
    pub name: String,
    pub members: BTreeMap<String, Member>,
    /// Display names of members as they were, for history after removal.
    pub former: HashMap<String, String>,
    pub epoch: Option<Gen>,
    /// Every announced team key id, oldest first.
    pub epoch_history: Vec<String>,
    pub records: BTreeMap<String, Record>,
    pub activity: Vec<Activity>,
    pub problems: Vec<Problem>,
    /// Last valid entry per device.
    pub heads: BTreeMap<String, Head>,
    /// Device -> its author.
    pub devices: BTreeMap<String, String>,
    pub latest: Hlc,
    /// Key id -> every key this reader received.
    pub keys: HashMap<String, Key>,
    /// Data parts this reader has no key for.
    pub unreadable: usize,
    /// Time of each device's last entry.
    pub last_at: BTreeMap<String, Hlc>,
    applied: Vec<Applied>,
    /// Entries written after their author lost the right they needed.
    exclude: HashSet<String>,
}

impl State {
    pub fn role_of(&self, id: &str) -> Option<Role> {
        self.members.get(id).map(|m| m.role)
    }

    pub fn level_of(&self, member: &str, record: &str) -> Level {
        if !self.members.contains_key(member) {
            return Level::None;
        }
        self.records.get(record).and_then(|r| r.acl.get(member).copied()).unwrap_or(Level::None)
    }

    pub fn member_name(&self, id: &str) -> String {
        self.members.get(id).map(|m| m.name.clone()).or_else(|| self.former.get(id).cloned()).unwrap_or_default()
    }

    /// Someone holds the current team key who is no longer a member.
    pub fn team_key_stale(&self) -> bool {
        self.epoch.as_ref().is_some_and(|g| g.holders.iter().any(|h| !self.members.contains_key(h)))
    }

    /// Someone holds the record section's current key who no longer has that access.
    pub fn section_stale(&self, record: &str, section: Section) -> bool {
        let Some(g) = self.records.get(record).and_then(|r| r.gens.get(&section)) else { return false };
        g.holders.iter().any(|h| self.level_of(h, record) < section.level())
    }

    /// The device heads of `member` as this reader knows them.
    pub fn cutoff_for(&self, member: &str) -> Cutoff {
        self.devices.iter().filter(|(_, a)| a.as_str() == member).filter_map(|(d, _)| self.heads.get(d).map(|h| (d.clone(), h.seq))).collect()
    }
}

/* ---------- verification ---------- */

/// Entries that passed the storage checks, in replay order.
pub struct Verified {
    pub entries: Vec<Entry>,
    pub heads: BTreeMap<String, Head>,
    pub devices: BTreeMap<String, String>,
    pub problems: Vec<Problem>,
}

/// Checks everything in the folder against itself and against the heads this client remembers.
pub fn verify_all(scan: Scan, known: &BTreeMap<String, Head>) -> Verified {
    let mut problems = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut heads = BTreeMap::new();
    let mut devices = BTreeMap::new();
    // Device -> seq -> hash of its valid entries.
    let mut index: BTreeMap<String, BTreeMap<u64, String>> = BTreeMap::new();

    // Each device's chain; stop a device at its first gap or bad entry.
    for (device, files) in scan.logs {
        let mut expected_seq = 1;
        let mut prev: Option<String> = None;
        let mut author: Option<String> = None;
        for (seq, bytes) in files {
            let who = author.clone().unwrap_or_default();
            if seq != expected_seq {
                problems.push(Problem::Missing { device: device.clone(), author: who, seq: expected_seq });
                break;
            }
            let e = match log::verify(&bytes) {
                Ok(e) => e,
                Err(why) => {
                    problems.push(Problem::Rejected { device: device.clone(), author: who, seq, reason: why });
                    break;
                }
            };
            if e.body.device != device || e.body.seq != seq || e.body.prev != prev || author.as_ref().is_some_and(|a| a != &e.body.author) {
                problems.push(Problem::Rejected { device: device.clone(), author: e.body.author.clone(), seq, reason: "broken chain".into() });
                break;
            }
            author = Some(e.body.author.clone());
            prev = Some(e.hash.clone());
            devices.insert(device.clone(), e.body.author.clone());
            heads.insert(device.clone(), Head { seq, hash: e.hash.clone() });
            index.entry(device.clone()).or_default().insert(seq, e.hash.clone());
            expected_seq += 1;
            entries.push(e);
        }
    }
    let author_of = |d: &str| devices.get(d).cloned().unwrap_or_default();

    // Stray files. A valid entry stored under another name is a second version if it differs.
    for x in scan.extras {
        if let (Some(device), Some(bytes)) = (&x.device, &x.bytes)
            && let Ok(e) = log::verify(bytes)
            && &e.body.device == device
        {
            match index.get(device).and_then(|m| m.get(&e.body.seq)) {
                Some(h) if *h == e.hash => continue,
                Some(_) => {
                    problems.push(Problem::Forked { device: device.clone(), author: e.body.author.clone(), seq: e.body.seq });
                    continue;
                }
                None => {}
            }
        }
        if !x.debris {
            problems.push(Problem::UnknownFile { path: x.path });
        }
    }

    // What others had read must still be here, byte for byte.
    let mut refs = BTreeSet::new();
    for e in &entries {
        for (d, h) in &e.body.seen {
            if d != &e.body.device {
                refs.insert((d.clone(), h.seq, h.hash.clone()));
            }
        }
    }
    let check = |d: &str, h: &Head, gone: fn(String, String, u64) -> Problem, problems: &mut Vec<Problem>| match index.get(d).and_then(|m| m.get(&h.seq)) {
        Some(hash) if *hash == h.hash => {}
        Some(_) => problems.push(Problem::Forked { device: d.to_string(), author: author_of(d), seq: h.seq }),
        None => problems.push(gone(d.to_string(), author_of(d), h.seq)),
    };
    for (d, seq, hash) in refs {
        check(&d, &Head { seq, hash }, |device, author, seq| Problem::Missing { device, author, seq }, &mut problems);
    }
    for (d, h) in known {
        check(d, h, |device, author, seq| Problem::RolledBack { device, author, seq }, &mut problems);
    }

    let mut seen = HashSet::new();
    problems.retain(|p| seen.insert(format!("{p:?}")));
    entries.sort_by(|a, b| (a.body.hlc, &a.body.author, &a.body.device, a.body.seq).cmp(&(b.body.hlc, &b.body.author, &b.body.device, b.body.seq)));
    Verified { entries, heads, devices, problems }
}

/* ---------- replay ---------- */

/// Keys from drops, cached per entry hash: opening a drop costs an X25519 operation.
pub type DropCache = HashMap<String, Vec<Key>>;

/// Rebuilds state. `root` is the trusted creator key id.
pub fn replay(v: &Verified, root: &str, me: &Identity, drops: &mut DropCache) -> State {
    let mut excluded = HashSet::new();
    loop {
        let st = replay_once(v, root, me, drops, &excluded);
        if st.exclude.is_subset(&excluded) {
            return st;
        }
        excluded.extend(st.exclude);
    }
}

fn replay_once(v: &Verified, root: &str, me: &Identity, drops: &mut DropCache, excluded: &HashSet<String>) -> State {
    let mut st = State { heads: v.heads.clone(), devices: v.devices.clone(), problems: v.problems.clone(), ..Default::default() };

    // Keys handed to us are usable for reading whoever sent them; which key a
    // writer must use is decided by the announcements replayed below.
    for e in &v.entries {
        let keys = drops.entry(e.hash.clone()).or_insert_with(|| {
            e.body.parts.iter().filter_map(|p| if let Part::Drop { ct } = p { log::open_drop(ct, me) } else { None }).flatten().collect()
        });
        for k in keys.iter() {
            st.keys.insert(log::key_id(k), *k);
        }
    }

    for e in &v.entries {
        let b = &e.body;
        if excluded.contains(&e.hash) {
            st.problems.push(Problem::Rejected { device: b.device.clone(), author: b.author.clone(), seq: b.seq, reason: "written after losing access".into() });
            continue;
        }
        st.latest = st.latest.max(b.hlc);
        st.last_at.insert(b.device.clone(), b.hlc);
        let version = Version::from(b);
        let mut ctx = Ctx { bootstrapping: st.members.is_empty() && st.epoch.is_none() && b.author == root, created: HashSet::new(), needs: Vec::new() };
        for part in &b.parts {
            let Part::Data { kid, ct } = part else { continue };
            let Some(key) = st.keys.get(kid).copied() else {
                st.unreadable += 1;
                continue;
            };
            let reject = |st: &mut State, why: &str| st.problems.push(Problem::Rejected { device: b.device.clone(), author: b.author.clone(), seq: b.seq, reason: why.into() });
            let Some(plain) = log::open_data(&key, kid, ct) else {
                reject(&mut st, "undecryptable");
                continue;
            };
            let Ok(payload) = serde_json::from_slice::<Payload>(&plain) else {
                reject(&mut st, "unreadable changes");
                continue;
            };
            match payload {
                Payload::Team { ops } => {
                    for op in ops {
                        if let Err(why) = apply_team(&mut st, op, &version, &mut ctx) {
                            reject(&mut st, why);
                        }
                    }
                }
                Payload::Record { record, section, ops } => {
                    for op in ops {
                        if let Err(why) = apply_record(&mut st, &record, section, op, &version, &mut ctx) {
                            reject(&mut st, why);
                        }
                    }
                }
            }
        }
        if !ctx.needs.is_empty() {
            st.applied.push(Applied { hash: e.hash.clone(), author: b.author.clone(), device: b.device.clone(), seq: b.seq, needs: ctx.needs });
        }
    }
    st
}

struct Ctx {
    bootstrapping: bool,
    /// Records created by this entry: their creator sets the first access list.
    created: HashSet<String>,
    needs: Vec<Need>,
}

/// After `target` lost rights: their earlier-placed entries that the person
/// restricting them had not read are judged with the reduced rights.
fn restrict(st: &mut State, target: &str, cutoff: &Cutoff, still: impl Fn(&Need) -> bool) {
    let late: Vec<String> = st
        .applied
        .iter()
        .filter(|a| a.author == target && a.seq > cutoff.get(&a.device).copied().unwrap_or(0) && a.needs.iter().any(|n| !still(n)))
        .map(|a| a.hash.clone())
        .collect();
    st.exclude.extend(late);
}

fn apply_team(st: &mut State, op: TeamOp, v: &Version, ctx: &mut Ctx) -> Result<(), &'static str> {
    let author_role = st.role_of(&v.author);
    let at = v.hlc.ms;
    let log = |st: &mut State, what: ActivityKind| st.activity.push(Activity { author: v.author.clone(), at, what });

    match op {
        TeamOp::Genesis { name, members } => {
            if !ctx.bootstrapping || !st.members.is_empty() {
                return Err("genesis from someone other than the creator");
            }
            if !members.iter().all(member_id_matches) {
                return Err("member id does not match its Detour ID");
            }
            st.name = name;
            for m in members {
                st.former.insert(m.id.clone(), m.name.clone());
                st.members.insert(m.id.clone(), m);
            }
            if st.role_of(&v.author) != Some(Role::Owner) {
                st.members.clear();
                return Err("creator is not an owner");
            }
            log(st, ActivityKind::Created);
        }
        TeamOp::Epoch { epoch, kid, holders } => {
            let first = st.epoch.is_none() && ctx.bootstrapping;
            if !first && !author_role.is_some_and(Role::can_manage) {
                return Err("team key from a non-admin");
            }
            if st.epoch.as_ref().is_some_and(|g| epoch <= g.generation) {
                return Err("old team key generation");
            }
            if !first {
                ctx.needs.push(Need::Role(Role::Admin));
            }
            st.epoch_history.push(kid.clone());
            st.epoch = Some(Gen { generation: epoch, kid, holders: holders.into_iter().collect() });
        }
        TeamOp::Rename { name } => {
            if !author_role.is_some_and(Role::can_manage) {
                return Err("rename by a non-admin");
            }
            ctx.needs.push(Need::Role(Role::Admin));
            st.name = name.clone();
            log(st, ActivityKind::Renamed { name });
        }
        TeamOp::MemberPut { member, cutoff } => {
            let author = author_role.ok_or("change by a non-member")?;
            if !member_id_matches(&member) {
                return Err("member id does not match its Detour ID");
            }
            let existing = st.members.get(&member.id).cloned();
            if member.recovery != existing.as_ref().is_some_and(|m| m.recovery) {
                return Err("recovery flag changed");
            }
            // Admins manage members; only owners touch owners and admins.
            let privileged = member.role.can_manage() || existing.as_ref().is_some_and(|m| m.role.can_manage());
            if !author.can_manage() || (privileged && author != Role::Owner) {
                return Err("not allowed to change this member");
            }
            let owners_left = st.members.values().filter(|m| m.role == Role::Owner && m.id != member.id).count();
            if existing.as_ref().is_some_and(|m| m.role == Role::Owner) && member.role != Role::Owner && owners_left == 0 {
                return Err("cannot demote the last owner");
            }
            ctx.needs.push(Need::Role(if privileged { Role::Owner } else { Role::Admin }));
            let name = member.name.clone();
            let id = member.id.clone();
            let new_role = member.role;
            st.former.insert(id.clone(), name.clone());
            st.members.insert(id.clone(), member);
            match existing {
                None => {
                    // A new member gets the current team key in the same entry.
                    if let Some(g) = st.epoch.as_mut() {
                        g.holders.insert(id);
                    }
                    log(st, ActivityKind::MemberAdded { member: name });
                }
                Some(old) => {
                    if !new_role.at_least(old.role) {
                        restrict(st, &id, &cutoff, |n| !matches!(n, Need::Role(min) if !new_role.at_least(*min)));
                    }
                    log(st, ActivityKind::MemberChanged { member: name });
                }
            }
        }
        TeamOp::MemberRemove { id, cutoff } => {
            let author = author_role.ok_or("change by a non-member")?;
            let target = st.members.get(&id).cloned().ok_or("removing a non-member")?;
            if !author.can_manage() || (target.role.can_manage() && author != Role::Owner) {
                return Err("not allowed to remove this member");
            }
            let owners_left = st.members.values().filter(|m| m.role == Role::Owner && m.id != id).count();
            if target.role == Role::Owner && owners_left == 0 {
                return Err("cannot remove the last owner");
            }
            ctx.needs.push(Need::Role(if target.role.can_manage() { Role::Owner } else { Role::Admin }));
            st.former.insert(id.clone(), target.name.clone());
            st.members.remove(&id);
            restrict(st, &id, &cutoff, |_| false);
            log(st, ActivityKind::MemberRemoved { member: target.name });
        }
        TeamOp::Share { member, epoch } => {
            author_role.ok_or("change by a non-member")?;
            if !st.members.contains_key(&member) {
                return Err("team key for a non-member");
            }
            ctx.needs.push(Need::Member);
            if let Some(g) = st.epoch.as_mut().filter(|g| g.generation == epoch) {
                g.holders.insert(member);
            }
        }
    }
    Ok(())
}

fn member_id_matches(m: &Member) -> bool {
    PublicIdentity::parse(&m.detour_id).is_ok_and(|p| p.key_id() == m.id)
}

fn apply_record(st: &mut State, rid: &str, section: Section, op: RecordOp, v: &Version, ctx: &mut Ctx) -> Result<(), &'static str> {
    let author = v.author.as_str();
    let role = st.role_of(author).ok_or("change by a non-member")?;
    let at = v.hlc.ms;
    let creating_entry = ctx.created.contains(rid);
    if section == Section::Secret && !matches!(op, RecordOp::Set { .. }) {
        return Err("only values belong in the secret section");
    }
    let my_level = st.level_of(author, rid);

    match op {
        RecordOp::Set { coll, fields } => {
            if fields.keys().any(|f| coll.is_secret_field(f) != (section == Section::Secret)) {
                return Err("field in the wrong section");
            }
            let rec = st.records.entry(rid.to_string()).or_default();
            let creating = rec.coll.is_none();
            if creating {
                if section != Section::Meta {
                    return Err("secret for a record that does not exist");
                }
                rec.coll = Some(coll);
                rec.creator = Some(author.to_string());
                rec.acl.insert(author.to_string(), Level::Edit);
                ctx.created.insert(rid.to_string());
                ctx.needs.push(Need::Member);
            } else {
                if rec.coll != Some(coll) {
                    return Err("record type changed");
                }
                if my_level < Level::Edit {
                    return Err("edit without access");
                }
                ctx.needs.push(Need::Level(rid.to_string(), Level::Edit));
            }
            let was_new = creating || rec.is_deleted();
            // An edit made after seeing the delete brings the record back.
            if rec.deleted.as_ref().is_some_and(|d| v.has_seen(d)) {
                rec.deleted = None;
            }
            for (field, value) in fields {
                let cs = rec.fields.entry(field).or_default();
                cs.retain(|c| !v.has_seen(&c.version));
                cs.push(Contender { value, version: v.clone() });
            }
            let (id, name) = (rid.to_string(), rec.name());
            let what = match section {
                Section::Secret => ActivityKind::SecretChanged { coll, id, name },
                Section::Meta if was_new => ActivityKind::Added { coll, id, name },
                Section::Meta => ActivityKind::Changed { coll, id, name },
            };
            // Values set while creating are part of "added".
            if !(section == Section::Secret && creating_entry) {
                st.activity.push(Activity { author: author.to_string(), at, what });
            }
        }
        RecordOp::Delete => {
            let rec = st.records.get_mut(rid).filter(|r| r.coll.is_some()).ok_or("unknown record")?;
            if my_level < Level::Edit {
                return Err("delete without access");
            }
            ctx.needs.push(Need::Level(rid.to_string(), Level::Edit));
            // Values are kept (hidden while deleted) so an undo or "keep" can restore them.
            rec.deleted = Some(v.clone());
            let (coll, name) = (rec.coll.unwrap(), rec.name());
            st.activity.push(Activity { author: author.to_string(), at, what: ActivityKind::Deleted { coll, id: rid.to_string(), name } });
        }
        RecordOp::Grant { member, level, cutoff } => {
            let rec = st.records.get(rid).filter(|r| r.coll.is_some()).ok_or("unknown record")?;
            let coll = rec.coll.unwrap();
            let level = level.normalize(coll);
            let target_role = st.role_of(&member).ok_or("access for a non-member")?;
            let current = st.level_of(&member, rid);
            let by_creator = creating_entry && rec.creator.as_deref() == Some(author);
            if by_creator {
                ctx.needs.push(Need::Member);
            } else {
                if !role.can_manage() {
                    return Err("access changed by a non-admin");
                }
                let reach = level.max(current);
                if my_level < reach {
                    return Err("access beyond the granter's own");
                }
                let restricting_admin = level < current && target_role.can_manage();
                if restricting_admin && role != Role::Owner {
                    return Err("only owners restrict admins");
                }
                ctx.needs.push(Need::Role(if restricting_admin { Role::Owner } else { Role::Admin }));
                ctx.needs.push(Need::Level(rid.to_string(), reach));
            }
            let rec = st.records.get_mut(rid).unwrap();
            if level == Level::None {
                rec.acl.remove(&member);
            } else {
                rec.acl.insert(member.clone(), level);
            }
            // A grant comes with the current keys up to its level.
            for (s, g) in rec.gens.iter_mut() {
                if level >= s.level() {
                    g.holders.insert(member.clone());
                }
            }
            let name = rec.name();
            if level < current {
                let rid_owned = rid.to_string();
                restrict(st, &member, &cutoff, |n| !matches!(n, Need::Level(r, min) if *r == rid_owned && level < *min));
            }
            if !by_creator {
                let who = st.member_name(&member);
                st.activity.push(Activity { author: author.to_string(), at, what: ActivityKind::AccessChanged { coll, id: rid.to_string(), name, member: who, level } });
            }
        }
        RecordOp::Rotate { section: s, generation, kid, holders } => {
            let rec = st.records.get(rid).filter(|r| r.coll.is_some()).ok_or("unknown record")?;
            let by_creator = creating_entry && rec.creator.as_deref() == Some(author);
            let allowed = by_creator || my_level >= Level::Edit || (role.can_manage() && my_level >= s.level());
            if !allowed {
                return Err("key change without access");
            }
            if rec.gens.get(&s).is_some_and(|g| generation <= g.generation) {
                return Err("old key generation");
            }
            ctx.needs.push(if by_creator { Need::Member } else { Need::Level(rid.to_string(), s.level()) });
            let rec = st.records.get_mut(rid).unwrap();
            rec.history.entry(s).or_default().push(kid.clone());
            rec.gens.insert(s, Gen { generation, kid, holders: holders.into_iter().collect() });
        }
        RecordOp::Share { member, section: s, generation } => {
            if my_level < s.level() {
                return Err("sharing a key without access");
            }
            if st.level_of(&member, rid) < s.level() {
                return Err("key for someone without access");
            }
            ctx.needs.push(Need::Level(rid.to_string(), s.level()));
            if let Some(g) = st.records.get_mut(rid).and_then(|r| r.gens.get_mut(&s)).filter(|g| g.generation == generation) {
                g.holders.insert(member);
            }
        }
    }
    Ok(())
}
