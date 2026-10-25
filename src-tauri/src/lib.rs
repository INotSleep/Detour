//! Thin Tauri shell: every command here only translates between the webview
//! and `detour_core`. No business logic belongs in this crate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use detour_core::model::{Auth, KeyAlgorithm, KeyView, OwnLoginView, VaultView};
use detour_core::ssh::{self, SessionEvent, SessionHandle};
use detour_core::personal::Pairing;
use detour_core::store::{ImportPreview, ImportResult, MyId, PERSONAL, PendingTeam, Store, SyncStatus, TeamEntry, VaultUpdate};
use detour_core::team::state::{Coll, Level, Role};
use detour_core::{github, sync};
use detour_core::sftp::{self, FileEntry, Files};
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
    files: Files,
}

fn poisoned<T>(_: T) -> Error {
    Error::Internal { message: "state lock poisoned".into() }
}

/// After a successful edit of a vault kept in Git (a team, or the personal vault
/// synced through a repository), sends it to the repository in the background.
fn pushed<T>(app: &App, vault_id: &str, result: Result<T>) -> Result<T> {
    if result.is_ok() {
        let (store, id) = (app.store.clone(), vault_id.to_string());
        tauri::async_runtime::spawn(async move {
            let _ = if id == PERSONAL { sync::push_personal(&store).await } else { sync::push_team(&store, &id).await };
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
    app.files.close_all();
    app.store.lock().map_err(poisoned)?.lock();
    Ok(())
}

/* ---------- files (SFTP) ---------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesOpened {
    id: String,
    home: String,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

/// Opens a file browser on a host. `secret` and `trust_fingerprint` answer a login's prompts.
#[tauri::command]
async fn files_open(app: State<'_, App>, vault_id: String, host_id: String, secret: Option<String>, trust_fingerprint: Option<String>) -> Result<FilesOpened> {
    let (id, home) = sftp::open_in(&app.store, &app.files, &vault_id, &host_id, secret, trust_fingerprint).await?;
    Ok(FilesOpened { id, home })
}

/* ---------- opening files in other apps ---------- */

/// Opens a file on this computer with the app the system uses for it.
#[tauri::command]
fn open_local(path: String) -> Result<()> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| Error::Io { message: e.to_string() })
}

/// A program that can edit files, as this computer has it.
#[derive(Serialize)]
struct Editor {
    name: String,
    /// The program, or on macOS the app bundle.
    path: String,
}

/// Editors installed here, the common ones in their usual places.
#[tauri::command]
fn editors() -> Vec<Editor> {
    let mut found: Vec<(&str, std::path::PathBuf)> = Vec::new();
    #[cfg(windows)]
    {
        let env = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
        let (local, pf, pf86, win) = (env("LOCALAPPDATA"), env("ProgramFiles"), env("ProgramFiles(x86)"), env("SystemRoot"));
        let under = |base: &Option<std::path::PathBuf>, rel: &str| base.as_ref().map(|b| b.join(rel));
        let candidates = [
            ("VS Code", under(&local, r"Programs\Microsoft VS Code\Code.exe")),
            ("VS Code", under(&pf, r"Microsoft VS Code\Code.exe")),
            ("Cursor", under(&local, r"Programs\cursor\Cursor.exe")),
            ("Notepad++", under(&pf, r"Notepad++\notepad++.exe")),
            ("Notepad++", under(&pf86, r"Notepad++\notepad++.exe")),
            ("Sublime Text", under(&pf, r"Sublime Text\sublime_text.exe")),
            ("Notepad", under(&win, r"System32\notepad.exe")),
        ];
        for (name, path) in candidates {
            if let Some(p) = path.filter(|p| p.is_file()) {
                found.push((name, p));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for (name, app) in [
            ("VS Code", "/Applications/Visual Studio Code.app"),
            ("Cursor", "/Applications/Cursor.app"),
            ("Sublime Text", "/Applications/Sublime Text.app"),
            ("BBEdit", "/Applications/BBEdit.app"),
            ("TextEdit", "/System/Applications/TextEdit.app"),
        ] {
            if std::path::Path::new(app).exists() {
                found.push((name, app.into()));
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        for (name, bin) in [("VS Code", "code"), ("Cursor", "cursor"), ("Sublime Text", "subl"), ("Kate", "kate"), ("Text Editor", "gnome-text-editor"), ("gedit", "gedit"), ("Mousepad", "mousepad")] {
            if let Some(p) = dirs.iter().map(|d| d.join(bin)).find(|p| p.is_file()) {
                found.push((name, p));
            }
        }
    }
    let mut out: Vec<Editor> = Vec::new();
    for (name, path) in found {
        if !out.iter().any(|e| e.name == name) {
            out.push(Editor { name: name.into(), path: path.display().to_string() });
        }
    }
    out
}

/// Opens a file on this computer in a given program.
#[tauri::command]
fn open_with(path: String, program: String) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&program);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new(&program);
    cmd.arg(&path).spawn().map(|_| ()).map_err(|e| Error::Io { message: format!("{program}: {e}") })
}

/// A fresh place in the temp folder for the copy of a server's file being edited.
#[tauri::command]
fn edit_copy_path(name: String) -> Result<String> {
    let safe: String = name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join("detour-edit").join(format!("{stamp:x}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(safe).display().to_string())
}

/// When a file on this computer last changed (Unix ms); None once it is gone.
#[tauri::command]
fn local_modified(path: String) -> Option<i64> {
    let t = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64)
}

/// This person's home folder, where the side for this computer starts.
#[tauri::command]
fn local_home() -> String {
    sftp::local_home()
}

/// A folder on this computer; "" lists the drives.
#[tauri::command]
async fn local_list(path: String) -> Result<Vec<FileEntry>> {
    tauri::async_runtime::spawn_blocking(move || sftp::local_list(&path)).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

#[tauri::command]
fn local_mkdir(path: String) -> Result<()> {
    Ok(std::fs::create_dir(path)?)
}

#[tauri::command]
async fn files_list(app: State<'_, App>, id: String, path: String) -> Result<Vec<FileEntry>> {
    app.files.list(&id, &path).await
}

#[tauri::command]
async fn files_resolve(app: State<'_, App>, id: String, path: String) -> Result<String> {
    app.files.resolve(&id, &path).await
}

#[tauri::command]
async fn files_mkdir(app: State<'_, App>, id: String, path: String) -> Result<()> {
    app.files.mkdir(&id, &path).await
}

#[tauri::command]
async fn files_rename(app: State<'_, App>, id: String, from: String, to: String) -> Result<()> {
    app.files.rename(&id, &from, &to).await
}

#[tauri::command]
async fn files_remove(app: State<'_, App>, id: String, path: String) -> Result<()> {
    app.files.remove(&id, &path).await
}

/// Downloads a file; progress arrives on `on_progress`. `transfer` names it for `files_cancel`.
#[tauri::command]
async fn files_download(app: State<'_, App>, id: String, remote: String, local: String, transfer: String, on_progress: Channel<Progress>) -> Result<()> {
    app.files.download(&id, &remote, std::path::Path::new(&local), &transfer, move |done, total| { let _ = on_progress.send(Progress { done, total }); }).await
}

/// Uploads a file or a folder; `replace` lets it replace what is there once complete.
#[tauri::command]
async fn files_upload(app: State<'_, App>, id: String, local: String, remote: String, replace: bool, transfer: String, on_progress: Channel<Progress>) -> Result<()> {
    app.files.upload(&id, std::path::Path::new(&local), &remote, replace, &transfer, move |done, total| { let _ = on_progress.send(Progress { done, total }); }).await
}

/// Downloads a file or a folder into a local folder under a free name. Returns where it went.
#[tauri::command]
async fn files_download_into(app: State<'_, App>, id: String, remote: String, local_dir: String, transfer: String, on_progress: Channel<Progress>) -> Result<String> {
    let to = app.files.download_into(&id, &remote, std::path::Path::new(&local_dir), &transfer, move |done, total| { let _ = on_progress.send(Progress { done, total }); }).await?;
    Ok(to.display().to_string())
}

#[tauri::command]
async fn files_chmod(app: State<'_, App>, id: String, path: String, mode: u32) -> Result<()> {
    app.files.chmod(&id, &path, mode).await
}

#[tauri::command]
fn files_cancel(app: State<App>, transfer: String) {
    app.files.cancel(&transfer);
}

#[tauri::command]
fn files_close(app: State<App>, id: String) {
    app.files.close(&id);
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
    let r = app.store.lock().map_err(poisoned)?.put(update);
    pushed(&app, PERSONAL, r)
}

/* ---------- moving over from OpenSSH ---------- */

/// What ~/.ssh/config and ~/.ssh/known_hosts hold, against the personal vault.
#[tauri::command]
fn ssh_import_preview(app: State<App>) -> Result<ImportPreview> {
    app.store.lock().map_err(poisoned)?.ssh_import_preview()
}

#[tauri::command]
fn ssh_import(app: State<App>, aliases: Vec<String>, known_hosts: bool) -> Result<ImportResult> {
    let r = app.store.lock().map_err(poisoned)?.ssh_import(&aliases, known_hosts);
    pushed(&app, PERSONAL, r)
}

/* ---------- personal vault sync ---------- */

/// A sync's outcome: whether records changed here (the UI then re-reads the vault) and the status.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Synced {
    changed: bool,
    status: SyncStatus,
}

fn synced(app: &App, changed: bool) -> Result<Synced> {
    Ok(Synced { changed, status: app.store.lock().map_err(poisoned)?.sync_status()? })
}

fn sync_status_now(app: &App) -> Result<SyncStatus> {
    app.store.lock().map_err(poisoned)?.sync_status()
}

#[tauri::command]
fn sync_status(app: State<App>) -> Result<SyncStatus> {
    sync_status_now(&app)
}

/// Syncs the personal vault now; `trust_fingerprint` answers a repository server's new key.
#[tauri::command]
async fn sync_now(app: State<'_, App>, trust_fingerprint: Option<String>) -> Result<Synced> {
    let changed = sync::sync_personal(&app.store, trust_fingerprint).await?;
    // Teams in Git joined on another device get joined here too.
    let joined = sync::join_linked_teams(&app.store).await;
    synced(&app, changed || joined)
}

/// Teams joined on this person's other devices and not here.
#[tauri::command]
fn teams_pending(app: State<App>) -> Result<Vec<PendingTeam>> {
    app.store.lock().map_err(poisoned)?.pending_teams()
}

/// Joins, in a folder of this device, a team joined on another one.
#[tauri::command]
fn team_join_linked(app: State<App>, team_id: String, path: String) -> Result<TeamView> {
    let r = app.store.lock().map_err(poisoned)?.join_linked_team(&team_id, &path);
    pushed(&app, PERSONAL, r)
}

#[tauri::command]
fn sync_enable_folder(app: State<App>, path: String) -> Result<SyncStatus> {
    app.store.lock().map_err(poisoned)?.sync_enable(&path, None)?;
    sync_status_now(&app)
}

#[tauri::command]
async fn sync_enable_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, github: Option<bool>, trust_fingerprint: Option<String>) -> Result<SyncStatus> {
    sync::enable_git_sync(&app.store, git_remote(url, auth, token, github), trust_fingerprint).await?;
    sync_status_now(&app)
}

/// Joins this person's synced vault with a pairing code or the recovery key.
#[tauri::command]
async fn sync_join_folder(app: State<'_, App>, path: String, code: String) -> Result<SyncStatus> {
    sync::join_folder_sync(&app.store, path, code).await?;
    sync_status_now(&app)
}

#[tauri::command]
async fn sync_join_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, github: Option<bool>, code: String, trust_fingerprint: Option<String>) -> Result<SyncStatus> {
    sync::join_git_sync(&app.store, git_remote(url, auth, token, github), code, trust_fingerprint).await?;
    sync_status_now(&app)
}

/// A one-time code for another device of this person.
#[tauri::command]
async fn sync_add_device(app: State<'_, App>) -> Result<Pairing> {
    sync::add_device(&app.store).await
}

#[tauri::command]
fn sync_recovery_key(app: State<App>) -> Result<String> {
    app.store.lock().map_err(poisoned)?.sync_recovery_key()
}

#[tauri::command]
fn sync_disable(app: State<App>) -> Result<()> {
    app.store.lock().map_err(poisoned)?.sync_disable()
}

#[tauri::command]
async fn sync_replace_server_key(app: State<'_, App>, fingerprint: String) -> Result<Synced> {
    let changed = sync::replace_sync_server_key(&app.store, fingerprint).await?;
    synced(&app, changed)
}

#[tauri::command]
async fn sync_set_token(app: State<'_, App>, token: String) -> Result<Synced> {
    app.store.lock().map_err(poisoned)?.sync_set_token(&token)?;
    let changed = sync::sync_personal(&app.store, None).await?;
    synced(&app, changed)
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

fn git_remote(url: String, auth: Auth, token: Option<String>, github: Option<bool>) -> GitRemote {
    let github = github.unwrap_or(false);
    GitRemote { url, auth, token: token.filter(|_| !github), github }
}

/* ---------- signing in with GitHub ---------- */

/// Who this device is signed in to GitHub as, if anyone.
#[tauri::command]
fn github_user(app: State<App>) -> Result<Option<String>> {
    Ok(app.store.lock().map_err(poisoned)?.github_login()?.map(|l| l.user))
}

/// Starts signing in: the code to enter on GitHub.
#[tauri::command]
async fn github_start() -> Result<github::DeviceCode> {
    github::start(&github::Endpoints::default()).await
}

/// Whether the code was entered yet; keeps the sign-in once it was.
#[tauri::command]
async fn github_poll(app: State<'_, App>, device_code: String) -> Result<github::Poll> {
    github::poll(&app.store, &github::Endpoints::default(), &device_code).await
}

/// Whether the sign-in reaches a github.com repository (the app is installed on it).
#[tauri::command]
async fn github_repo_access(app: State<'_, App>, url: String) -> Result<bool> {
    github::repo_access(&app.store, &github::Endpoints::default(), &url).await
}

#[tauri::command]
fn github_sign_out(app: State<App>) -> Result<()> {
    app.store.lock().map_err(poisoned)?.set_github_login(None)
}

/// A team's repository is reached with the GitHub sign-in from now on; then syncs.
#[tauri::command]
async fn team_use_github(app: State<'_, App>, id: String) -> Result<TeamView> {
    app.store.lock().map_err(poisoned)?.team_use_github(&id)?;
    sync::sync_team(&app.store, &id, None).await
}

/// The sync repository is reached with the GitHub sign-in from now on; then syncs.
#[tauri::command]
async fn sync_use_github(app: State<'_, App>) -> Result<Synced> {
    app.store.lock().map_err(poisoned)?.sync_use_github()?;
    let changed = sync::sync_personal(&app.store, None).await?;
    synced(&app, changed)
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
async fn team_create_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, github: Option<bool>, name: String, my_name: String, recovery_key: String, trust_fingerprint: Option<String>) -> Result<TeamView> {
    sync::create_git_team(&app.store, git_remote(url, auth, token, github), &name, &my_name, &recovery_key, trust_fingerprint).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitInspected {
    info: VaultInfo,
    /// Hands the clone back to `team_join_git`.
    clone: String,
}

#[tauri::command]
async fn team_inspect_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, github: Option<bool>, trust_fingerprint: Option<String>) -> Result<GitInspected> {
    let (info, clone) = sync::inspect_git_team(&app.store, &git_remote(url, auth, token, github), trust_fingerprint).await?;
    Ok(GitInspected { info, clone })
}

#[tauri::command]
fn team_join_git(app: State<App>, clone: String, root: String, url: String, auth: Auth, token: Option<String>, github: Option<bool>) -> Result<TeamView> {
    sync::join_git_team(&app.store, &clone, &root, git_remote(url, auth, token, github))
}

#[tauri::command]
async fn team_recover_git(app: State<'_, App>, url: String, auth: Auth, token: Option<String>, github: Option<bool>, recovery_key: String, my_name: String, trust_fingerprint: Option<String>) -> Result<TeamView> {
    sync::recover_git_team(&app.store, git_remote(url, auth, token, github), &recovery_key, &my_name, trust_fingerprint).await
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
    // Own logins live in the personal vault.
    let r = app.store.lock().map_err(poisoned)?.team_own_login_set(&id, &host_id, login);
    pushed(&app, PERSONAL, r)
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

    let pinned_fingerprint = connected.pinned_fingerprint.clone();
    app.store.lock().map_err(poisoned)?.record_connection_in(&args.vault_id, &args.host_id, connected.new_keys)?;

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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(App { store: Arc::new(Mutex::new(Store::open(data_dir))), sessions: Mutex::new(HashMap::new()), tunnels: Tunnels::new(), files: Files::new() })
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
            ssh_import_preview,
            ssh_import,
            sync_status,
            teams_pending,
            team_join_linked,
            sync_now,
            sync_enable_folder,
            sync_enable_git,
            sync_join_folder,
            sync_join_git,
            sync_add_device,
            sync_recovery_key,
            sync_disable,
            sync_replace_server_key,
            sync_set_token,
            sync_use_github,
            github_user,
            github_start,
            github_poll,
            github_repo_access,
            github_sign_out,
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
            files_open,
            files_list,
            local_home,
            local_list,
            local_mkdir,
            open_local,
            editors,
            open_with,
            edit_copy_path,
            local_modified,
            files_resolve,
            files_mkdir,
            files_rename,
            files_remove,
            files_download,
            files_upload,
            files_download_into,
            files_chmod,
            files_cancel,
            files_close,
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
            team_use_github,
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
    // The updater reads its key and address from tauri.conf.json, which test builds don't have.
    builder(tauri::Builder::default().plugin(tauri_plugin_updater::Builder::new().build()), data_dir())
        .run(tauri::generate_context!())
        .expect("error while running Detour");
}


