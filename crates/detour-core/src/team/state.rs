//! Rebuilding a team vault's state from its logs.
//!
//! Entries are verified, then replayed in a total order (HLC, author, device,
//! seq). Membership and key operations are checked against the author's role
//! at that point. Record fields merge last-writer-wins; when two writers changed
//! the same field without having seen each other's change, both values are
//! kept as a conflict until someone resolves it.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::log::{self, Entry, EpochKeys, Hlc, Kind};
use crate::identity::Identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coll {
    Host,
    Key,
    Snippet,
    Tunnel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Owner,
    Admin,
    Member,
    ReadOnly,
}

impl Role {
    pub fn can_write(self) -> bool {
        matches!(self, Role::Owner | Role::Admin | Role::Member)
    }
    pub fn can_manage(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
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
    pub can_reveal: bool,
    #[serde(default)]
    pub recovery: bool,
}

/// Decrypted contents of a `Data` log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Op {
    Genesis { name: String, members: Vec<Member> },
    Rename { name: String },
    MemberPut { member: Member },
    MemberRemove { id: String },
    Set { coll: Coll, id: String, fields: BTreeMap<String, Value> },
    Delete { coll: Coll, id: String },
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
        Self { hlc: b.hlc, author: b.author.clone(), device: b.device.clone(), seq: b.seq, seen: b.seen.clone() }
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

#[derive(Debug, Clone, Default)]
pub struct Record {
    /// Concurrent values per field; one element unless the field is in conflict.
    pub fields: BTreeMap<String, Vec<Contender>>,
    pub deleted: Option<Version>,
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
}

/// Something in the logs that was skipped.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub device: String,
    pub reason: String,
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
    Added { coll: Coll, id: String },
    Changed { coll: Coll, id: String },
    Deleted { coll: Coll, id: String },
    MemberAdded { member: String },
    MemberChanged { member: String },
    MemberRemoved { member: String },
}

#[derive(Debug, Default)]
pub struct State {
    pub name: String,
    pub members: BTreeMap<String, Member>,
    /// Display names of members as they were, for history after removal.
    pub former: HashMap<String, String>,
    pub keys: EpochKeys,
    pub records: BTreeMap<(Coll, String), Record>,
    pub activity: Vec<Activity>,
    pub problems: Vec<Problem>,
    /// Highest valid sequence number per device.
    pub heads: BTreeMap<String, (u64, String)>,
    pub latest: Hlc,
    /// Entries for epochs this reader has no key for.
    pub unreadable: usize,
}

impl State {
    pub fn current_epoch(&self) -> Option<(u32, [u8; 32])> {
        self.keys.iter().next_back().map(|(e, k)| (*e, *k))
    }

    fn role_of(&self, id: &str) -> Option<Role> {
        self.members.get(id).map(|m| m.role)
    }

    pub fn member_name(&self, id: &str) -> String {
        self.members.get(id).map(|m| m.name.clone()).or_else(|| self.former.get(id).cloned()).unwrap_or_default()
    }
}

/// Rebuilds state from raw log files. `root` is the trusted creator key id.
pub fn replay(files: BTreeMap<String, Vec<(u64, Vec<u8>)>>, root: &str, me: &Identity) -> State {
    let mut st = State::default();
    let mut entries: Vec<Entry> = Vec::new();

    // Verify signatures and each device's hash chain; stop a device at its first gap or bad entry.
    for (device, files) in files {
        let mut expected_seq = 1;
        let mut prev: Option<String> = None;
        let mut author: Option<String> = None;
        for (seq, bytes) in files {
            let reject = |st: &mut State, why: &str| st.problems.push(Problem { device: device.clone(), reason: format!("#{seq}: {why}") });
            if seq != expected_seq {
                reject(&mut st, "missing earlier changes");
                break;
            }
            let e = match log::verify(&bytes) {
                Ok(e) => e,
                Err(why) => {
                    reject(&mut st, &why);
                    break;
                }
            };
            if e.body.device != device || e.body.seq != seq || e.body.prev != prev || author.as_ref().is_some_and(|a| a != &e.body.author) {
                reject(&mut st, "broken chain");
                break;
            }
            author = Some(e.body.author.clone());
            prev = Some(e.hash.clone());
            st.heads.insert(device.clone(), (seq, e.hash.clone()));
            expected_seq += 1;
            entries.push(e);
        }
    }

    entries.sort_by(|a, b| (a.body.hlc, &a.body.author, &a.body.device, a.body.seq).cmp(&(b.body.hlc, &b.body.author, &b.body.device, b.body.seq)));

    // A newcomer receives the keys after the history was written, so every key
    // addressed to us is usable for reading. Only keys from admins (checked in
    // order below) are used for writing.
    let mut readable = EpochKeys::new();
    for e in entries.iter().filter(|e| e.body.kind == Kind::Keys) {
        if let Some(keys) = log::open_keys(&e.body.payload, me) {
            readable.extend(keys);
        }
    }

    for e in &entries {
        let b = &e.body;
        st.latest = st.latest.max(b.hlc);
        let author_role = st.role_of(&b.author);
        let bootstrapping = st.members.is_empty() && b.author == root;

        match b.kind {
            Kind::Keys => {
                if !(bootstrapping || author_role.is_some_and(Role::can_manage)) {
                    st.problems.push(Problem { device: b.device.clone(), reason: "keys from a non-admin".into() });
                    continue;
                }
                if let Some(keys) = log::open_keys(&b.payload, me) {
                    st.keys.extend(keys);
                }
            }
            Kind::Data => {
                let Some(key) = readable.get(&b.epoch) else {
                    st.unreadable += 1;
                    continue;
                };
                let Some(plain) = log::open_data(key, &b.payload) else {
                    st.problems.push(Problem { device: b.device.clone(), reason: "undecryptable".into() });
                    continue;
                };
                let Ok(ops) = serde_json::from_slice::<Vec<Op>>(&plain) else {
                    st.problems.push(Problem { device: b.device.clone(), reason: "unreadable changes".into() });
                    continue;
                };
                let version = Version::from(b);
                for op in ops {
                    if let Err(why) = apply(&mut st, op, &version, bootstrapping) {
                        st.problems.push(Problem { device: b.device.clone(), reason: why.into() });
                    }
                }
            }
        }
    }
    st
}

