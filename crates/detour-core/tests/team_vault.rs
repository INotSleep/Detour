//! Team vaults with several people and devices sharing one folder.
//!
//! Many tests act as a compromised client: they read the storage directly
//! with a member's identity, or write signed entries no honest Detour would.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use detour_core::Error;
use detour_core::identity::Identity;
use detour_core::model::{Auth, HostView, KeyAlgorithm, Snippet, VaultView};
use detour_core::store::VaultUpdate;
use detour_core::team::log::{self, Body, Folder, Head, Hlc, Key, Part};
use detour_core::team::state::{Coll, Level, Member, Payload, Problem, RecordOp, Role, Section, TeamOp};
use detour_core::team::{AccessChange, ConflictView, TeamVault};

fn temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-team-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(id: &str, label: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: label.into(), address: "10.0.0.1".into(), port: 22, user: "root".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None }
}

fn with_password(mut h: HostView, pw: &str) -> HostView {
    h.new_password = Some(pw.into());
    h
}

fn hosts(hs: Vec<HostView>) -> VaultUpdate {
    VaultUpdate { hosts: hs, keys: vec![], snippets: vec![], tunnels: vec![] }
}

fn update_from(data: &VaultView) -> VaultUpdate {
    VaultUpdate { hosts: data.hosts.clone(), keys: data.keys.clone(), snippets: data.snippets.clone(), tunnels: data.tunnels.clone() }
}

struct Team {
    path: PathBuf,
    alice: Identity,
    recovery: Identity,
}

fn new_team() -> (Team, TeamVault) {
    let path = temp_dir().join("vault");
    let alice = Identity::generate();
    let recovery = Identity::generate();
    let v = TeamVault::create(&path, "Acme", &alice, "Alice", "alice-pc", &recovery).unwrap();
    (Team { path, alice, recovery }, v)
}

fn open(t: &Team, who: &Identity, device: &str) -> TeamVault {
    let info = TeamVault::inspect(&t.path).unwrap();
    TeamVault::open(&t.path, &info.root, who, device, BTreeMap::new()).unwrap()
}

/// Adds a new person and opens the vault as them.
fn join(t: &Team, admin: &mut TeamVault, name: &str, role: Role, access: Level) -> (Identity, TeamVault) {
    let who = Identity::generate();
    admin.add_member(&who.public().detour_id(), name, role, access).unwrap();
    let v = open(t, &who, &format!("{name}-pc"));
    (who, v)
}

fn set(v: &mut TeamVault, record: &str, member: &Identity, level: Level) {
    v.set_access(&[AccessChange { record: record.into(), member: member.public().key_id(), level }]).unwrap();
}

/* ---------- reading storage like a compromised client ---------- */

/// Every key `who` was handed and every plaintext those keys open, straight from the files.
struct Raw {
    keys: Vec<Key>,
    plain: Vec<String>,
}

fn raw(path: &Path, who: &Identity) -> Raw {
    let scan = Folder::new(path).scan().unwrap();
    let mut keys = Vec::new();
    let mut parts = Vec::new();
    for files in scan.logs.values() {
        for (_, bytes) in files {
            for p in log::verify(bytes).unwrap().body.parts {
                match p {
                    Part::Drop { ct } => keys.extend(log::open_drop(&ct, who).unwrap_or_default()),
                    Part::Data { kid, ct } => parts.push((kid, ct)),
                }
            }
        }
    }
    let plain = parts.iter().filter_map(|(kid, ct)| keys.iter().find_map(|k| log::open_data(k, kid, ct))).map(|p| String::from_utf8_lossy(&p).to_string()).collect();
    Raw { keys, plain }
}

impl Raw {
    fn sees(&self, needle: &str) -> bool {
        self.plain.iter().any(|p| p.contains(needle))
    }

