//! Looking back at a team vault and going back: the logs are never rewritten, a rollback is a new entry.

use std::collections::BTreeMap;
use std::path::PathBuf;

use detour_core::identity::Identity;
use detour_core::model::{Auth, HostView};
use detour_core::store::VaultUpdate;
use detour_core::team::history::{ChangeKind, PlanAction, Rollback};
use detour_core::team::state::{Coll, Level, Role};
use detour_core::team::{Change, TeamVault};
use serde_json::json;

fn temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-history-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(id: &str, label: &str, address: &str) -> HostView {
    HostView { id: id.into(), group: String::new(), label: label.into(), address: address.into(), port: 22, user: "root".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None, proxy_id: None, forward_agent: false, order: None }
}

fn hosts(hs: Vec<HostView>) -> VaultUpdate {
    VaultUpdate { hosts: hs, keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }
}

fn team() -> (PathBuf, TeamVault) {
    let path = temp_dir().join("vault");
    let v = TeamVault::create(&path, "Acme", &Identity::generate(), "Alice", "alice-pc", &Identity::generate()).unwrap();
    (path, v)
}

fn join(path: &PathBuf, admin: &mut TeamVault, name: &str, role: Role) -> TeamVault {
    let who = Identity::generate();
    admin.add_member(&who.public().detour_id(), name, role, Level::Edit).unwrap();
    let info = TeamVault::inspect(path).unwrap();
    TeamVault::open(path, &info.root, &who, &format!("{name}-pc"), BTreeMap::new()).unwrap()
}

/// Host id -> (label, address), as the vault shows them now.
fn shown(v: &mut TeamVault) -> BTreeMap<String, (String, String)> {
    v.reload().unwrap();
    v.data().hosts.into_iter().map(|h| (h.id, (h.label, h.address))).collect()
}

fn pair(label: &str, address: &str) -> (String, String) {
    (label.to_string(), address.to_string())
}

fn last_entry(v: &TeamVault) -> String {
    v.log().unwrap().last().unwrap().entry.clone()
}

fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

#[test]
fn undoing_a_stale_write_brings_back_what_it_overwrote() {
    let (path, mut alice) = team();
    alice.put(hosts(vec![host("h1", "out-1", "10.0.0.1")])).unwrap();
    let mut bob = join(&path, &mut alice, "Bob", Role::Owner);
    bob.reload().unwrap();

    tick();
    alice.put(hosts(vec![host("h1", "out-1", "10.0.0.2"), host("h2", "new-box", "10.0.0.9")])).unwrap();

    // Bob's window still showed the old picture and saved it: old address back, the new host gone.
    bob.reload().unwrap();
    tick();
    bob.apply(vec![Change::Put { coll: Coll::Host, id: "h1".into(), fields: BTreeMap::from([("address".into(), json!("10.0.0.1"))]) }, Change::Delete { id: "h2".into() }]).unwrap();
    let stale = last_entry(&bob);
    assert_eq!(shown(&mut alice), BTreeMap::from([("h1".into(), pair("out-1", "10.0.0.1"))]));

    let plan = alice.rollback_plan(&Rollback::Undo { entry: stale.clone() }).unwrap();
    let actions: BTreeMap<String, PlanAction> = plan.items.iter().map(|i| (i.record.clone(), i.action)).collect();
    assert_eq!(actions, BTreeMap::from([("h1".into(), PlanAction::Change), ("h2".into(), PlanAction::Restore)]));
    let address = plan.items.iter().find(|i| i.record == "h1").unwrap().fields.iter().find(|f| f.field == "address").unwrap();
    assert_eq!((address.from.clone(), address.to.clone()), (Some(json!("10.0.0.1")), Some(json!("10.0.0.2"))));

    let done = alice.rollback(&Rollback::Undo { entry: stale }).unwrap();
    assert_eq!((done.applied, done.skipped), (2, 0));
    let back = BTreeMap::from([("h1".into(), pair("out-1", "10.0.0.2")), ("h2".into(), pair("new-box", "10.0.0.9"))]);
    assert_eq!(shown(&mut alice), back);
    // Everyone reads the rollback as an ordinary entry.
    assert_eq!(shown(&mut bob), back);
}

#[test]
fn going_back_to_a_time_changes_removes_and_restores() {
    let (_p, mut a) = team();
    a.put(hosts(vec![host("h1", "web", "10.0.0.1"), host("h2", "db", "10.0.0.2")])).unwrap();
    let then = a.log().unwrap().last().unwrap().at;
    tick();
    a.put(hosts(vec![host("h1", "web-renamed", "10.0.0.1"), host("h3", "cache", "10.0.0.3")])).unwrap();

    let done = a.rollback(&Rollback::At { at: then }).unwrap();
    assert_eq!(done.applied, 3);
    assert_eq!(shown(&mut a), BTreeMap::from([("h1".into(), pair("web", "10.0.0.1")), ("h2".into(), pair("db", "10.0.0.2"))]));

    // The rollback is an entry like any other: undoing it gives back the state before it.
    a.rollback(&Rollback::Undo { entry: done.entry.unwrap() }).unwrap();
    assert_eq!(shown(&mut a), BTreeMap::from([("h1".into(), pair("web-renamed", "10.0.0.1")), ("h3".into(), pair("cache", "10.0.0.3"))]));
}

