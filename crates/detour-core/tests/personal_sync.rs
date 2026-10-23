//! One person's devices sharing their personal vault through a folder.

use std::path::PathBuf;
use std::sync::Mutex;

use detour_core::Error;
use detour_core::model::{Auth, HostView, KeyAlgorithm, KnownHost, Snippet};
use detour_core::store::{Store, VaultUpdate};
use detour_core::sync;

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-psync-{tag}-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn device(tag: &str) -> Mutex<Store> {
    let mut s = Store::open(temp_dir(tag));
    s.create("m").unwrap();
    Mutex::new(s)
}

fn host(id: &str, label: &str, auth: Auth) -> HostView {
    HostView { id: id.into(), group: String::new(), label: label.into(), address: "10.0.0.7".into(), port: 22, user: "opsuser".into(), auth, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None }
}

/// The UI's edit: the current view with `f` applied.
fn edit(s: &Mutex<Store>, f: impl FnOnce(&mut VaultUpdate)) {
    let mut st = s.lock().unwrap();
    let v = st.view().unwrap();
    let mut u = VaultUpdate { hosts: v.hosts, keys: v.keys, snippets: v.snippets, tunnels: v.tunnels };
    f(&mut u);
    st.put(u).unwrap();
}

fn labels(s: &Mutex<Store>) -> Vec<String> {
    let mut l: Vec<String> = s.lock().unwrap().view().unwrap().hosts.iter().map(|h| h.label.clone()).collect();
    l.sort();
    l
}

/// Laptop with a host (with a saved password) and a key, syncing through a fresh folder.
fn laptop_in_sync() -> (Mutex<Store>, String) {
    let laptop = device("laptop");
    edit(&laptop, |u| {
        let mut h = host("h1", "db", Auth::Password);
        h.new_password = Some("pw-db-1".into());
        u.hosts.push(h);
    });
    laptop.lock().unwrap().generate_key("deploy", KeyAlgorithm::Ed25519, None, false).unwrap();
    let folder = temp_dir("folder").join("vault");
    laptop.lock().unwrap().sync_enable(&folder.display().to_string(), None).unwrap();
    (laptop, folder.display().to_string())
}

#[tokio::test]
async fn a_new_device_joins_with_a_code_and_both_merge() {
    let (laptop, folder) = laptop_in_sync();
    let my_id = laptop.lock().unwrap().my_id().unwrap().detour_id;
    let code = sync::add_device(&laptop).await.unwrap();
    assert_eq!(code.code.len(), 19, "{}", code.code);

    // The desktop already had a host of its own and a pinned server key.
    let desktop = device("desktop");
    edit(&desktop, |u| u.hosts.push(host("h2", "web", Auth::Agent)));
    desktop.lock().unwrap().record_connection("h2", vec![KnownHost { endpoint: "10.0.0.7:22".into(), key: "ssh-ed25519 AAAA".into(), fingerprint: "SHA256:x".into(), added_at: 1 }]).unwrap();
    assert_ne!(desktop.lock().unwrap().my_id().unwrap().detour_id, my_id);

    sync::join_folder_sync(&desktop, folder.clone(), code.code.to_lowercase().replace('-', " ")).await.unwrap();
    // The desktop is now the same person: same Detour ID, both hosts, the password and the key.
    assert_eq!(desktop.lock().unwrap().my_id().unwrap().detour_id, my_id);
    assert_eq!(labels(&desktop), ["db", "web"]);
    let d = desktop.lock().unwrap();
    assert_eq!(d.host_password_in("personal", "h1", false).unwrap().as_deref(), Some("pw-db-1"));
    let key = d.view().unwrap().keys[0].clone();
    assert!(d.export_key(&key.id, None).unwrap().contains("OPENSSH PRIVATE KEY"));
    drop(d);

    // The laptop picks up the desktop's host and pinned key.
    assert!(laptop.lock().unwrap().sync_personal().unwrap());
    assert_eq!(labels(&laptop), ["db", "web"]);
    let req = laptop.lock().unwrap().connect_request("h2", None, None).unwrap();
    assert_eq!(req.known.map(|k| k.fingerprint).as_deref(), Some("SHA256:x"));
    assert!(laptop.lock().unwrap().sync_status().unwrap().problems.is_empty());
}