    /// A key that opens a part about `record`'s `section`.
    fn key_for(&self, path: &Path, record: &str, section: Section) -> Key {
        let scan = Folder::new(path).scan().unwrap();
        for files in scan.logs.values() {
            for (_, bytes) in files {
                for p in log::verify(bytes).unwrap().body.parts {
                    let Part::Data { kid, ct } = p else { continue };
                    for k in &self.keys {
                        if let Some(plain) = log::open_data(k, &kid, &ct)
                            && let Ok(Payload::Record { record: r, section: s, .. }) = serde_json::from_slice::<Payload>(&plain)
                            && r == record
                            && s == section
                        {
                            return *k;
                        }
                    }
                }
            }
        }
        panic!("no key for {record}");
    }

    fn team_key(&self, path: &Path) -> Key {
        let scan = Folder::new(path).scan().unwrap();
        let mut found = None;
        for files in scan.logs.values() {
            for (_, bytes) in files {
                for p in log::verify(bytes).unwrap().body.parts {
                    let Part::Data { kid, ct } = p else { continue };
                    for k in &self.keys {
                        if let Some(plain) = log::open_data(k, &kid, &ct)
                            && matches!(serde_json::from_slice::<Payload>(&plain), Ok(Payload::Team { .. }))
                        {
                            found = Some(*k);
                        }
                    }
                }
            }
        }
        found.expect("a team key")
    }
}

/// The last entry a device wrote: (seq, hash, hlc).
fn device_head(path: &Path, device: &str) -> Option<(u64, String, Hlc)> {
    let scan = Folder::new(path).scan().unwrap();
    let (seq, bytes) = scan.logs.get(device)?.last()?.clone();
    let e = log::verify(&bytes).unwrap();
    Some((seq, e.hash, e.body.hlc))
}

/// Signs and stores an entry exactly as `who` wants it.
fn forge(path: &Path, vault: &str, who: &Identity, device: &str, hlc: Hlc, parts: Vec<Part>) {
    let (seq, prev) = match device_head(path, device) {
        Some((s, h, _)) => (s + 1, Some(h)),
        None => (1, None),
    };
    let body = Body { v: log::FORMAT, vault: vault.into(), device: device.into(), author: who.public().key_id(), seq, prev, hlc, seen: BTreeMap::new(), parts };
    Folder::new(path).write_log(device, seq, &log::sign(&body, who).unwrap()).unwrap();
}

fn sealed(key: &Key, payload: &Payload) -> Part {
    log::seal_data(key, &serde_json::to_vec(payload).unwrap()).unwrap()
}

fn has_problem(v: &TeamVault, f: impl Fn(&Problem) -> bool) -> bool {
    v.view().problems.iter().any(|p| f(&p.problem))
}

/* ---------- basics ---------- */

#[test]
fn creator_is_owner_and_recovery_key_is_second_owner() {
    let (t, v) = new_team();
    let view = v.view();
    assert_eq!(view.name, "Acme");
    assert!(view.joined);
    assert_eq!(view.my_role, Some(Role::Owner));
    assert_eq!(view.members.len(), 2);
    assert!(view.members.iter().any(|m| m.recovery && m.role == Role::Owner));
    assert_eq!(TeamVault::inspect(&t.path).unwrap().root_fingerprint, t.alice.public().fingerprint());
    assert!(view.problems.is_empty(), "{:?}", view.problems);
}

#[test]
fn newcomer_reads_full_history_after_being_added() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();

    let bob = Identity::generate();
    let mut b = open(&t, &bob, "bob-laptop");
    assert!(!b.view().joined && !b.view().removed);
    assert!(b.view().data.hosts.is_empty());

    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, Level::Edit).unwrap();
    b.reload().unwrap();
    let view = b.view();
    assert!(view.joined);
    assert_eq!(view.my_role, Some(Role::Member));
    assert_eq!(view.data.hosts[0].label, "api-1");

    // Bob's edits reach Alice.
    let mut data = view.data.clone();
    data.hosts[0].port = 2222;
    b.put(update_from(&data)).unwrap();
    a.reload().unwrap();
    assert_eq!(a.view().data.hosts[0].port, 2222);
    assert!(a.view().problems.is_empty(), "{:?}", a.view().problems);
}

#[test]
fn storage_holds_no_plaintext() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("plain-host-id", "super-secret-label"), "hunter2-password")])).unwrap();
    for entry in walk(&t.path) {
        let text = String::from_utf8_lossy(&std::fs::read(&entry).unwrap()).to_string();
        assert!(!text.contains("super-secret-label") && !text.contains("hunter2") && !text.contains("Alice") && !text.contains("Acme") && !text.contains("plain-host-id"), "{}", entry.display());
    }
}

