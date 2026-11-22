//! Looking back at a team vault: what every entry changed, the versions of one record, and putting
//! records back as they were.
//!
//! Going back never rewrites the logs. The state as it was (at a time, or without some entries) is
//! rebuilt from the logs, compared with the state now, and the difference is written as one new
//! entry, checked by everyone like any other edit. So a rollback is itself in the history and can
//! be undone. Only records go back: members, roles and access stay as they are.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::log::{self, Part};
use super::state::{self, Coll, Level, Payload, Record, RecordOp, Role, Section, State, TeamOp, Verified};
use super::{Draft, TeamVault, split};
use crate::identity::PublicIdentity;
use crate::{Error, Result};

/// What to go back to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Rollback {
    /// Everything as it was at this time (milliseconds since the epoch).
    At { at: u64 },
    /// Everything as if this entry had never been written.
    Undo { entry: String },
    /// Everything as if a device's or a member's entries after `after` had never been written.
    Without { device: Option<String>, author: Option<String>, after: u64 },
    /// One record as it was right after this entry.
    Record { record: String, entry: String },
    /// A deleted record back, as it was when deleted.
    Restore { record: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Created,
    Changed,
    Deleted,
    Restored,
    Access,
    TeamCreated,
    TeamRenamed,
    MemberAdded,
    MemberChanged,
    MemberRemoved,
}

/// One thing an entry changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeView {
    pub kind: ChangeKind,
    pub coll: Option<Coll>,
    pub record: Option<String>,
    /// The record's, member's or team's name right after this change.
    pub name: String,
    /// Metadata fields set and their new values.
    pub fields: BTreeMap<String, Value>,
    /// Secret fields set: named, never shown.
    pub secrets: Vec<String>,
    /// For an access change: whose and to what.
    pub member: Option<String>,
    pub level: Option<Level>,
}

/// An entry of the logs with what it changed that this member can read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryView {
    pub entry: String,
    pub at: u64,
    pub author: String,
    pub author_name: String,
    pub is_you: bool,
    pub device: String,
    pub this_device: bool,
    pub changes: Vec<ChangeView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanAction {
    /// Comes back from being deleted.
    Restore,
    /// Did not exist then: gets deleted.
    Remove,
    /// Fields go back to their values.
    Change,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDiff {
    pub field: String,
    pub secret: bool,
    /// Values now and after going back; never filled for a secret.
    pub from: Option<Value>,
    pub to: Option<Value>,
}

/// One record going back.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub coll: Coll,
    pub record: String,
    pub name: String,
    pub action: PlanAction,
    pub fields: Vec<FieldDiff>,
    /// This member may change the record (edit access).
    pub allowed: bool,
}

/// What going back would change.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPlan {
    pub items: Vec<PlanItem>,
}

/// What going back changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackDone {
    pub applied: usize,
    /// Records left as they are: no edit access.
    pub skipped: usize,
    /// The entry written, to undo the rollback itself; none when nothing changed.
    pub entry: Option<String>,
}

/// A deleted record, with who deleted it and when.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedView {
    pub coll: Coll,
    pub record: String,
    pub name: String,
    pub at: u64,
    pub author_name: String,
    pub is_you: bool,
}

/// A plan item with everything needed to write it.
struct Step {
    item: PlanItem,
    /// Every field of the record as it should be.
    values: BTreeMap<String, Value>,
}

impl TeamVault {
    /// Every entry, in the order they apply.
    fn verified_entries(&self) -> Result<Verified> {
        let mut verifier = log::Verifier::with_trusted(self.verifier.verified().cloned());
        Ok(state::verify_all(self.folder.scan()?, &self.known, &mut verifier))
    }

