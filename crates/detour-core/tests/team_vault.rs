//! Team vaults with several people and devices sharing one folder.

use std::collections::BTreeMap;
use std::path::PathBuf;

use detour_core::Error;
use detour_core::identity::Identity;
use detour_core::model::{Auth, HostView, KeyAlgorithm, Snippet, VaultView};
use detour_core::store::VaultUpdate;
use detour_core::team::log::{self, Body, Folder, Hlc, Kind};
use detour_core::team::state::{Member, Op, Role};
use detour_core::team::{ConflictView, TeamVault};

fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("detour-team-{}", uuid_like()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)) + &format!("-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

fn host(id: &str, label: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: label.into(), address: "10.0.0.1".into(), port: 22, user: "root".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: None }
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
    TeamVault::open(&t.path, &info.root, who, device).unwrap()
}

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
    a.put(VaultUpdate { hosts: vec![host("h1", "api-1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();

    let bob = Identity::generate();
    let mut b = open(&t, &bob, "bob-laptop");
    assert!(!b.view().joined && !b.view().removed);
    assert!(b.view().data.hosts.is_empty());

    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, false).unwrap();
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
}

#[test]
fn storage_holds_no_plaintext() {
    let (t, mut a) = new_team();
    let mut h = host("h1", "super-secret-label");
    h.new_password = Some("hunter2-password".into());
    a.put(VaultUpdate { hosts: vec![h], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    for entry in walk(&t.path) {
        let text = String::from_utf8_lossy(&std::fs::read(&entry).unwrap()).to_string();
        assert!(!text.contains("super-secret-label") && !text.contains("hunter2") && !text.contains("Alice") && !text.contains("Acme"), "{}", entry.display());
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
    let bob = Identity::generate();
    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, false).unwrap();
    a.put(VaultUpdate { hosts: vec![host("h1", "api-1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    let mut b = open(&t, &bob, "bob-laptop");

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
    a.resolve_field(detour_core::team::state::Coll::Host, "h1", "port", pick).unwrap();
    b.reload().unwrap();
    for v in [&a, &b] {
        assert!(v.view().conflicts.is_empty());
        assert_eq!(v.view().data.hosts[0].port, 2300);
    }
}

#[test]
fn edit_versus_delete_is_a_conflict() {
    let (t, mut a) = new_team();
    let bob = Identity::generate();
    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, false).unwrap();
    a.put(VaultUpdate { hosts: vec![host("h1", "api-1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    let mut b = open(&t, &bob, "bob-laptop");

    a.put(VaultUpdate { hosts: vec![], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    let mut db = b.view().data;
    db.hosts[0].label = "api-1-renamed".into();
    b.put(update_from(&db)).unwrap();
    a.reload().unwrap();

    let view = a.view();
    assert_eq!(view.data.hosts.len(), 1, "edit made without seeing the delete keeps the host");
    assert!(matches!(&view.conflicts[0], ConflictView::Deleted { by_name, .. } if by_name == "Alice"));

    a.resolve_deleted(detour_core::team::state::Coll::Host, "h1", false).unwrap();
    b.reload().unwrap();
    assert!(b.view().data.hosts.is_empty());
    assert!(b.view().conflicts.is_empty());
}

#[test]
fn undo_delete_restores_host_with_its_password() {
    let (_t, mut a) = new_team();
    let mut h = host("h1", "api-1");
    h.new_password = Some("pw".into());
    a.put(VaultUpdate { hosts: vec![h.clone()], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    a.put(VaultUpdate { hosts: vec![], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    assert!(a.view().data.hosts.is_empty());
    h.new_password = None;
    a.put(VaultUpdate { hosts: vec![h], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    assert_eq!(a.host("h1").unwrap().password.as_deref(), Some("pw"));
}

#[test]
fn roles_are_enforced_by_every_reader() {
    let (t, mut a) = new_team();
    let bob = Identity::generate();
    let carol = Identity::generate();
    a.add_member(&bob.public().detour_id(), "Bob", Role::ReadOnly, false).unwrap();
    let mut b = open(&t, &bob, "bob-laptop");

    // The client refuses...
    assert!(matches!(b.put(VaultUpdate { hosts: vec![host("x", "x")], keys: vec![], snippets: vec![], tunnels: vec![] }), Err(Error::Forbidden)));
    assert!(matches!(b.add_member(&carol.public().detour_id(), "Carol", Role::Owner, true), Err(Error::Forbidden)));

    // ...and a forged entry written straight to storage is ignored by everyone.
    let folder = Folder::new(&t.path);
    let (epoch, key) = epoch_key_of(&t.path, &bob);
    let ops = vec![
        Op::MemberPut { member: Member { id: carol.public().key_id(), detour_id: carol.public().detour_id(), name: "Carol".into(), role: Role::Owner, can_reveal: true, recovery: false } },
        Op::Set { coll: detour_core::team::state::Coll::Snippet, id: "s".into(), fields: BTreeMap::from([("name".into(), "evil".into()), ("command".into(), "rm -rf /".into())]) },
    ];
    let body = Body {
        v: 1,
        vault: a.id().into(),
        device: "bob-forge".into(),
        author: bob.public().key_id(),
        seq: 1,
        prev: None,
        hlc: Hlc { ms: u64::MAX / 2, c: 0 },
        seen: BTreeMap::new(),
        kind: Kind::Data,
        epoch,
        payload: log::seal_data(&key, &serde_json::to_vec(&ops).unwrap()).unwrap(),
    };
    folder.write_log("bob-forge", 1, &log::sign(&body, &bob).unwrap()).unwrap();
    a.reload().unwrap();
    let view = a.view();
    assert_eq!(view.members.len(), 3, "Carol must not have been added");
    assert!(view.data.snippets.is_empty());
    assert!(view.problems.len() >= 2);
}

/// The latest epoch key `who` can open, read straight from storage.
fn epoch_key_of(path: &PathBuf, who: &Identity) -> (u32, [u8; 32]) {
    let mut best = None;
    for (_, files) in Folder::new(path).read_logs().unwrap() {
        for (_, bytes) in files {
            let e = log::verify(&bytes).unwrap();
            if e.body.kind == Kind::Keys
                && let Some(keys) = log::open_keys(&e.body.payload, who)
            {
                best = keys.into_iter().next_back();
            }
        }
    }
    best.expect("a key for this member")
}

#[test]
fn tampered_files_are_rejected() {
    let (t, mut a) = new_team();
    a.put(VaultUpdate { hosts: vec![host("h1", "api-1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    let file = t.path.join("ops").join("alice-pc").join("00000003.json");
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
    let bob = Identity::generate();
    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, true).unwrap();
    let bob_id = bob.public().key_id();
    a.remove_member(&bob_id).unwrap();
    a.put(VaultUpdate { hosts: vec![host("h2", "after-removal")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();

    let b = open(&t, &bob, "bob-laptop");
    let view = b.view();
    assert!(!view.joined && view.removed);
    assert!(view.data.hosts.iter().all(|h| h.label != "after-removal"));

    // Bob can't write either: everything he signs now is from a non-member.
    a.reload().unwrap();
    assert_eq!(a.view().members.len(), 2);
}

#[test]
fn reveal_permission_guards_key_export() {
    let (t, mut a) = new_team();
    let key = a.generate_key("deploy", KeyAlgorithm::Ed25519, None, false).unwrap();
    let bob = Identity::generate();
    a.add_member(&bob.public().detour_id(), "Bob", Role::Member, false).unwrap();
    let mut b = open(&t, &bob, "bob-laptop");
    assert!(b.key(&key.id).is_some(), "Bob can use the key");
    assert!(matches!(b.revealable_key(&key.id), Err(Error::Forbidden)));
    assert!(a.revealable_key(&key.id).is_ok());

    a.update_member(&bob.public().key_id(), Role::Member, true).unwrap();
    b.reload().unwrap();
    assert!(b.revealable_key(&key.id).is_ok());
}

#[test]
fn recovery_key_brings_back_an_owner() {
    let (t, mut a) = new_team();
    a.put(VaultUpdate { hosts: vec![host("h1", "api-1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    let key_text = t.recovery.to_recovery_key();

    // Alice lost everything; with a new identity she uses the recovery key.
    let alice2 = Identity::generate();
    TeamVault::recover(&t.path, &key_text, &alice2.public().detour_id(), "Alice").unwrap();
    let v = open(&t, &alice2, "alice-new-pc");
    let view = v.view();
    assert_eq!(view.my_role, Some(Role::Owner));
    assert_eq!(view.data.hosts[0].label, "api-1");

    assert!(TeamVault::recover(&t.path, &Identity::generate().to_recovery_key(), &alice2.public().detour_id(), "x").is_err());
}

#[test]
fn joining_rejects_a_swapped_creator() {
    let (t, _a) = new_team();
    let other = Identity::generate();
    assert!(matches!(TeamVault::open(&t.path, &other.public().detour_id(), &other, "d"), Err(Error::RootChanged)));
}

#[test]
fn snippets_roundtrip() {
    let (_t, mut a) = new_team();
    let s = Snippet { id: "s1".into(), name: "Disk".into(), command: "df -h".into() };
    a.put(VaultUpdate { hosts: vec![], keys: vec![], snippets: vec![s], tunnels: vec![] }).unwrap();
    assert_eq!(a.view().data.snippets[0].command, "df -h");
}