fn walk(p: &PathBuf) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(p).unwrap() {
        let e = e.unwrap().path();
        if e.is_dir() { out.extend(walk(&e)) } else { out.push(e) }
    }
    out
}

#[test]
fn concurrent_edits_merge_and_same_field_conflicts_until_resolved() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();
    let (_bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::Edit);

    // Both edit without seeing each other: different fields merge, same field conflicts.
    let mut da = a.view().data;
    da.hosts[0].port = 2200;
    da.hosts[0].user = "alice-user".into();
    let mut db = b.view().data;
    db.hosts[0].port = 2300;
    db.hosts[0].tags = vec!["prod".into()];
    a.put(update_from(&da)).unwrap();
    b.put(update_from(&db)).unwrap();
    a.reload().unwrap();
    b.reload().unwrap();

    for v in [&a, &b] {
        let view = v.view();
        let h = &view.data.hosts[0];
        assert_eq!(h.user, "alice-user");
        assert_eq!(h.tags, vec!["prod".to_string()]);
        assert_eq!(view.conflicts.len(), 1, "{:?}", view.conflicts);
        match &view.conflicts[0] {
            ConflictView::Field { field, values, .. } => {
                assert_eq!(field, "port");
                assert_eq!(values.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    // Alice picks Bob's port; everyone converges.
    let pick = match &a.view().conflicts[0] {
        ConflictView::Field { values, .. } => values.iter().position(|v| v.author_name == "Bob").unwrap(),
        _ => unreachable!(),
    };
    a.resolve_field(Coll::Host, "h1", "port", pick).unwrap();
    b.reload().unwrap();
    for v in [&a, &b] {
        assert!(v.view().conflicts.is_empty());
        assert_eq!(v.view().data.hosts[0].port, 2300);
    }
}

#[test]
fn edit_versus_delete_is_a_conflict() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();
    let (_bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::Edit);

    a.put(hosts(vec![])).unwrap();
    let mut db = b.view().data;
    db.hosts[0].label = "api-1-renamed".into();
    b.put(update_from(&db)).unwrap();
    a.reload().unwrap();

    let view = a.view();
    assert_eq!(view.data.hosts.len(), 1, "edit made without seeing the delete keeps the host");
    assert!(matches!(&view.conflicts[0], ConflictView::Deleted { by_name, .. } if by_name == "Alice"));

    a.resolve_deleted(Coll::Host, "h1", false).unwrap();
    b.reload().unwrap();
    assert!(b.view().data.hosts.is_empty());
    assert!(b.view().conflicts.is_empty());
}

#[test]
fn undo_delete_restores_host_with_its_password() {
    let (_t, mut a) = new_team();
    let h = with_password(host("h1", "api-1"), "pw");
    a.put(hosts(vec![h.clone()])).unwrap();
    a.put(hosts(vec![])).unwrap();
    assert!(a.view().data.hosts.is_empty());
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();
    assert_eq!(a.host("h1").unwrap().password.as_deref(), Some("pw"));
}

#[test]
fn activity_names_records_as_they_were() {
    let (_t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "team-box")])).unwrap();
    a.put(hosts(vec![host("h1", "renamed")])).unwrap();
    let names: Vec<String> = a.view().activity.iter().filter_map(|x| serde_json::to_value(&x.activity).ok()?.get("name")?.as_str().map(String::from)).collect();
    assert_eq!(names, vec!["renamed".to_string(), "team-box".to_string()]);
}

#[test]
fn tampered_files_are_rejected() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();
    let file = t.path.join("ops").join("alice-pc").join("00000002.json");
    let mut bytes = std::fs::read(&file).unwrap();
    let i = bytes.len() / 2;
    bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
    std::fs::write(&file, bytes).unwrap();
    a.reload().unwrap();
    let view = a.view();
    assert!(view.data.hosts.is_empty());
    assert!(!view.problems.is_empty());
}

