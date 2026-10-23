//! Thin Tauri shell: every command here only translates between the webview
//! and `detour_core`. No business logic belongs in this crate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use detour_core::model::{Auth, KeyAlgorithm, KeyView, OwnLoginView, VaultView};
use detour_core::ssh::{self, SessionEvent, SessionHandle};
use detour_core::store::{MyId, Store, TeamEntry, VaultUpdate};
use detour_core::team::state::{Coll, Level, Role};
use detour_core::sync;
use detour_core::tunnel::{self, TunnelStatus, Tunnels};
use detour_core::team::git::GitRemote;
use detour_core::team::{AccessChange, TeamView, VaultInfo};
use detour_core::{Error, Result, SecretKind};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

struct App {
    store: Arc<Mutex<Store>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    tunnels: Tunnels,
}

fn poisoned<T>(_: T) -> Error {
    Error::Internal { message: "state lock poisoned".into() }
}

/// After a successful edit of a team kept in Git, sends it to the repository in the background.
fn pushed<T>(app: &App, vault_id: &str, result: Result<T>) -> Result<T> {
    if result.is_ok() && vault_id != detour_core::store::PERSONAL {
        let (store, id) = (app.store.clone(), vault_id.to_string());
        tauri::async_runtime::spawn(async move {
            let _ = sync::push_team(&store, &id).await;
        });
    }
    result
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInfo {
    core_version: &'static str,
    os: &'static str,
}

#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo { core_version: detour_core::VERSION, os: std::env::consts::OS }
}

/// Writes a file the user picked in the native save dialog (key export).
#[tauri::command]
fn write_text_file(path: String, contents: String) -> std::result::Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/* ---------- vault ---------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VaultStatus {
    initialized: bool,
    unlocked: bool,
}

#[tauri::command]
fn vault_status(app: State<App>) -> Result<VaultStatus> {
    let s = app.store.lock().map_err(poisoned)?;
    Ok(VaultStatus { initialized: s.is_initialized(), unlocked: s.is_unlocked() })
}

#[tauri::command]
async fn vault_create(app: State<'_, App>, master_password: String) -> Result<VaultView> {
    let mut s = app.store.lock().map_err(poisoned)?;
    s.create(&master_password)?;
    s.view()
}

#[tauri::command]
async fn vault_unlock(app: State<'_, App>, master_password: String) -> Result<VaultView> {
    let mut s = app.store.lock().map_err(poisoned)?;
    s.unlock(&master_password)?;
    s.view()
}

#[tauri::command]
fn vault_lock(app: State<App>) -> Result<()> {
    app.tunnels.stop_all();
    app.store.lock().map_err(poisoned)?.lock();
    Ok(())
}

/* ---------- tunnels ---------- */

/// Opens a tunnel. `secret` and `trust_fingerprint` answer the prompts a login may need.
#[tauri::command]
async fn tunnel_start(app: State<'_, App>, vault_id: String, tunnel_id: String, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<()> {
    tunnel::start_in(&app.store, &app.tunnels, &vault_id, &tunnel_id, secret, trust_fingerprint).await
}

#[tauri::command]
fn tunnel_stop(app: State<App>, vault_id: String, tunnel_id: String) {
    app.tunnels.stop(&format!("{vault_id}/{tunnel_id}"));
}

/// Every running or failed tunnel, keyed `<vault>/<tunnel>`.
#[tauri::command]
fn tunnels_status(app: State<App>) -> Vec<TunnelStatus> {
    app.tunnels.statuses()
}

#[tauri::command]
async fn vault_change_master_password(app: State<'_, App>, current: String, new: String) -> Result<()> {
    app.store.lock().map_err(poisoned)?.change_master_password(&current, &new)
}

#[tauri::command]
fn vault_get(app: State<App>) -> Result<VaultView> {
    app.store.lock().map_err(poisoned)?.view()
}

#[tauri::command]
fn vault_put(app: State<App>, update: VaultUpdate) -> Result<VaultView> {
    app.store.lock().map_err(poisoned)?.put(update)
}

