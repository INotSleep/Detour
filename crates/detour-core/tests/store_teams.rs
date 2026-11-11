//! Two people with their own Detour stores sharing a team folder.

use std::path::PathBuf;

use detour_core::model::{Auth, Group, GroupPrefsView, HostView, KeyAlgorithm, Layout, MyGroup, OwnLoginView, ProxyKind, ProxyRef, ProxyView};
use detour_core::ssh::Route;

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
    HostView { id: id.into(), group: "g".into(), label: id.into(), address: "127.0.0.1".into(), port: 22, user: "u".into(), auth: Auth::Password, tags: vec![], last_connected: None, has_password: false, new_password: Some("pw".into()), own: None, proxy: None, has_proxy_password: false, new_proxy_password: None, proxy_id: None, forward_agent: false, order: None }
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
    alice.team_put(&team.id, VaultUpdate { hosts: vec![host("h1")], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();

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
    let view = bob.team_own_login_set(&id, "h1", Some(OwnLoginView { user: "bob".into(), auth: Auth::Key { key_id: key.id.clone() }, has_password: false, active: true, new_password: None })).unwrap();
    assert_eq!(view.data.hosts[0].own.as_ref().unwrap().user, "bob");
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!(req.host.user, "bob");
    assert_eq!(req.key.unwrap().id, key.id);

    // Nothing about Bob's own login reaches the team.
    let view = bob.team_own_login_set(&id, "h1", None).unwrap();
    assert!(view.data.hosts[0].own.is_none());
}

#[test]
fn the_shared_login_can_be_used_without_losing_the_own_one() {
    let (_alice, mut bob, id) = team_with_bob(Level::Secret);
    bob.sync_team(&id).unwrap();
    let shared = bob.connect_request_in(&id, "h1", None, None).unwrap().host;
    let key = bob.generate_key("bob-key", KeyAlgorithm::Ed25519, None, false).unwrap();
    let own = |active| OwnLoginView { user: "bob".into(), auth: Auth::Key { key_id: key.id.clone() }, has_password: false, active, new_password: None };
    bob.team_own_login_set(&id, "h1", Some(own(true))).unwrap();
    assert_eq!(bob.connect_request_in(&id, "h1", None, None).unwrap().host.user, "bob");

    // Back to the shared login: the own one stays, switched off.
    let view = bob.team_own_login_set(&id, "h1", Some(own(false))).unwrap();
    let kept = view.data.hosts[0].own.as_ref().unwrap();
    assert_eq!((kept.user.as_str(), kept.active), ("bob", false));
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!((req.host.user, req.host.auth), (shared.user, shared.auth));

    // And on again, as it was.
    bob.team_own_login_set(&id, "h1", Some(own(true))).unwrap();
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!((req.host.user.as_str(), req.key.unwrap().id), ("bob", key.id.clone()));
}

#[test]
fn a_team_proxy_needs_secret_access_to_connect_through() {
    let (mut alice, mut bob, id) = team_with_bob(Level::Secret);
    let wg = ProxyKind::Wireguard { endpoint: "vpn.example:51820".into(), public_key: "pub".into(), addresses: vec!["10.9.0.2/32".into()], dns: vec![], mtu: None, allowed_ips: vec![], keepalive: None };
    let proxy = ProxyView { id: "vpn".into(), name: "Office VPN".into(), kind: wg, saved: vec![], new_secrets: [("privateKey".to_string(), "wg-private".to_string())].into() };
    let mut h1 = alice.team_view(&id).unwrap().data.hosts[0].clone();
    h1.proxy_id = Some("vpn".into());
    alice.team_put(&id, VaultUpdate { hosts: vec![h1], keys: vec![], snippets: vec![], tunnels: vec![], proxies: Some(vec![proxy]), groups: None }).unwrap();

    // A member sees the proxy, not its key, and can't connect through it.
    let view = bob.sync_team(&id).unwrap();
    assert_eq!(view.data.proxies.len(), 1);
    assert!(view.data.proxies[0].saved.is_empty());
    assert!(matches!(bob.connect_request_in(&id, "h1", None, None), Err(Error::Proxy { reason }) if reason == "noAccess"));

    let bob_member = view.members.iter().find(|m| m.is_you).unwrap().id.clone();
    alice.team_set_access(&id, &[AccessChange { record: "vpn".into(), member: bob_member, level: Level::Secret }]).unwrap();
    bob.sync_team(&id).unwrap();
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert!(matches!(req.route, Route::Proxy(p) if p.private_key.as_deref() == Some("wg-private")));
}

#[test]
fn shared_groups_and_order_are_the_editors_to_change() {
    let (mut alice, mut bob, id) = team_with_bob(Level::Secret);
    let mut h1 = alice.team_view(&id).unwrap().data.hosts[0].clone();
    h1.group = "Production".into();
    h1.order = Some(2.0);
    let prod = Group { id: "g1".into(), name: "Production".into(), order: 1.0, proxy_id: None };
    alice.team_put(&id, VaultUpdate { hosts: vec![h1], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: Some(vec![prod.clone()]) }).unwrap();

    let view = bob.sync_team(&id).unwrap();
    assert_eq!(view.data.groups, vec![prod.clone()]);
    assert_eq!(view.data.hosts[0].order, Some(2.0));
    // A member who may only view the group can't move it.
    let moved = Group { order: 5.0, ..prod };
    assert!(matches!(bob.team_put(&id, VaultUpdate { hosts: view.data.hosts.clone(), keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: Some(vec![moved]) }), Err(Error::Forbidden)));
}

fn socks(id: &str, port: u16) -> ProxyView {
    ProxyView { id: id.into(), name: id.into(), kind: ProxyKind::Socks5 { address: "127.0.0.1".into(), port, user: String::new() }, saved: vec![], new_secrets: Default::default() }
}

fn proxy_port(r: &Route) -> Option<u16> {
    match r {
        Route::Proxy(p) => match p.kind {
            ProxyKind::Socks5 { port, .. } => Some(port),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn a_hosts_way_and_login_come_from_its_groups_by_priority() {
    let (mut alice, mut bob, id) = team_with_bob(Level::Secret);
    let mut h1 = alice.team_view(&id).unwrap().data.hosts[0].clone();
    h1.group = "Prod".into();
    let prod = Group { id: "g1".into(), name: "Prod".into(), order: 1.0, proxy_id: Some("shared".into()) };
    alice.team_put(&id, VaultUpdate { hosts: vec![h1.clone()], keys: vec![], snippets: vec![], tunnels: vec![], proxies: Some(vec![socks("shared", 1001), socks("hosts", 1002)]), groups: Some(vec![prod]) }).unwrap();
    bob.sync_team(&id).unwrap();
    let route = |s: &Store| proxy_port(&s.connect_request_in(&id, "h1", None, None).unwrap().route);

    // The group's shared proxy, for a host with none of its own.
    assert_eq!(route(&bob), Some(1001));
    // The host's own proxy comes before the group's shared one.
    h1.proxy_id = Some("hosts".into());
    alice.team_put(&id, VaultUpdate { hosts: vec![h1.clone()], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
    bob.sync_team(&id).unwrap();
    assert_eq!(route(&bob), Some(1002));

    // Bob's own settings of the group come first: his proxy and his login.
    bob.put(VaultUpdate { hosts: vec![], keys: vec![], snippets: vec![], tunnels: vec![], proxies: Some(vec![socks("bobs", 1003)]), groups: None }).unwrap();
    let key = bob.generate_key("bob-key", KeyAlgorithm::Ed25519, None, false).unwrap();
    let prefs = |key_name: &str, port_id: &str| GroupPrefsView { key: key_name.into(), proxy: Some(ProxyRef { vault: "personal".into(), id: port_id.into() }), user: Some("bob".into()), auth: Some(Auth::Key { key_id: key.id.clone() }), has_password: false, new_password: None };
    bob.group_prefs_set(prefs(&format!("{id}/team:Prod"), "bobs"), false).unwrap();
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    assert_eq!((proxy_port(&req.route), req.host.user.as_str(), req.key.map(|k| k.id)), (Some(1003), "bob", Some(key.id.clone())));

    // In his own grouping, the settings of his group come before those of the shared one.
    let keys = bob.view().unwrap().keys;
    bob.put(VaultUpdate { hosts: vec![], keys, snippets: vec![], tunnels: vec![], proxies: Some(vec![socks("bobs", 1003), socks("mine", 1004)]), groups: None }).unwrap();
    bob.layout_set(&id, Some(Layout { groups: vec![MyGroup { id: "m1".into(), name: "Work".into(), hosts: vec!["h1".into()] }] })).unwrap();
    bob.group_prefs_set(GroupPrefsView { key: format!("{id}/mine:m1"), proxy: Some(ProxyRef { vault: "personal".into(), id: "mine".into() }), user: None, auth: None, has_password: false, new_password: None }, false).unwrap();
    let req = bob.connect_request_in(&id, "h1", None, None).unwrap();
    // The proxy from his group; the login still from the shared group's settings.
    assert_eq!((proxy_port(&req.route), req.host.user.as_str()), (Some(1004), "bob"));
    assert_eq!(bob.team_view(&id).unwrap().layout.unwrap().groups[0].name, "Work");

    // Without his settings, the host's own proxy again.
    bob.group_prefs_set(GroupPrefsView { key: format!("{id}/mine:m1"), proxy: None, user: None, auth: None, has_password: false, new_password: None }, true).unwrap();
    bob.group_prefs_set(GroupPrefsView { key: format!("{id}/team:Prod"), proxy: None, user: None, auth: None, has_password: false, new_password: None }, true).unwrap();
    assert_eq!(route(&bob), Some(1002));
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
    alice.team_put(&id, VaultUpdate { hosts: vec![host("h1"), host("h2")], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
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

#[test]
fn a_team_opens_from_this_devices_cache_without_checking_everything_again() {
    let dir = temp_dir("cache");
    let mut s = Store::open(&dir);
    s.create("m").unwrap();
    let path = temp_dir("cache-team").join("t").display().to_string();
    let team = s.create_team(&path, "Acme", "Me", &Store::new_recovery_key()).unwrap();
    for i in 0..10 {
        s.team_put(&team.id, VaultUpdate { hosts: (0..=i).map(|j| host(&format!("h{j}"))).collect(), keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
    }
    s.lock();
    s.unlock("m").unwrap();
    assert_eq!(s.team_signatures_checked(&team.id), 0, "every entry was verified on this device before");
    assert_eq!(s.team_view(&team.id).unwrap().data.hosts.len(), 10);
    // The cache is sealed to this device: no key ids or hashes in the clear.
    let raw = std::fs::read(dir.join("cache").join(format!("{}.age", team.id))).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("verified"));

    // A changed file is a new file: it is checked again (and here, rejected).
    let ops = std::path::Path::new(&path).join("ops");
    let device = std::fs::read_dir(&ops).unwrap().next().unwrap().unwrap().path();
    let first = device.join("00000002.json");
    let mut bytes = std::fs::read(&first).unwrap();
    let i = bytes.len() / 2;
    bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
    std::fs::write(&first, bytes).unwrap();
    s.lock();
    s.unlock("m").unwrap();
    assert!(s.team_signatures_checked(&team.id) >= 1);
    assert!(s.team_view(&team.id).unwrap().problems.iter().any(|p| matches!(p.problem, Problem::Rejected { .. })));
}

#[test]
fn agent_forwarding_is_each_members_own_choice() {
    let (mut alice, mut bob, id) = team_with_bob(Level::Secret);
    let forwarded = HostView { forward_agent: true, new_password: None, ..host("h1") };
    let view = alice.team_put(&id, VaultUpdate { hosts: vec![forwarded], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
    assert!(view.data.hosts[0].forward_agent);
    assert!(alice.connect_request_in(&id, "h1", None, None).unwrap().host.forward_agent);

    // Bob's keys are not forwarded because Alice wants hers to be.
    let view = bob.sync_team(&id).unwrap();
    assert!(!view.data.hosts[0].forward_agent);
    assert!(!bob.connect_request_in(&id, "h1", None, None).unwrap().host.forward_agent);
}