#[test]
fn removed_member_cannot_read_new_changes() {
    let (t, mut a) = new_team();
    let (bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::Edit);
    a.put(hosts(vec![host("h1", "before")])).unwrap();
    b.reload().unwrap();
    a.remove_member(&bob.public().key_id()).unwrap();
    a.put(hosts(vec![host("h1", "before"), with_password(host("h2", "after-removal"), "new-secret")])).unwrap();
    let mut data = a.view().data;
    data.hosts[0].label = "renamed-after".into();
    a.put(update_from(&data)).unwrap();

    b.reload().unwrap();
    let view = b.view();
    assert!(!view.joined && view.removed);
    let r = raw(&t.path, &bob);
    assert!(r.sees("before"), "what Bob had before stays readable to him");
    assert!(!r.sees("after-removal") && !r.sees("new-secret") && !r.sees("renamed-after"), "nothing written after the removal");
}

#[test]
fn recovery_key_brings_back_an_owner() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("h1", "api-1"), "pw")])).unwrap();
    let key_text = t.recovery.to_recovery_key();

    // Alice lost everything; with a new identity she uses the recovery key.
    let alice2 = Identity::generate();
    TeamVault::recover(&t.path, &key_text, &alice2.public().detour_id(), "Alice").unwrap();
    let v = open(&t, &alice2, "alice-new-pc");
    let view = v.view();
    assert_eq!(view.my_role, Some(Role::Owner));
    assert_eq!(view.data.hosts[0].label, "api-1");
    assert_eq!(v.host_password("h1").unwrap().as_deref(), Some("pw"));

    assert!(TeamVault::recover(&t.path, &Identity::generate().to_recovery_key(), &alice2.public().detour_id(), "x").is_err());
}

#[test]
fn joining_rejects_a_swapped_creator() {
    let (t, _a) = new_team();
    let other = Identity::generate();
    assert!(matches!(TeamVault::open(&t.path, &other.public().detour_id(), &other, "d", BTreeMap::new()), Err(Error::RootChanged)));
}

#[test]
fn snippets_roundtrip() {
    let (_t, mut a) = new_team();
    let s = Snippet { id: "s1".into(), name: "Disk".into(), command: "df -h".into() };
    a.put(VaultUpdate { hosts: vec![], keys: vec![], snippets: vec![s], tunnels: vec![] }).unwrap();
    assert_eq!(a.view().data.snippets[0].command, "df -h");
}

/* ---------- access is enforced by encryption ---------- */

#[test]
fn view_access_never_hands_over_secrets() {
    let (t, mut a) = new_team();
    let key = a.generate_key("deploy", KeyAlgorithm::Ed25519, None, false).unwrap();
    let mut h2 = host("h2", "key-box");
    h2.auth = Auth::Key { key_id: key.id.clone() };
    a.put(VaultUpdate { hosts: vec![with_password(host("h1", "pw-box"), "hunter2-password"), h2], keys: vec![key.clone()], snippets: vec![], tunnels: vec![] }).unwrap();
    let private = a.secret_key(&key.id).unwrap().private_key;
    let (bob, b) = join(&t, &mut a, "Bob", Role::Member, Level::View);

    // The honest client shows the hosts and refuses the secrets...
    assert_eq!(b.view().data.hosts.len(), 2);
    assert!(matches!(b.host_password("h1"), Err(Error::Forbidden)));
    assert!(matches!(b.secret_key(&key.id), Err(Error::Forbidden)));
    // ...and a client reading the files itself finds the metadata but no secret anywhere.
    let r = raw(&t.path, &bob);
    assert!(r.sees("pw-box") && r.sees("key-box"));
    assert!(!r.sees("hunter2-password"));
    assert!(!r.sees("PRIVATE KEY") && !r.plain.iter().any(|p| p.contains(&private[40..80])));
}

#[test]
fn no_access_means_not_knowing_the_record_exists() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("hidden-host-id", "hidden-box")])).unwrap();
    let (bob, b) = join(&t, &mut a, "Bob", Role::Member, Level::None);
    assert!(b.view().data.hosts.is_empty());
    let r = raw(&t.path, &bob);
    assert!(!r.sees("hidden-box") && !r.sees("hidden-host-id"));
    assert!(r.sees("Alice"), "the member list is shared with every member");
}