#[tauri::command]
fn host_set_password(app: State<App>, vault_id: String, host_id: String, password: Option<String>) -> Result<()> {
    let r = app.store.lock().map_err(poisoned)?.set_host_password_in(&vault_id, &host_id, password);
    pushed(&app, &vault_id, r)
}

/// A saved password to show or copy: the shared one, or with `own` the person's own login for a team host.
#[tauri::command]
fn host_password(app: State<App>, vault_id: String, host_id: String, own: bool) -> Result<Option<String>> {
    app.store.lock().map_err(poisoned)?.host_password_in(&vault_id, &host_id, own)
}

#[tauri::command]
fn host_forget_key(app: State<App>, vault_id: String, host_id: String) -> Result<()> {
    app.store.lock().map_err(poisoned)?.forget_host_key_in(&vault_id, &host_id)
}

/* ---------- keys ---------- */

#[tauri::command]
async fn key_generate(app: State<'_, App>, vault_id: String, name: String, algorithm: KeyAlgorithm, passphrase: Option<String>, remember: bool) -> Result<KeyView> {
    let r = app.store.lock().map_err(poisoned)?.generate_key_in(&vault_id, &name, algorithm, passphrase.as_deref(), remember);
    pushed(&app, &vault_id, r)
}

#[tauri::command]
async fn key_import(app: State<'_, App>, vault_id: String, name: String, text: String, passphrase: Option<String>, remember: bool) -> Result<KeyView> {
    let r = app.store.lock().map_err(poisoned)?.import_key_in(&vault_id, &name, &text, passphrase.as_deref(), remember);
    pushed(&app, &vault_id, r)
}

#[tauri::command]
fn key_export(app: State<App>, vault_id: String, key_id: String, passphrase: Option<String>) -> Result<String> {
    app.store.lock().map_err(poisoned)?.export_key_in(&vault_id, &key_id, passphrase.as_deref())
}

#[tauri::command]
async fn key_set_passphrase(app: State<'_, App>, vault_id: String, key_id: String, current: Option<String>, new: Option<String>, remember: bool) -> Result<KeyView> {
    let r = app.store.lock().map_err(poisoned)?.set_key_passphrase_in(&vault_id, &key_id, current.as_deref(), new.as_deref(), remember);
    pushed(&app, &vault_id, r)
}

/* ---------- teams ---------- */

#[tauri::command]
fn my_id(app: State<App>) -> Result<MyId> {
    app.store.lock().map_err(poisoned)?.my_id()
}

/// Fingerprint of a pasted Detour ID (validates it too).
#[tauri::command]
fn id_inspect(detour_id: String) -> Result<String> {
    Store::inspect_id(&detour_id)
}

#[tauri::command]
fn recovery_key_new() -> String {
    Store::new_recovery_key()
}

#[tauri::command]
fn teams_list(app: State<App>) -> Result<Vec<TeamEntry>> {
    Ok(app.store.lock().map_err(poisoned)?.teams())
}

#[tauri::command]
async fn team_create(app: State<'_, App>, path: String, name: String, my_name: String, recovery_key: String) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.create_team(&path, &name, &my_name, &recovery_key)
}

#[tauri::command]
fn team_inspect(path: String) -> Result<VaultInfo> {
    Store::inspect_team(&path)
}

#[tauri::command]
async fn team_join(app: State<'_, App>, path: String, root: String) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.join_team(&path, &root)
}

#[tauri::command]
async fn team_recover(app: State<'_, App>, path: String, recovery_key: String, my_name: String) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.recover_team(&path, &recovery_key, &my_name)
}

#[tauri::command]
fn team_leave(app: State<App>, id: String) -> Result<()> {
    app.store.lock().map_err(poisoned)?.leave_team(&id)
}

#[tauri::command]
/// Re-reads a team; a team in Git is pulled and pushed. `trust_fingerprint` answers a new server key.
async fn team_sync(app: State<'_, App>, id: String, trust_fingerprint: Option<String>) -> Result<TeamView> {
    sync::sync_team(&app.store, &id, trust_fingerprint).await
}

fn git_remote(url: String, auth: Auth, token: Option<String>) -> GitRemote {
    GitRemote { url, auth, token }
}