#[test]
fn going_back_on_one_device_keeps_everyone_elses_changes() {
    let (path, mut alice) = team();
    alice.put(hosts(vec![host("h1", "web", "10.0.0.1"), host("h2", "db", "10.0.0.2")])).unwrap();
    let mut bob = join(&path, &mut alice, "Bob", Role::Owner);
    bob.reload().unwrap();
    let since = alice.log().unwrap().last().unwrap().at;

    tick();
    let mut now = bob.data().hosts;
    now[0].address = "6.6.6.6".into();
    now.retain(|h| h.id != "h2");
    bob.put(hosts(now)).unwrap();
    alice.reload().unwrap();
    tick();
    let mut mine = alice.data().hosts;
    mine[0].label = "web-1".into();
    alice.put(hosts(mine)).unwrap();

    alice.rollback(&Rollback::Without { device: Some("Bob-pc".into()), author: None, after: since }).unwrap();
    assert_eq!(shown(&mut alice), BTreeMap::from([("h1".into(), pair("web-1", "10.0.0.1")), ("h2".into(), pair("db", "10.0.0.2"))]));
}

#[test]
fn a_record_goes_back_to_a_version_and_a_deleted_one_comes_back_with_its_password() {
    let (_p, mut a) = team();
    let mut h = host("h1", "web", "10.0.0.1");
    h.new_password = Some("pw".into());
    a.put(hosts(vec![h, host("h2", "db", "10.0.0.2")])).unwrap();
    tick();
    a.put(hosts(vec![host("h1", "web-2", "10.0.0.5"), host("h2", "db", "10.0.0.2")])).unwrap();

    let versions = a.history("h1").unwrap();
    assert_eq!(versions.iter().map(|e| e.changes[0].kind).collect::<Vec<_>>(), vec![ChangeKind::Created, ChangeKind::Changed]);
    assert_eq!(versions[1].changes[0].fields.get("label"), Some(&json!("web-2")));
    assert!(versions[0].changes[0].secrets.contains(&"password".to_string()), "the password is named, never shown");

    a.rollback(&Rollback::Record { record: "h1".into(), entry: versions[0].entry.clone() }).unwrap();
    assert_eq!(shown(&mut a).get("h1"), Some(&pair("web", "10.0.0.1")));

    a.put(hosts(vec![host("h2", "db", "10.0.0.2")])).unwrap();
    assert_eq!(a.deleted().into_iter().map(|d| (d.record, d.name, d.is_you)).collect::<Vec<_>>(), vec![("h1".to_string(), "web".to_string(), true)]);
    a.rollback(&Rollback::Restore { record: "h1".into() }).unwrap();
    assert!(a.deleted().is_empty());
    assert_eq!(a.host("h1").unwrap().password.as_deref(), Some("pw"));
}

#[test]
fn records_without_edit_access_stay_as_they_are() {
    let (path, mut alice) = team();
    alice.put(hosts(vec![host("h1", "web", "10.0.0.1")])).unwrap();
    let then = alice.log().unwrap().last().unwrap().at;
    tick();
    alice.put(hosts(vec![host("h1", "web-2", "10.0.0.1")])).unwrap();
    // A plain member sees records but may not change them.
    let mut carol = join(&path, &mut alice, "Carol", Role::Member);
    carol.reload().unwrap();
    alice.set_access(&[detour_core::team::AccessChange { record: "h1".into(), member: carol_id(&carol), level: Level::View }]).unwrap();
    carol.reload().unwrap();

    let plan = carol.rollback_plan(&Rollback::At { at: then }).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(!plan.items[0].allowed);
    let done = carol.rollback(&Rollback::At { at: then }).unwrap();
    assert_eq!((done.applied, done.skipped, done.entry), (0, 1, None));
    assert_eq!(shown(&mut alice).get("h1"), Some(&pair("web-2", "10.0.0.1")));
}

fn carol_id(v: &TeamVault) -> String {
    v.view().members.iter().find(|m| m.is_you).unwrap().id.clone()
}

#[test]
fn the_log_names_who_changed_what() {
    let (_p, mut a) = team();
    a.put(hosts(vec![host("h1", "web", "10.0.0.1")])).unwrap();
    a.put(hosts(vec![])).unwrap();
    let log = a.log().unwrap();
    let kinds: Vec<ChangeKind> = log.iter().flat_map(|e| e.changes.iter().map(|c| c.kind)).collect();
    assert_eq!(kinds.first(), Some(&ChangeKind::TeamCreated));
    assert!(kinds.contains(&ChangeKind::Created) && kinds.contains(&ChangeKind::Deleted));
    let last = log.last().unwrap();
    assert!(last.is_you && last.this_device && last.author_name == "Alice");
    assert_eq!(last.changes[0].name, "web");
}