#[test]
fn secret_access_reveals_but_does_not_edit() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("h1", "api-1"), "pw")])).unwrap();
    let (bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::Secret);
    assert_eq!(b.host_password("h1").unwrap().as_deref(), Some("pw"));

    let mut data = b.view().data;
    data.hosts[0].label = "bob-was-here".into();
    assert!(matches!(b.put(update_from(&data)), Err(Error::Forbidden)));

    // Written past the client anyway: every reader ignores it.
    let r = raw(&t.path, &bob);
    let meta = r.key_for(&t.path, "h1", Section::Meta);
    let op = RecordOp::Set { coll: Coll::Host, fields: BTreeMap::from([("label".into(), "bob-was-here".into())]) };
    forge(&t.path, a.id(), &bob, "Bob-pc", Hlc { ms: u64::MAX / 2, c: 0 }, vec![sealed(&meta, &Payload::Record { record: "h1".into(), section: Section::Meta, ops: vec![op] })]);
    a.reload().unwrap();
    assert_eq!(a.view().data.hosts[0].label, "api-1");
    assert!(has_problem(&a, |p| matches!(p, Problem::Rejected { reason, .. } if reason == "edit without access")));
}

#[test]
fn lowering_access_rotates_keys() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("h1", "api-1"), "old-pw")])).unwrap();
    let (bob, _b) = join(&t, &mut a, "Bob", Role::Member, Level::Secret);
    set(&mut a, "h1", &bob, Level::View);
    a.set_host_password("h1", Some("new-pw".into())).unwrap();

    let r = raw(&t.path, &bob);
    assert!(r.sees("old-pw"), "what he could see before stays seen");
    assert!(!r.sees("new-pw"));
    assert!(r.sees("api-1"), "he keeps the metadata");
}

#[test]
fn members_and_admins_cannot_raise_their_own_rights() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("h1", "api-1"), "pw")])).unwrap();
    let carol = Identity::generate();
    let (bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::View);
    let (dave, mut d) = join(&t, &mut a, "Dave", Role::Admin, Level::View);
    let _ = b.reload();

    // The clients refuse...
    assert!(matches!(b.add_member(&carol.public().detour_id(), "Carol", Role::Owner, Level::Edit), Err(Error::Forbidden)));
    assert!(matches!(d.set_access(&[AccessChange { record: "h1".into(), member: bob.public().key_id(), level: Level::Secret }]), Err(Error::Forbidden)));

    // ...and forged entries change nothing for anyone.
    let rb = raw(&t.path, &bob);
    let team_key = rb.team_key(&t.path);
    let meta = rb.key_for(&t.path, "h1", Section::Meta);
    let carol_m = Member { id: carol.public().key_id(), detour_id: carol.public().detour_id(), name: "Carol".into(), role: Role::Owner, recovery: false };
    let bob_up = Member { id: bob.public().key_id(), detour_id: bob.public().detour_id(), name: "Bob".into(), role: Role::Owner, recovery: false };
    forge(&t.path, a.id(), &bob, "bob-forge", Hlc { ms: u64::MAX / 2, c: 0 }, vec![
        sealed(&team_key, &Payload::Team { ops: vec![TeamOp::MemberPut { member: carol_m, cutoff: Default::default() }, TeamOp::MemberPut { member: bob_up, cutoff: Default::default() }] }),
        sealed(&meta, &Payload::Record { record: "h1".into(), section: Section::Meta, ops: vec![RecordOp::Grant { member: bob.public().key_id(), level: Level::Edit, cutoff: Default::default() }] }),
    ]);
    let dave_meta = raw(&t.path, &dave).key_for(&t.path, "h1", Section::Meta);
    forge(&t.path, a.id(), &dave, "dave-forge", Hlc { ms: u64::MAX / 2, c: 1 }, vec![sealed(&dave_meta, &Payload::Record { record: "h1".into(), section: Section::Meta, ops: vec![RecordOp::Grant { member: dave.public().key_id(), level: Level::Edit, cutoff: Default::default() }] })]);

    a.reload().unwrap();
    let view = a.view();
    assert_eq!(view.members.len(), 4, "Carol must not have been added");
    assert_eq!(view.members.iter().find(|m| m.name == "Bob").unwrap().role, Role::Member);
    assert_eq!(a.view().access.iter().find(|x| x.member == bob.public().key_id()).unwrap().level, Level::View);
    assert_eq!(a.view().access.iter().find(|x| x.member == dave.public().key_id()).unwrap().level, Level::View);
    assert!(view.problems.len() >= 3, "{:?}", view.problems);
    // Even "granting" himself Secret would not have given Bob the key: only holders can hand it over.
    assert!(!raw(&t.path, &bob).sees("\"pw\""));
}