fn apply(st: &mut State, op: Op, v: &Version, bootstrapping: bool) -> Result<(), &'static str> {
    let author_role = st.role_of(&v.author);
    let at = v.hlc.ms;
    let log = |st: &mut State, what: ActivityKind| st.activity.push(Activity { author: v.author.clone(), at, what });

    match op {
        Op::Genesis { name, members } => {
            if !bootstrapping {
                return Err("genesis from someone other than the creator");
            }
            st.name = name;
            for m in members {
                st.members.insert(m.id.clone(), m);
            }
            if st.role_of(&v.author) != Some(Role::Owner) {
                st.members.clear();
                return Err("creator is not an owner");
            }
            log(st, ActivityKind::Created);
        }
        Op::Rename { name } => {
            if !author_role.is_some_and(Role::can_manage) {
                return Err("rename by a non-admin");
            }
            st.name = name.clone();
            log(st, ActivityKind::Renamed { name });
        }
        Op::MemberPut { member } => {
            let author = author_role.ok_or("change by a non-member")?;
            let existing = st.members.get(&member.id).cloned();
            // Admins manage members and read-only users; only owners touch owners and admins.
            let touches_privileged = member.role.can_manage() || existing.as_ref().is_some_and(|m| m.role.can_manage());
            if !author.can_manage() || (touches_privileged && author != Role::Owner) {
                return Err("not allowed to change this member");
            }
            let name = member.name.clone();
            st.former.insert(member.id.clone(), name.clone());
            st.members.insert(member.id.clone(), member);
            log(st, if existing.is_some() { ActivityKind::MemberChanged { member: name } } else { ActivityKind::MemberAdded { member: name } });
        }
        Op::MemberRemove { id } => {
            let author = author_role.ok_or("change by a non-member")?;
            let target = st.members.get(&id).cloned().ok_or("removing a non-member")?;
            if !author.can_manage() || (target.role.can_manage() && author != Role::Owner) {
                return Err("not allowed to remove this member");
            }
            let owners_left = st.members.values().filter(|m| m.role == Role::Owner && m.id != id).count();
            if target.role == Role::Owner && owners_left == 0 {
                return Err("cannot remove the last owner");
            }
            st.former.insert(id.clone(), target.name.clone());
            st.members.remove(&id);
            log(st, ActivityKind::MemberRemoved { member: target.name });
        }
        Op::Set { coll, id, fields } => {
            if !author_role.is_some_and(Role::can_write) {
                return Err("edit by a member without write access");
            }
            let rec = st.records.entry((coll, id.clone())).or_default();
            let was_new = rec.fields.is_empty() || rec.is_deleted();
            // An edit made after seeing the delete brings the record back.
            if rec.deleted.as_ref().is_some_and(|d| v.has_seen(d)) {
                rec.deleted = None;
            }
            for (field, value) in fields {
                let cs = rec.fields.entry(field).or_default();
                cs.retain(|c| !v.has_seen(&c.version));
                cs.push(Contender { value, version: v.clone() });
            }
            log(st, if was_new { ActivityKind::Added { coll, id } } else { ActivityKind::Changed { coll, id } });
        }
        Op::Delete { coll, id } => {
            if !author_role.is_some_and(Role::can_write) {
                return Err("delete by a member without write access");
            }
            // Values are kept (hidden while deleted) so an undo or "keep" can restore them.
            let rec = st.records.entry((coll, id.clone())).or_default();
            rec.deleted = Some(v.clone());
            log(st, ActivityKind::Deleted { coll, id });
        }
    }
    Ok(())
}