/// Replaces the HTTPS token of a team's repository, then syncs.
#[tauri::command]
async fn team_set_git_token(app: State<'_, App>, id: String, token: String) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.set_git_token(&id, &token)?;
    sync::sync_team(&app.store, &id, None).await
}

/// Trusts a repository server's new key in place of the pinned one.
#[tauri::command]
async fn team_replace_server_key(app: State<'_, App>, id: String, fingerprint: String) -> Result<TeamView> {
    sync::replace_server_key(&app.store, &id, fingerprint).await
}

#[tauri::command]
async fn team_create_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, name: String, my_name: String, recovery_key: String, trust_fingerprint: Option<String>) -> Result<TeamView> {
    sync::create_git_team(&app.store, git_remote(url, auth, token), &name, &my_name, &recovery_key, trust_fingerprint).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitInspected {
    info: VaultInfo,
    /// Hands the clone back to `team_join_git`.
    clone: String,
}

#[tauri::command]
async fn team_inspect_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, trust_fingerprint: Option<String>) -> Result<GitInspected> {
    let (info, clone) = sync::inspect_git_team(&app.store, &git_remote(url, auth, token), trust_fingerprint).await?;
    Ok(GitInspected { info, clone })
}

#[tauri::command]
fn team_join_git(app: State<App>, clone: String, root: String, url: String, auth: Auth, token: Option<String>) -> Result<TeamView> {
    sync::join_git_team(&app.store, &clone, &root, git_remote(url, auth, token))
}

#[tauri::command]
async fn team_recover_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, recovery_key: String, my_name: String, trust_fingerprint: Option<String>) -> Result<TeamView> {
    sync::recover_git_team(&app.store, git_remote(url, auth, token), &recovery_key, &my_name, trust_fingerprint).await
}

#[tauri::command]
async fn team_put(app: State<'_, App>, id: String, update: VaultUpdate) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_put(&id, update);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_rename(app: State<'_, App>, id: String, name: String) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_rename(&id, &name);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_member_add(app: State<'_, App>, id: String, detour_id: String, name: String, role: Role, access: Level) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_add_member(&id, &detour_id, &name, role, access);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_member_update(app: State<'_, App>, id: String, member: String, role: Role) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_update_member(&id, &member, role);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_access_set(app: State<'_, App>, id: String, changes: Vec<AccessChange>) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_set_access(&id, &changes);
    pushed(&app, &id, r)
}

#[tauri::command]
fn team_own_login_set(app: State<App>, id: String, host_id: String, login: Option<OwnLoginView>) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.team_own_login_set(&id, &host_id, login)
}

#[tauri::command]
async fn team_member_remove(app: State<'_, App>, id: String, member: String) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_remove_member(&id, &member);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_resolve_field(app: State<'_, App>, id: String, coll: Coll, record: String, field: String, pick: usize) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_resolve_field(&id, coll, &record, &field, pick);
    pushed(&app, &id, r)
}

#[tauri::command]
async fn team_resolve_deleted(app: State<'_, App>, id: String, coll: Coll, record: String, keep: bool) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.team_resolve_deleted(&id, coll, &record, keep);
    pushed(&app, &id, r)
}