#[test]
fn removed_member_cannot_backdate_changes() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "api-1")])).unwrap();
    let (bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::Edit);
    let mut data = b.view().data;
    data.hosts[0].port = 2201;
    b.put(update_from(&data)).unwrap();
    let (_, _, bob_time) = device_head(&t.path, "Bob-pc").unwrap();
    let meta = raw(&t.path, &bob).key_for(&t.path, "h1", Section::Meta);

    a.reload().unwrap();
    a.remove_member(&bob.public().key_id()).unwrap();

    // Bob signs an edit dated before his removal, from his device and from a new one.
    let op = || RecordOp::Set { coll: Coll::Host, fields: BTreeMap::from([("label".into(), "backdated".into())]) };
    let early = Hlc { ms: bob_time.ms, c: bob_time.c + 1 };
    forge(&t.path, a.id(), &bob, "Bob-pc", early, vec![sealed(&meta, &Payload::Record { record: "h1".into(), section: Section::Meta, ops: vec![op()] })]);
    forge(&t.path, a.id(), &bob, "bob-new", early, vec![sealed(&meta, &Payload::Record { record: "h1".into(), section: Section::Meta, ops: vec![op()] })]);

    a.reload().unwrap();
    assert_eq!(a.view().data.hosts[0].label, "api-1");
    assert_eq!(a.view().data.hosts[0].port, 2201, "what he did before the removal stays");
    assert!(has_problem(&a, |p| matches!(p, Problem::Rejected { reason, .. } if reason == "written after losing access")));
}

#[test]
fn a_hosts_secret_brings_its_key_along() {
    let (t, mut a) = new_team();
    let key = a.generate_key("deploy", KeyAlgorithm::Ed25519, None, false).unwrap();
    let mut h = host("h1", "key-box");
    h.auth = Auth::Key { key_id: key.id.clone() };
    a.put(VaultUpdate { hosts: vec![h], keys: vec![key.clone()], snippets: vec![], tunnels: vec![] }).unwrap();
    let (bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::None);

    set(&mut a, "h1", &bob, Level::Secret);
    b.reload().unwrap();
    assert!(b.secret_key(&key.id).is_ok());
    assert_eq!(b.level(&key.id), Level::Secret);

    set(&mut a, "h1", &bob, Level::View);
    b.reload().unwrap();
    assert_eq!(b.level(&key.id), Level::View);
    assert!(b.secret_key(&key.id).is_err());
}

#[test]
fn keys_are_rotated_by_whoever_holds_them_next() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![with_password(host("h1", "api-1"), "pw")])).unwrap();
    let (bob, _b) = join(&t, &mut a, "Bob", Role::Member, Level::Secret);
    // Dave is an admin who can only see the host: he can't rotate its secret key himself.
    let (_dave, mut d) = join(&t, &mut a, "Dave", Role::Admin, Level::View);
    d.remove_member(&bob.public().key_id()).unwrap();

    // Alice holds the secret: her next sync rotates it, before anything new is written there.
    a.reload().unwrap();
    assert!(a.maintain().unwrap());
    a.set_host_password("h1", Some("after".into())).unwrap();
    assert!(!raw(&t.path, &bob).sees("after"));
    assert!(a.view().problems.is_empty(), "{:?}", a.view().problems);
}

/* ---------- the storage can't hide anything silently ---------- */