    /// What an entry carries that this member can decrypt.
    fn payloads(&self, e: &log::Entry) -> Vec<Payload> {
        e.body
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Data { kid, ct } => {
                    let key = self.state.keys.get(kid)?;
                    serde_json::from_slice(&log::open_data(key, kid, ct)?).ok()
                }
                Part::Drop { .. } => None,
            })
            .collect()
    }

    /// Every entry that changed something this member can read, oldest first.
    pub fn log(&self) -> Result<Vec<LogEntryView>> {
        let v = self.verified_entries()?;
        let me = self.my_id();
        let mut names: HashMap<String, String> = HashMap::new();
        let mut members: HashMap<String, String> = HashMap::new();
        let mut deleted: HashSet<String> = HashSet::new();
        let mut known: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for e in &v.entries {
            let b = &e.body;
            let mut changes: Vec<ChangeView> = Vec::new();
            for p in self.payloads(e) {
                match p {
                    Payload::Team { ops } => {
                        for op in ops {
                            let (kind, name) = match op {
                                TeamOp::Genesis { name, members: ms } => {
                                    members.extend(ms.into_iter().map(|m| (m.id, m.name)));
                                    (ChangeKind::TeamCreated, name)
                                }
                                TeamOp::Rename { name } => (ChangeKind::TeamRenamed, name),
                                TeamOp::MemberPut { member, .. } => {
                                    let known = members.insert(member.id.clone(), member.name.clone()).is_some();
                                    (if known { ChangeKind::MemberChanged } else { ChangeKind::MemberAdded }, member.name)
                                }
                                TeamOp::MemberRemove { id, .. } => {
                                    let name = members.remove(&id).unwrap_or_else(|| self.state.member_name(&id));
                                    (ChangeKind::MemberRemoved, name)
                                }
                                TeamOp::Epoch { .. } | TeamOp::Share { .. } => continue,
                            };
                            changes.push(ChangeView { kind, coll: None, record: None, name, fields: BTreeMap::new(), secrets: vec![], member: None, level: None });
                        }
                    }
                    Payload::Record { record, section, ops } => {
                        for op in ops {
                            match op {
                                RecordOp::Set { coll, fields } => {
                                    if let Some(Value::String(n)) = fields.get(coll.name_field()) {
                                        names.insert(record.clone(), n.clone());
                                    }
                                    let c = record_change(&mut changes, &record, Some(coll), || {
                                        if deleted.remove(&record) {
                                            ChangeKind::Restored
                                        } else if known.insert(record.clone()) {
                                            ChangeKind::Created
                                        } else {
                                            ChangeKind::Changed
                                        }
                                    });
                                    let new = c.kind == ChangeKind::Created;
                                    for (k, val) in fields {
                                        if section == Section::Secret || coll.is_secret_field(&k) {
                                            // A new record without a password has none, it didn't change.
                                            if !(new && val.is_null()) {
                                                c.secrets.push(k);
                                            }
                                        } else {
                                            c.fields.insert(k, val);
                                        }
                                    }
                                }
                                RecordOp::Delete => {
                                    deleted.insert(record.clone());
                                    record_change(&mut changes, &record, None, || ChangeKind::Deleted).kind = ChangeKind::Deleted;
                                }
                                RecordOp::Grant { member, level, .. } => {
                                    let coll = self.state.records.get(&record).and_then(|r| r.coll);
                                    changes.push(ChangeView { kind: ChangeKind::Access, coll, record: Some(record.clone()), name: String::new(), fields: BTreeMap::new(), secrets: vec![], member: Some(members.get(&member).cloned().unwrap_or_else(|| self.state.member_name(&member))), level: Some(level) });
                                }
                                RecordOp::Rotate { .. } | RecordOp::Share { .. } => {}
                            }
                        }
                    }
                }
            }
            // Access handed out with a new record or to a new member is part of that change, not news.
            let created: HashSet<String> = changes.iter().filter(|c| c.kind == ChangeKind::Created).filter_map(|c| c.record.clone()).collect();
            let joined = changes.iter().any(|c| c.kind == ChangeKind::MemberAdded);
            changes.retain(|c| c.kind != ChangeKind::Access || !(joined || c.record.as_ref().is_some_and(|r| created.contains(r))));
            // Names as they were right after this entry.
            for c in &mut changes {
                if let Some(r) = &c.record {
                    if c.coll.is_none() {
                        c.coll = self.state.records.get(r).and_then(|r| r.coll);
                    }
                    if c.name.is_empty() {
                        c.name = names.get(r).cloned().unwrap_or_default();
                    }
                }
            }
            if changes.is_empty() {
                continue;
            }
            out.push(LogEntryView {
                entry: e.hash.clone(),
                at: b.hlc.ms,
                author: b.author.clone(),
                author_name: members.get(&b.author).cloned().unwrap_or_else(|| self.state.member_name(&b.author)),
                is_you: b.author == me,
                device: b.device.clone(),
                this_device: b.device == self.device,
                changes,
            });
        }
        Ok(out)
    }

    /// The versions of one record: every entry that changed it, oldest first.
    pub fn history(&self, record: &str) -> Result<Vec<LogEntryView>> {
        if !self.state.records.get(record).is_some_and(|r| self.visible(record, r)) {
            return Err(Error::NotFound { what: "record".into() });
        }
        Ok(self
            .log()?
            .into_iter()
            .filter_map(|mut e| {
                e.changes.retain(|c| c.record.as_deref() == Some(record));
                (!e.changes.is_empty()).then_some(e)
            })
            .collect())
    }

    /// Records this member can see that are deleted now, latest first.
    pub fn deleted(&self) -> Vec<DeletedView> {
        let me = self.my_id();
        let mut out: Vec<DeletedView> = self
            .state
            .records
            .iter()
            .filter(|(id, r)| r.is_deleted() && self.visible(id, r))
            .filter_map(|(id, r)| {
                let by = r.deleted.as_ref()?;
                Some(DeletedView { coll: r.coll?, record: id.clone(), name: r.name(), at: by.hlc.ms, author_name: self.state.member_name(&by.author), is_you: by.author == me })
            })
            .collect();
        out.sort_by(|a, b| b.at.cmp(&a.at));
        out
    }

    /// What going back would change.
    pub fn rollback_plan(&self, to: &Rollback) -> Result<RollbackPlan> {
        Ok(RollbackPlan { items: self.steps(to)?.into_iter().map(|s| s.item).collect() })
    }

    /// Goes back: writes one entry that puts every record this member may edit as it was.
    pub fn rollback(&mut self, to: &Rollback) -> Result<RollbackDone> {
        self.require_role(Role::Member)?;
        let steps = self.steps(to)?;
        let mut d = Draft::default();
        let (mut applied, mut skipped) = (0, 0);
        for s in steps {
            if !s.item.allowed {
                skipped += 1;
                continue;
            }
            applied += 1;
            let rid = &s.item.record;
            let coll = s.item.coll;
            match s.item.action {
                PlanAction::Remove => d.rec(rid, Section::Meta, RecordOp::Delete),
                PlanAction::Restore | PlanAction::Change => {
                    let changed: BTreeMap<String, Value> = s.item.fields.iter().filter_map(|f| s.values.get(&f.field).map(|v| (f.field.clone(), v.clone()))).collect();
                    let (mut meta, secret) = split(coll, changed);
                    // Coming back from a delete takes a write of the record itself, even with nothing to change.
                    if s.item.action == PlanAction::Restore {
                        meta = split(coll, s.values.clone()).0;
                    }
                    if !meta.is_empty() {
                        d.rec(rid, Section::Meta, RecordOp::Set { coll, fields: meta });
                    }
                    if !secret.is_empty() {
                        d.rec(rid, Section::Secret, RecordOp::Set { coll, fields: secret });
                    }
                }
            }
        }
        if d.is_empty() {
            return Ok(RollbackDone { applied: 0, skipped, entry: None });
        }
        self.write(d)?;
        Ok(RollbackDone { applied, skipped, entry: self.state.heads.get(&self.device).map(|h| h.hash.clone()) })
    }

    /// The state to go back to, and the one record it is limited to.
    fn target(&self, to: &Rollback, v: &Verified) -> Result<(State, Option<String>)> {
        let hashes = |keep: &dyn Fn(usize, &log::Entry) -> bool| -> HashSet<String> { v.entries.iter().enumerate().filter(|(i, e)| !keep(*i, e)).map(|(_, e)| e.hash.clone()).collect() };
        let position = |entry: &str| v.entries.iter().position(|e| e.hash == entry).ok_or_else(|| Error::NotFound { what: "entry".into() });
        let (skip, only) = match to {
            Rollback::At { at } => (hashes(&|_, e| e.body.hlc.ms <= *at), None),
            Rollback::Undo { entry } => {
                position(entry)?;
                (HashSet::from([entry.clone()]), None)
            }
            Rollback::Without { device, author, after } => {
                if device.is_none() && author.is_none() {
                    return Err(Error::Internal { message: "rollback: a device or a member is needed".into() });
                }
                let theirs = |e: &log::Entry| device.as_ref().is_none_or(|d| &e.body.device == d) && author.as_ref().is_none_or(|a| &e.body.author == a);
                (hashes(&|_, e| !(theirs(e) && e.body.hlc.ms > *after)), None)
            }
            Rollback::Record { record, entry } => {
                let at = position(entry)?;
                (hashes(&|i, _| i <= at), Some(record.clone()))
            }
            Rollback::Restore { record } => return Ok((self.state.clone(), Some(record.clone()))),
        };
        let root = PublicIdentity::parse(&self.meta.root)?.key_id();
        let mut drops = self.drops.clone();
        Ok((state::replay_without(v, &root, &self.me, &mut drops, &skip), only))
    }

    fn steps(&self, to: &Rollback) -> Result<Vec<Step>> {
        let v = self.verified_entries()?;
        let (then, only) = self.target(to, &v)?;
        let restore = matches!(to, Rollback::Restore { .. });
        let mut steps = Vec::new();
        for (rid, now) in &self.state.records {
            if only.as_ref().is_some_and(|o| o != rid) || !self.visible(rid, now) {
                continue;
            }
            let Some(coll) = now.coll else { continue };
            let was: Option<&Record> = if restore { Some(now) } else { then.records.get(rid).filter(|r| r.coll.is_some() && !r.fields.is_empty() && !r.is_deleted()) };
            let is_now = !now.is_deleted();
            let current = now.values();
            let (action, values, fields) = match (was, is_now) {
                (None, false) => continue,
                (None, true) => (PlanAction::Remove, BTreeMap::new(), vec![]),
                (Some(w), is_now) => {
                    let values = w.values();
                    // Only fields the record had then: a field added since keeps its value.
                    let fields: Vec<FieldDiff> = values
                        .iter()
                        .filter(|(k, val)| current.get(*k) != Some(*val))
                        .map(|(k, val)| {
                            let secret = coll.is_secret_field(k);
                            FieldDiff { field: k.clone(), secret, from: (!secret).then(|| current.get(k).cloned()).flatten(), to: (!secret).then(|| val.clone()) }
                        })
                        .collect();
                    match is_now {
                        false => (PlanAction::Restore, values, fields),
                        true if fields.is_empty() => continue,
                        true => (PlanAction::Change, values, fields),
                    }
                }
            };
            let name = was.map(|w| w.name()).filter(|n| !n.is_empty()).unwrap_or_else(|| now.name());
            let allowed = self.level(rid) >= Level::Edit;
            steps.push(Step { item: PlanItem { coll, record: rid.clone(), name, action, fields, allowed }, values });
        }
        Ok(steps)
    }
}

/// The change of `record` in this entry, added if not there yet.
fn record_change<'a>(changes: &'a mut Vec<ChangeView>, record: &str, coll: Option<Coll>, kind: impl FnOnce() -> ChangeKind) -> &'a mut ChangeView {
    if let Some(i) = changes.iter().position(|c| c.record.as_deref() == Some(record) && c.kind != ChangeKind::Access) {
        if coll.is_some() && changes[i].coll.is_none() {
            changes[i].coll = coll;
        }
        return &mut changes[i];
    }
    changes.push(ChangeView { kind: kind(), coll, record: Some(record.to_string()), name: String::new(), fields: BTreeMap::new(), secrets: vec![], member: None, level: None });
    changes.last_mut().unwrap()
}
