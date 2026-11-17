//! Teams and the personal vault kept in a cloud drive, against an in-memory drive.

use std::path::PathBuf;
use std::sync::Mutex;

use detour_core::Error;
use detour_core::drive::fake::FakeDrive;
use detour_core::drive::{DriveLogin, DriveRemote, FAKES, Provider};
use detour_core::model::{Auth, HostView};
use detour_core::store::{Store, VaultUpdate};
use detour_core::sync::{self, Place};
use detour_core::team::state::{Level, Role};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("detour-drive-{tag}-{}", detour_core::new_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(id: &str, label: &str) -> HostView {
    HostView { id: id.into(), group: "g".into(), label: label.into(), address: "10.0.0.1".into(), port: 22, user: "root".into(), auth: Auth::Agent, tags: vec![], last_connected: None, has_password: false, new_password: None, own: None, proxy: None, has_proxy_password: false, new_proxy_password: None, proxy_id: None, forward_agent: false, order: None }
}

/// A store signed in to OneDrive (the token is never used: the drive is in memory).
fn store(tag: &str) -> Mutex<Store> {
    let mut s = Store::open(temp_dir(tag));
    s.create("m").unwrap();
    s.set_drive_login(DriveLogin { provider: Provider::OneDrive, account: format!("{tag}@example.com"), access_token: "tok".into(), expires_at: i64::MAX / 2, refresh_token: None }).unwrap();
    Mutex::new(s)
}

/// A folder in a fresh in-memory drive.
fn folder() -> (FakeDrive, DriveRemote) {
    let drive = FakeDrive::new();
    let id = format!("folder-{}", detour_core::new_id());
    FAKES.lock().unwrap().insert(id.clone(), drive.clone());
    (drive, DriveRemote { provider: Provider::OneDrive, id, drive: Some("d1".into()), label: "Detour/Team".into(), link: None })
}

fn put(s: &Mutex<Store>, id: &str, hosts: Vec<HostView>) {
    s.lock().unwrap().team_put(id, VaultUpdate { hosts, keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
}

#[tokio::test]
async fn two_stores_share_a_team_through_a_drive() {
    let (drive, remote) = folder();
    let alice = store("alice");
    let team = sync::create_remote_team(&alice, Place::Drive(remote.clone()), "Acme", "Alice", &Store::new_recovery_key(), None).await.unwrap();
    let r = team.remote.clone().unwrap();
    assert_eq!((r.kind.as_str(), r.url.as_str(), r.pending), ("onedrive", "OneDrive: Detour/Team", false));
    assert!(drive.has("vault.json"));
    // Not into a folder that has something in it.
    assert!(matches!(sync::create_remote_team(&alice, Place::Drive(remote.clone()), "Other", "Alice", &Store::new_recovery_key(), None).await, Err(Error::FolderNotEmpty)));

    put(&alice, &team.id, vec![host("h1", "in-the-drive")]);
    assert!(alice.lock().unwrap().team_view(&team.id).unwrap().remote.unwrap().pending);
    sync::push_team(&alice, &team.id).await.unwrap();

    let bob = store("bob");
    let (info, token) = sync::inspect_remote_team(&bob, &Place::Drive(remote.clone()), None).await.unwrap();
    assert_eq!(info.root_fingerprint, alice.lock().unwrap().my_id().unwrap().fingerprint);
    sync::join_remote_team(&bob, &token, &info.root, Place::Drive(remote.clone())).unwrap();
    let bob_id = bob.lock().unwrap().my_id().unwrap().detour_id;
    alice.lock().unwrap().team_add_member(&team.id, &bob_id, "Bob", Role::Member, Level::Edit).unwrap();
    sync::sync_team(&alice, &team.id, None).await.unwrap();
    let view = sync::sync_team(&bob, &team.id, None).await.unwrap();
    assert!(view.joined);
    assert_eq!(view.data.hosts[0].label, "in-the-drive");

    let mut hosts = view.data.hosts.clone();
    hosts[0].label = "renamed-by-bob".into();
    put(&bob, &team.id, hosts);
    sync::push_team(&bob, &team.id).await.unwrap();
    let view = sync::sync_team(&alice, &team.id, None).await.unwrap();
    assert_eq!(view.data.hosts[0].label, "renamed-by-bob");
    let r = view.remote.unwrap();
    assert!(r.error.is_none() && !r.pending && r.synced_at.is_some(), "{r:?}");
}

#[tokio::test]
async fn signed_out_of_the_drive_the_team_says_so() {
    let (_drive, remote) = folder();
    let alice = store("alice");
    let team = sync::create_remote_team(&alice, Place::Drive(remote), "Acme", "Alice", &Store::new_recovery_key(), None).await.unwrap();
    alice.lock().unwrap().drive_sign_out(Provider::OneDrive).unwrap();
    let view = sync::sync_team(&alice, &team.id, None).await.unwrap();
    assert_eq!(view.remote.unwrap().error.unwrap()["kind"], "driveSignIn");
}

#[tokio::test]
async fn a_personal_vault_syncs_through_a_drive_and_its_teams_follow() {
    let (_d1, sync_folder) = folder();
    let (_d2, team_folder) = folder();
    let laptop = store("laptop");
    laptop.lock().unwrap().put(VaultUpdate { hosts: vec![host("h1", "mine")], keys: vec![], snippets: vec![], tunnels: vec![], proxies: None, groups: None }).unwrap();
    sync::enable_remote_sync(&laptop, Place::Drive(sync_folder.clone()), None).await.unwrap();
    let st = laptop.lock().unwrap().sync_status().unwrap();
    assert!(st.enabled && !st.git && st.storage.as_deref() == Some("onedrive") && !st.pending && st.error.is_none(), "{st:?}");
    let team = sync::create_remote_team(&laptop, Place::Drive(team_folder), "Acme", "Me", &Store::new_recovery_key(), None).await.unwrap();
    sync::sync_personal(&laptop, None).await.unwrap();

    let code = sync::add_device(&laptop).await.unwrap();
    let phone = store("phone");
    sync::join_remote_sync(&phone, Place::Drive(sync_folder), code.code, None).await.unwrap();
    assert!(phone.lock().unwrap().view().unwrap().hosts.iter().any(|h| h.label == "mine"));

    // The team's drive folder came with its link: signed in to the drive, the phone joins it by itself.
    assert!(phone.lock().unwrap().pending_teams().unwrap()[0].drive.is_some());
    assert!(sync::join_linked_teams(&phone).await);
    let teams = phone.lock().unwrap().teams();
    assert_eq!((teams.len(), teams[0].id.as_str()), (1, team.id.as_str()));
}

#[tokio::test]
async fn a_folder_team_from_another_device_opens_through_the_drive() {
    // The laptop keeps the team in a folder its drive's own app syncs; the phone reaches the
    // same folder through the drive's API.
    let (drive, remote) = folder();
    let laptop = store("laptop");
    let local = temp_dir("onedrive-folder").join("Team");
    let team = laptop.lock().unwrap().create_team(&local.display().to_string(), "Acme", "Me", &Store::new_recovery_key()).unwrap();
    let sync_dir = temp_dir("sync").join("vault").display().to_string();
    laptop.lock().unwrap().sync_enable(&sync_dir, None).unwrap();
    let code = sync::add_device(&laptop).await.unwrap();
    // The drive app's upload of the folder.
    copy_into(&drive, &local, "");

    let phone = store("phone");
    sync::join_folder_sync(&phone, sync_dir, code.code).await.unwrap();
    let pending = phone.lock().unwrap().pending_teams().unwrap();
    assert!(pending[0].url.is_none() && pending[0].drive.is_none());
    let view = sync::join_linked_drive_team(&phone, &team.id, remote).await.unwrap();
    assert_eq!(view.id, team.id);
    assert_eq!(view.remote.unwrap().kind, "onedrive");
}

fn copy_into(drive: &FakeDrive, dir: &std::path::Path, prefix: &str) {
    use detour_core::drive::Storage;
    let f = drive.folder();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if e.file_type().unwrap().is_dir() {
            copy_into(drive, &e.path(), &rel);
        } else {
            let parent = prefix.split('/').filter(|s| !s.is_empty()).fold(None::<String>, |p, part| Some(f.folder(p.as_deref(), part).unwrap()));
            f.create(parent.as_deref(), &name, &std::fs::read(e.path()).unwrap()).unwrap();
        }
    }
}