#[tokio::test]
async fn edits_and_deletes_travel_both_ways() {
    let (laptop, folder) = laptop_in_sync();
    let code = sync::add_device(&laptop).await.unwrap();
    let desktop = device("desktop");
    sync::join_folder_sync(&desktop, folder, code.code).await.unwrap();

    // Different fields of one host changed on both, before either synced: both survive.
    edit(&laptop, |u| u.hosts[0].label = "db-main".into());
    edit(&desktop, |u| u.hosts[0].port = 2222);
    laptop.lock().unwrap().sync_personal().unwrap();
    desktop.lock().unwrap().sync_personal().unwrap();
    laptop.lock().unwrap().sync_personal().unwrap();
    for d in [&laptop, &desktop] {
        let h = d.lock().unwrap().view().unwrap().hosts[0].clone();
        assert_eq!((h.label.as_str(), h.port), ("db-main", 2222));
    }

    // A new snippet and a deleted key.
    edit(&desktop, |u| {
        u.snippets.push(Snippet { id: "s1".into(), name: "disk".into(), command: "df -h".into() });
        u.keys.clear();
    });
    laptop.lock().unwrap().sync_personal().unwrap();
    let v = laptop.lock().unwrap().view().unwrap();
    assert_eq!(v.snippets.len(), 1);
    assert!(v.keys.is_empty());

    // A password set later on one device reaches the other.
    laptop.lock().unwrap().set_host_password("h1", Some("pw-db-2".into())).unwrap();
    desktop.lock().unwrap().sync_personal().unwrap();
    assert_eq!(desktop.lock().unwrap().host_password_in("personal", "h1", false).unwrap().as_deref(), Some("pw-db-2"));
}

#[tokio::test]
async fn the_folder_holds_no_secrets_or_names() {
    let (laptop, folder) = laptop_in_sync();
    sync::add_device(&laptop).await.unwrap();
    let mut all = Vec::new();
    for e in walk(PathBuf::from(&folder)) {
        all.extend(std::fs::read(e).unwrap());
    }
    let text = String::from_utf8_lossy(&all);
    for secret in ["pw-db-1", "10.0.0.7", "\"db\"", "deploy", "OPENSSH", "opsuser"] {
        assert!(!text.contains(secret), "{secret} is in the folder");
    }
}

fn walk(dir: PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() { out.extend(walk(e.path())) } else { out.push(e.path()) }
    }
    out
}

#[tokio::test]
async fn wrong_codes_and_wrong_folders_are_refused() {
    let (laptop, folder) = laptop_in_sync();
    let code = sync::add_device(&laptop).await.unwrap();
    let desktop = device("desktop");

    let mut wrong = code.code.clone().into_bytes();
    let last = wrong.len() - 1;
    wrong[last] = if wrong[last] == b'A' { b'B' } else { b'A' };
    let r = sync::join_folder_sync(&desktop, folder.clone(), String::from_utf8(wrong).unwrap()).await;
    assert!(matches!(r, Err(Error::InvalidCode)), "{r:?}");
    assert!(matches!(sync::join_folder_sync(&desktop, folder.clone(), "ABCD".into()).await, Err(Error::InvalidCode)));
    assert!(!desktop.lock().unwrap().sync_status().unwrap().enabled);

    // A team vault is not a personal one, and the other way round.
    let team = temp_dir("team").join("t");
    let t = team.display().to_string();
    let recovery = Store::new_recovery_key();
    laptop.lock().unwrap().create_team(&t, "Acme", "Me", &recovery).unwrap();
    assert!(matches!(sync::join_folder_sync(&desktop, t, code.code.clone()).await, Err(Error::NotASyncVault)));
    assert!(matches!(Store::inspect_team(&folder), Err(Error::NotATeamVault)));
}

#[tokio::test]
async fn the_recovery_key_joins_without_another_device() {
    let (laptop, folder) = laptop_in_sync();
    let recovery = laptop.lock().unwrap().sync_recovery_key().unwrap();
    drop(laptop); // lost

    let fresh = device("fresh");
    sync::join_folder_sync(&fresh, folder, recovery).await.unwrap();
    assert_eq!(labels(&fresh), ["db"]);
}

#[tokio::test]
async fn a_device_in_teams_keeps_its_own_identity() {
    let (laptop, folder) = laptop_in_sync();
    let code = sync::add_device(&laptop).await.unwrap();
    let desktop = device("desktop");
    let t = temp_dir("team").join("t").display().to_string();
    desktop.lock().unwrap().create_team(&t, "Acme", "Me", &Store::new_recovery_key()).unwrap();
    assert!(matches!(sync::join_folder_sync(&desktop, folder, code.code).await, Err(Error::SyncHasTeams)));
}

#[tokio::test]
async fn sync_survives_locking_and_can_be_turned_off() {
    let (laptop, folder) = laptop_in_sync();
    let code = sync::add_device(&laptop).await.unwrap();
    let desktop = device("desktop");
    sync::join_folder_sync(&desktop, folder, code.code).await.unwrap();

    desktop.lock().unwrap().lock();
    desktop.lock().unwrap().unlock("m").unwrap();
    edit(&laptop, |u| u.hosts.push(host("h3", "cache", Auth::Agent)));
    assert!(desktop.lock().unwrap().sync_personal().unwrap());
    assert_eq!(labels(&desktop), ["cache", "db"]);

    desktop.lock().unwrap().sync_disable().unwrap();
    assert!(!desktop.lock().unwrap().sync_status().unwrap().enabled);
    edit(&laptop, |u| u.hosts.clear());
    // Off: nothing comes in, nothing is lost here.
    assert!(desktop.lock().unwrap().sync_personal().is_err());
    assert_eq!(labels(&desktop), ["cache", "db"]);
}