#[test]
fn rollback_is_detected() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "one")])).unwrap();
    a.put(hosts(vec![host("h1", "two")])).unwrap();
    let (_bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::View);
    assert_eq!(b.view().data.hosts[0].label, "two");

    // The storage drops Alice's latest entries.
    let (last, _, _) = device_head(&t.path, "alice-pc").unwrap();
    std::fs::remove_file(t.path.join("ops/alice-pc").join(format!("{last:08}.json"))).unwrap();
    b.reload().unwrap();
    assert!(has_problem(&b, |p| matches!(p, Problem::RolledBack { device, .. } if device == "alice-pc")));

    // Bob's remembered heads survive a restart.
    let info = TeamVault::inspect(&t.path).unwrap();
    let again = TeamVault::open(&t.path, &info.root, &_bob, "Bob-pc", b.known_heads().clone()).unwrap();
    assert!(has_problem(&again, |p| matches!(p, Problem::RolledBack { .. })));
}

#[test]
fn hiding_an_acknowledged_entry_is_detected_by_anyone() {
    let (t, mut a) = new_team();
    let (_bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::View);
    a.put(hosts(vec![host("h1", "one")])).unwrap();
    b.reload().unwrap();
    assert!(b.maintain().unwrap(), "Bob acknowledges what he read");

    let (last, _, _) = device_head(&t.path, "alice-pc").unwrap();
    std::fs::remove_file(t.path.join("ops/alice-pc").join(format!("{last:08}.json"))).unwrap();
    // A brand new reader, with nothing remembered, still sees the hole.
    let carol = Identity::generate();
    let c = open(&t, &carol, "carol-pc");
    assert!(has_problem(&c, |p| matches!(p, Problem::Missing { device, seq, .. } if device == "alice-pc" && *seq == last)));
}

#[test]
fn stray_files_and_second_versions_are_reported() {
    let (t, mut a) = new_team();
    a.put(hosts(vec![host("h1", "one")])).unwrap();
    std::fs::create_dir_all(t.path.join(".stfolder")).unwrap();
    std::fs::write(t.path.join("ops/alice-pc/00000002.json.tmp"), b"half").unwrap();
    a.reload().unwrap();
    assert!(a.view().problems.is_empty(), "sync debris is fine: {:?}", a.view().problems);

    std::fs::write(t.path.join("notes.txt"), b"hello").unwrap();
    // Alice's device "also" signed a different entry 2, kept as a sync conflict copy.
    let real = log::verify(&std::fs::read(t.path.join("ops/alice-pc/00000002.json")).unwrap()).unwrap();
    let mut other = real.body.clone();
    other.hlc.c += 1;
    other.parts.clear();
    std::fs::write(t.path.join("ops/alice-pc/00000002.sync-conflict-20261023-ABC.json"), log::sign(&other, &t.alice).unwrap()).unwrap();
    a.reload().unwrap();
    assert!(has_problem(&a, |p| matches!(p, Problem::UnknownFile { path } if path == "notes.txt")));
    assert!(has_problem(&a, |p| matches!(p, Problem::Forked { device, seq, .. } if device == "alice-pc" && *seq == 2)));
}

#[test]
fn heads_include_hashes_so_a_swapped_entry_shows() {
    let (t, mut a) = new_team();
    let (_bob, mut b) = join(&t, &mut a, "Bob", Role::Member, Level::View);
    a.put(hosts(vec![host("h1", "one")])).unwrap();
    b.reload().unwrap();
    b.maintain().unwrap();
    let _ = Head { seq: 0, hash: String::new() };

    // Replace Alice's last entry by another valid one she signed (equivocation).
    let (last, _, _) = device_head(&t.path, "alice-pc").unwrap();
    let path = t.path.join("ops/alice-pc").join(format!("{last:08}.json"));
    let mut body = log::verify(&std::fs::read(&path).unwrap()).unwrap().body;
    body.parts.clear();
    std::fs::write(&path, log::sign(&body, &t.alice).unwrap()).unwrap();
    let carol = Identity::generate();
    let c = open(&t, &carol, "carol-pc");
    assert!(has_problem(&c, |p| matches!(p, Problem::Forked { device, .. } if device == "alice-pc")));
}
