//! Two people with their own Detour stores sharing a team folder.

use std::path::PathBuf;

use detour_core::Error;
use detour_core::model::{Auth, HostView};
use detour_core::store::{Store, VaultUpdate};
use detour_core::team::state::Role;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("detour-st-{tag}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(id: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: Some("pw".into()) }
}

#[test]
fn create_invite_join_and_survive_relock() {
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
    alice.team_add_member(&team.id, &bob_id.detour_id, "Bob", Role::Member, false).unwrap();

    let view = bob.sync_team(&team.id).unwrap();
    assert!(view.joined);
    assert_eq!(view.data.hosts.len(), 1);
    assert!(view.data.hosts[0].has_password);

    // Bob can connect with the shared password but not export a team key.
    let req = bob.connect_request_in(&team.id, "h1", None, None).unwrap();
    assert_eq!(req.host.password.as_deref(), Some("pw"));

    // Team membership survives locking and reopening the store.
    bob.lock();
    bob.unlock("bob-master").unwrap();
    assert_eq!(bob.teams().len(), 1);
    assert!(bob.teams()[0].view.as_ref().unwrap().joined);

    // A missing folder is reported, not fatal.
    let gone = temp_dir("gone").join("x");
    assert!(matches!(bob.join_team(gone.to_str().unwrap(), &info.root), Err(Error::Unavailable { .. } | Error::NotATeamVault | Error::Io { .. })));
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