/* ---------- sessions ---------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionOpened {
    session_id: String,
    /// Fingerprint pinned on this connection (first time this host was trusted).
    pinned_fingerprint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionClosed {
    closed: bool,
    exit_status: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenArgs {
    vault_id: String,
    host_id: String,
    secret: Option<String>,
    trust_fingerprint: Option<String>,
    cols: u32,
    rows: u32,
}

/// Opens a shell. Terminal output arrives on `on_event` as raw bytes; the
/// final message is a JSON `{ closed: true, exitStatus }`.
#[tauri::command]
async fn session_open(app: State<'_, App>, args: OpenArgs, on_event: Channel<InvokeResponseBody>) -> Result<SessionOpened> {
    let request = app.store.lock().map_err(poisoned)?.connect_request_in(&args.vault_id, &args.host_id, args.secret, args.trust_fingerprint)?;
    let connected = ssh::connect(request, args.cols, args.rows).await?;

    let pinned_fingerprint = connected.new_known_host.as_ref().map(|k| k.fingerprint.clone());
    app.store.lock().map_err(poisoned)?.record_connection_in(&args.vault_id, &args.host_id, connected.new_known_host)?;

    let session_id = uuid_like();
    app.sessions.lock().map_err(poisoned)?.insert(session_id.clone(), connected.handle);

    let mut events = connected.events;
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = events.recv().await {
            let sent = match ev {
                SessionEvent::Data(bytes) => on_event.send(InvokeResponseBody::Raw(bytes)),
                SessionEvent::Closed { exit_status } => {
                    let json = serde_json::to_string(&SessionClosed { closed: true, exit_status }).unwrap_or_default();
                    let _ = on_event.send(InvokeResponseBody::Json(json));
                    break;
                }
            };
            if sent.is_err() {
                break;
            }
        }
    });

    Ok(SessionOpened { session_id, pinned_fingerprint })
}

fn session(app: &App, id: &str) -> Result<SessionHandle> {
    app.sessions.lock().map_err(poisoned)?.get(id).cloned().ok_or_else(|| Error::NotFound { what: "session".into() })
}

#[tauri::command]
fn session_input(app: State<App>, session_id: String, data: String) -> Result<()> {
    session(&app, &session_id)?.input(data.into_bytes());
    Ok(())
}

#[tauri::command]
fn session_resize(app: State<App>, session_id: String, cols: u32, rows: u32) -> Result<()> {
    session(&app, &session_id)?.resize(cols, rows);
    Ok(())
}

#[tauri::command]
fn session_close(app: State<App>, session_id: String) -> Result<()> {
    if let Some(h) = app.sessions.lock().map_err(poisoned)?.remove(&session_id) {
        h.close();
    }
    Ok(())
}

/// Saves a password or passphrase the user typed during a login.
#[tauri::command]
fn secret_save(app: State<App>, vault_id: String, host_id: String, kind: SecretKindArg, value: String) -> Result<()> {
    let kind = match kind {
        SecretKindArg::Password => SecretKind::Password,
        SecretKindArg::Passphrase => SecretKind::Passphrase,
    };
    let r = app.store.lock().map_err(poisoned)?.save_secret_in(&vault_id, &host_id, kind, value);
    pushed(&app, &vault_id, r)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum SecretKindArg {
    Password,
    Passphrase,
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("s{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// `DETOUR_DATA_DIR` overrides where the vault lives (tests, portable installs).
fn data_dir() -> std::path::PathBuf {
    std::env::var_os("DETOUR_DATA_DIR").map(Into::into).unwrap_or_else(Store::default_dir)
}

/// The app's commands and state, shared by the real window and the IPC tests.
#[doc(hidden)]
pub fn builder<R: tauri::Runtime>(builder: tauri::Builder<R>, data_dir: std::path::PathBuf) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(App { store: Arc::new(Mutex::new(Store::open(data_dir))), sessions: Mutex::new(HashMap::new()), tunnels: Tunnels::new() })
        .invoke_handler(tauri::generate_handler![
            core_info,
            write_text_file,
            vault_status,
            vault_create,
            vault_unlock,
            vault_lock,
            vault_change_master_password,
            vault_get,
            vault_put,
            host_set_password,
            host_forget_key,
            host_password,
            key_generate,
            key_import,
            key_export,
            key_set_passphrase,
            session_open,
            session_input,
            session_resize,
            session_close,
            secret_save,
            tunnel_start,
            tunnel_stop,
            tunnels_status,
            my_id,
            id_inspect,
            recovery_key_new,
            teams_list,
            team_create,
            team_inspect,
            team_join,
            team_recover,
            team_leave,
            team_sync,
            team_create_git,
            team_replace_server_key,
            team_set_git_token,
            team_inspect_git,
            team_join_git,
            team_recover_git,
            team_put,
            team_rename,
            team_member_add,
            team_member_update,
            team_member_remove,
            team_access_set,
            team_own_login_set,
            team_resolve_field,
            team_resolve_deleted,
        ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    builder(tauri::Builder::default(), data_dir())
        .run(tauri::generate_context!())
        .expect("error while running Detour");
}


