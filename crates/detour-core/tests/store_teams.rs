//! Two people with their own Detour stores sharing a team folder.

use std::path::PathBuf;

use detour_core::model::{Auth, HostView, KeyAlgorithm, OwnLoginView};
use detour_core::store::{Store, VaultUpdate};
use detour_core::team::AccessChange;
use detour_core::team::state::{Level, Problem, Role};
use detour_core::{Error, SecretKind};

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-st-{tag}-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(id: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: Some("pw".into()), own: None }
}

/// Alice with a team holding one host with a shared password, and Bob joined with `access`.
fn team_with_bob(access: Level) -> (Store, Store, String) {
    let shared = temp_dir("shared").join("acme");
    let mut alice = Store::open(temp_dir("alice"));
    alice.create("alice-master").unwrap();
    let mut bob = Store::open(temp_dir("bob"));
    bob.create("bob-master").unwrap();

    let recovery = Store::new_recovery_key();
    let team = alice.create_team(shared.to_str().unwrap(), "Acme", "Alice", &recovery).unwrap();
    alice.team_put(&team.id, VaultUpdate { hosts: vec![host("h1")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();

    // Bob joins: he confirms the creator by fingerprint, then waits to be added.
    let info = Store::inspect_team(shared.to_str().unwrap()).unwrap();
    assert_eq!(info.root_fingerprint, alice.my_id().unwrap().fingerprint);
    let view = bob.join_team(shared.to_str().unwrap(), &info.root).unwrap();
    assert!(!view.joined);

    let bob_id = bob.my_id().unwrap();
    assert_eq!(Store::inspect_id(&bob_id.detour_id).unwrap(), bob_id.fingerprint);
    alice.team_add_member(&team.id, &bob_id.detour_id, "Bob", Role::Member, access).unwrap();
    (alice, bob, team.id)
}

#[test]
fn secret_access_connects_with_the_shared_password_and_survives_relock() {
    let (_alice, mut bob, id) = team_with_bob(Level::Secret);
    let view = bob.sync_team(&id).unwrap();
    assert!(view.joined);
    assert!(view.data.hosts[0].has_password);
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!(req.host.password.as_deref(), Some("pw"));
    assert_eq!(bob.host_password_in(&id, "h1", false).unwrap().as_deref(), Some("pw"));

    // Team membership survives locking and reopening the store.
    bob.lock();
    bob.unlock("bob-master").unwrap();
    assert_eq!(bob.teams().len(), 1);
    assert!(bob.teams()[0].view.as_ref().unwrap().joined);

    // A missing folder is reported, not fatal.
    let gone = temp_dir("gone").join("x");
    assert!(matches!(bob.join_team(gone.to_str().unwrap(), "detour1x"), Err(Error::Unavailable { .. } | Error::NotATeamVault | Error::Io { .. })));
}

#[test]
fn view_access_connects_with_own_login() {
    let (_alice, mut bob, id) = team_with_bob(Level::View);
    let view = bob.sync_team(&id).unwrap();
    assert!(!view.data.hosts[0].has_password);
    assert!(matches!(bob.host_password_in(&id, "h1", false), Err(Error::Forbidden)));

    // Without an own login Bob is asked for a password; the one he types becomes his own.
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!(req.host.auth, Auth::Password);
    assert!(req.host.password.is_none());
    bob.save_secret_in(&id, "h1", SecretKind::Password, "bobs-own".into()).unwrap();
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!(req.host.password.as_deref(), Some("bobs-own"));
    assert_eq!(bob.host_password_in(&id, "h1", true).unwrap().as_deref(), Some("bobs-own"));

    // Or his own key and login name.
    let key = bob.generate_key("bob-key", KeyAlgorithm::Ed25519, None, false).unwrap();
    let view = bob.team_own_login_set(&id, "h1", Some(OwnLoginView { user: "bob".into(), auth: Auth::Key { key_id: key.id.clone() }, has_password: false, new_password: None })).unwrap();
    assert_eq!(view.data.hosts[0].own.as_ref().unwrap().user, "bob");
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!(req.host.user, "bob");
    assert_eq!(req.key.unwrap().id, key.id);

    // Nothing about Bob's own login reaches the team.
    let view = bob.team_own_login_set(&id, "h1", None).unwrap();
    assert!(view.data.hosts[0].own.is_none());
}

#[test]
fn access_changes_reach_the_member() {
    let (mut alice, mut bob, id) = team_with_bob(Level::None);
    assert!(bob.sync_team(&id).unwrap().data.hosts.is_empty());
    let bob_member = bob.sync_team(&id).unwrap().members.iter().find(|m| m.is_you).unwrap().id.clone();
    let view = alice.team_set_access(&id, &[AccessChange { record: "h1".into(), member: bob_member.clone(), level: Level::Secret }]).unwrap();
    assert!(view.access.iter().any(|a| a.member == bob_member && a.level == Level::Secret));
    let view = bob.sync_team(&id).unwrap();
    assert_eq!(view.data.hosts.len(), 1);
    assert!(view.data.hosts[0].has_password);
}

#[test]
fn rollback_is_remembered_across_restarts() {
    let (mut alice, mut bob, id) = team_with_bob(Level::View);
    alice.team_put(&id, VaultUpdate { hosts: vec![host("h1"), host("h2")], keys: vec![], snippets: vec![], tunnels: vec![] }).unwrap();
    bob.sync_team(&id).unwrap();
    let path = alice.teams()[0].path.clone();
    let dir = std::path::Path::new(&path).join("ops");
    let alice_dev = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).find(|p| std::fs::read_dir(p).unwrap().count() >= 3).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&alice_dev).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    std::fs::remove_file(files.last().unwrap()).unwrap();

    bob.lock();
    bob.unlock("bob-master").unwrap();
    let view = bob.teams()[0].view.clone().unwrap();
    assert!(view.problems.iter().any(|p| matches!(p.problem, Problem::RolledBack { .. })), "{:?}", view.problems);
}

#[test]
fn recovery_key_from_creation_restores_access() {
    let shared = temp_dir("shared2").join("acme");
    let mut alice = Store::open(temp_dir("alice2"));
    alice.create("m").unwrap();
    let recovery = Store::new_recovery_key();
    let team = alice.create_team(shared.to_str().unwrap(), "Acme", "Alice", &recovery).unwrap();

    // Alice on a brand new install, with only the recovery key.
    let mut fresh = Store::open(temp_dir("alice-new"));
    fresh.create("m2").unwrap();
    let view = fresh.recover_team(shared.to_str().unwrap(), &recovery, "Alice").unwrap();
    assert_eq!(view.id, team.id);
    assert_eq!(view.my_role, Some(Role::Owner));
}
