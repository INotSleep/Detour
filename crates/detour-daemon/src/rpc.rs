//! The core's methods. Names and arguments are those the app's commands had
//! (`src/lib/api.ts`), so the desktop app forwards its calls unchanged; a stream
//! an app command got as a Tauri channel is a `stream` id here.

use std::sync::Arc;

use detour_core::model::{Auth, KeyAlgorithm, OwnLoginView, now_ms};
use detour_core::personal::Pairing;
use detour_core::sftp;
use detour_core::ssh;
use detour_core::store::{PERSONAL, Store, VaultUpdate};
use detour_core::team::AccessChange;
use detour_core::team::git::GitRemote;
use detour_core::team::state::{Coll, Level, Role};
use detour_core::{Error, SecretKind, github, sync, tunnel};
use detour_ipc::{Frame, Message};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::sessions::{Owner, SessionInfo};
use crate::{Core, Ctx};

pub(crate) type Out = Result<Value, Value>;

pub(crate) fn err(e: Error) -> Value {
    serde_json::to_value(&e).unwrap_or_else(|_| json!({ "kind": "internal", "message": e.to_string() }))
}

fn ok<T: Serialize>(r: detour_core::Result<T>) -> Out {
    r.map(|v| serde_json::to_value(v).unwrap_or(Value::Null)).map_err(err)
}

pub(crate) fn poisoned<T>(_: T) -> Value {
    err(Error::Internal { message: "state lock poisoned".into() })
}

/// The arguments of a method, as a struct its handler declares.
pub(crate) fn args<T: DeserializeOwned>(method: &str, p: Value) -> Result<T, Value> {
    let p = if p.is_null() { json!({}) } else { p };
    serde_json::from_value(p).map_err(|e| json!({ "kind": "badArguments", "message": format!("{method}: {e}") }))
}

macro_rules! args {
    ($method:expr, $p:expr, { $($name:ident : $ty:ty),* $(,)? }) => {{
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct A { $($name: $ty),* }
        args::<A>($method, $p)?
    }};
}

fn git_remote(url: String, auth: Auth, token: Option<String>, github: Option<bool>) -> GitRemote {
    let github = github.unwrap_or(false);
    GitRemote { url, auth, token: token.filter(|_| !github), github }
}

/// Methods that only read: everything else may change what clients show.
pub(crate) const READ_ONLY: &[&str] = &[
    "hello", "core_info", "vault_status", "vault_get", "ssh_import_preview", "sync_status", "teams_pending", "sync_recovery_key", "sync_add_device",
    "host_password", "key_export", "my_id", "id_inspect", "recovery_key_new", "teams_list", "team_inspect", "team_inspect_git",
    "github_user", "github_start", "github_repo_access", "session_input", "session_resize", "session_attach", "session_detach", "sessions_list", "session_runs",
    "files_open", "files_list", "files_resolve", "files_cancel", "files_close", "tunnels_status", "tunnel_start", "tunnel_stop", "settings_get",
    "settings_set", "activity", "quit", "trust_request", "trust_answer", "trust_with_password", "hosts_export",
    "agents_pending", "agents_list", "agents_answer", "agents_revoke",
];

/// What a client may call before it is trusted: enough to become trusted.
const OPEN: &[&str] = &["hello", "core_info", "vault_status", "vault_create", "vault_unlock", "trust_request", "trust_with_password"];

/// How long a terminal waits for "Allow" in the window.
const ASK_FOR: std::time::Duration = std::time::Duration::from_secs(120);

pub(crate) async fn call(core: &Arc<Core>, ctx: &Ctx, method: &str, p: Value) -> Out {
    // Agents prove themselves with a token in each call (see `agents`).
    if method.starts_with("agent_") {
        core.touch();
        return crate::agents::call(core, ctx, method, p).await;
    }
    // A core that holds nothing (no unlocked vault, no sessions, no tunnels) anyone may stop.
    let idle = || !core.store.lock().is_ok_and(|s| s.is_unlocked()) && core.sessions.is_empty() && core.tunnels.statuses().is_empty();
    if method == "quit" && idle() {
        core.quit.notify_one();
        return Ok(Value::Null);
    }
    if !OPEN.contains(&method) && !core.is_trusted(ctx.id) {
        return Err(json!({ "kind": "notTrusted", "window": core.has_window() }));
    }
    if method != "tunnels_status" {
        core.touch();
    }
    let result = dispatch(core, ctx, method, p).await;
    if result.is_ok() && !READ_ONLY.contains(&method) {
        core.emit("data.changed", json!({ "by": ctx.id, "method": method }));
    }
    result
}

/// After an edit of a vault kept in Git, sends it to the repository in the background.
fn pushed<T>(core: &Arc<Core>, vault_id: &str, result: detour_core::Result<T>) -> detour_core::Result<T> {
    if result.is_ok() {
        let (store, id) = (core.store.clone(), vault_id.to_string());
        tokio::spawn(async move {
            let _ = if id == PERSONAL { sync::push_personal(&store).await } else { sync::push_team(&store, &id).await };
        });
    }
    result
}

fn synced(core: &Core, changed: bool) -> Out {
    let status = core.store.lock().map_err(poisoned)?.sync_status().map_err(err)?;
    Ok(json!({ "changed": changed, "status": status }))
}

/// Progress of a transfer, sent to the caller's stream.
fn progress(ctx: &Ctx, stream: Option<u64>) -> impl Fn(u64, u64) + Send + Sync + 'static {
    let out = ctx.out.clone();
    move |done, total| {
        if let Some(stream) = stream {
            let _ = out.send(Frame::Msg(Message::Stream { stream, data: json!({ "done": done, "total": total }) }));
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenArgs {
    vault_id: String,
    host_id: String,
    secret: Option<String>,
    trust_fingerprint: Option<String>,
    cols: u32,
    rows: u32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum SecretKindArg {
    Password,
    Passphrase,
}

pub(crate) async fn dispatch(core: &Arc<Core>, ctx: &Ctx, method: &str, p: Value) -> Out {
    let store = &core.store;
    macro_rules! s {
        () => {
            store.lock().map_err(poisoned)?
        };
    }
    match method {
        /* ---------- the core itself ---------- */
        "hello" => Ok(json!({ "connId": ctx.id, "coreVersion": detour_core::VERSION, "protocol": detour_ipc::PROTOCOL, "pid": std::process::id() })),
        "core_info" => Ok(json!({ "coreVersion": detour_core::VERSION, "os": std::env::consts::OS })),
        "activity" => Ok(Value::Null),
        "settings_get" => Ok(serde_json::to_value(core.settings.lock().unwrap().clone()).unwrap_or_default()),
        "settings_set" => {
            let a = args!(method, p, { auto_lock_minutes: Option<u32> });
            if let Some(m) = a.auto_lock_minutes {
                core.settings.lock().unwrap().auto_lock_minutes = m;
            }
            core.save_settings().map_err(|e| err(e.into()))?;
            Ok(Value::Null)
        }
        "quit" => {
            core.quit.notify_one();
            Ok(Value::Null)
        }

        /* ---------- agents ---------- */
        "agents_pending" | "agents_list" | "agents_answer" | "agents_revoke" => crate::agents::person(core, method, p).await,

        /* ---------- vault ---------- */
        "vault_status" => {
            let s = s!();
            Ok(json!({ "initialized": s.is_initialized(), "unlocked": s.is_unlocked() }))
        }
        "vault_create" => {
            let a = args!(method, p, { master_password: String });
            let mut s = s!();
            s.create(&a.master_password).map_err(err)?;
            let view = ok(s.view());
            drop(s);
            core.trust_conn(ctx.id);
            core.emit("unlocked", Value::Null);
            view
        }
        // Whoever knows the master password is the person: their terminal is trusted too.
        "vault_unlock" => {
            let a = args!(method, p, { master_password: String });
            let mut s = s!();
            if !s.is_unlocked() {
                s.unlock(&a.master_password).map_err(err)?;
                drop(s);
                core.trust_conn(ctx.id);
                core.emit("unlocked", Value::Null);
                return ok(store.lock().map_err(poisoned)?.view());
            }
            if !core.is_trusted(ctx.id) {
                s.check_master_password(&a.master_password).map_err(err)?;
                core.trust_conn(ctx.id);
            }
            ok(s.view())
        }

        /* ---------- trusting a terminal ---------- */
        // Asks the window to allow this terminal; without a window the master password is needed.
        "trust_request" => {
            if core.is_trusted(ctx.id) {
                return Ok(Value::Null);
            }
            if !core.has_window() {
                return Err(json!({ "kind": "needMasterPassword" }));
            }
            let terminal = core.terminal_of(ctx.id);
            let (tx, rx) = tokio::sync::oneshot::channel();
            core.asking.lock().unwrap().insert(ctx.id, tx);
            core.emit("trust.requested", json!({ "id": ctx.id, "terminal": terminal }));
            let answer = tokio::time::timeout(ASK_FOR, rx).await;
            core.asking.lock().unwrap().remove(&ctx.id);
            core.emit("trust.answered", json!({ "id": ctx.id }));
            match answer {
                Ok(Ok(true)) => {
                    core.trust_conn(ctx.id);
                    Ok(Value::Null)
                }
                Ok(_) => Err(json!({ "kind": "denied" })),
                Err(_) => Err(json!({ "kind": "timedOut" })),
            }
        }
        "trust_answer" => {
            let a = args!(method, p, { id: u64, allow: bool });
            if let Some(tx) = core.asking.lock().unwrap().remove(&a.id) {
                let _ = tx.send(a.allow);
            }
            Ok(Value::Null)
        }
        "trust_with_password" => {
            let a = args!(method, p, { master_password: String });
            let mut s = s!();
            if s.is_unlocked() {
                s.check_master_password(&a.master_password).map_err(err)?;
            } else {
                s.unlock(&a.master_password).map_err(err)?;
                drop(s);
                core.emit("unlocked", Value::Null);
            }
            core.trust_conn(ctx.id);
            Ok(Value::Null)
        }
        "vault_lock" => {
            core.lock();
            Ok(Value::Null)
        }
        "vault_change_master_password" => {
            let a = args!(method, p, { current: String, new: String });
            ok(s!().change_master_password(&a.current, &a.new))
        }
        "vault_get" => ok(s!().view()),
        "vault_put" => {
            // `base` is what the client last saw: only its own changes are applied.
            let a = args!(method, p, { update: VaultUpdate, base: Option<VaultUpdate> });
            let r = match a.base {
                Some(base) => s!().put_from(&base, a.update),
                None => s!().put(a.update),
            };
            ok(pushed(core, PERSONAL, r))
        }
        "ssh_import_preview" => ok(s!().ssh_import_preview()),
        "ssh_import" => {
            let a = args!(method, p, { aliases: Vec<String>, known_hosts: bool });
            let r = s!().ssh_import(&a.aliases, a.known_hosts);
            ok(pushed(core, PERSONAL, r))
        }

        /* ---------- personal vault sync ---------- */
        "sync_status" => ok(s!().sync_status()),
        "sync_now" => {
            let a = args!(method, p, { trust_fingerprint: Option<String> });
            let changed = sync::sync_personal(store, a.trust_fingerprint).await.map_err(err)?;
            let joined = sync::join_linked_teams(store).await;
            synced(core, changed || joined)
        }
        "teams_pending" => ok(s!().pending_teams()),
        "team_join_linked" => {
            let a = args!(method, p, { team_id: String, path: String });
            let r = s!().join_linked_team(&a.team_id, &a.path);
            ok(pushed(core, PERSONAL, r))
        }
        "sync_enable_folder" => {
            let a = args!(method, p, { path: String });
            s!().sync_enable(&a.path, None).map_err(err)?;
            ok(s!().sync_status())
        }
        "sync_enable_git" => {
            let a = args!(method, p, { url: String, auth: Auth, token: Option<String>, github: Option<bool>, trust_fingerprint: Option<String> });
            sync::enable_git_sync(store, git_remote(a.url, a.auth, a.token, a.github), a.trust_fingerprint).await.map_err(err)?;
            ok(s!().sync_status())
        }
        "sync_join_folder" => {
            let a = args!(method, p, { path: String, code: String });
            sync::join_folder_sync(store, a.path, a.code).await.map_err(err)?;
            ok(s!().sync_status())
        }
        "sync_join_git" => {
            let a = args!(method, p, { url: String, auth: Auth, token: Option<String>, github: Option<bool>, code: String, trust_fingerprint: Option<String> });
            sync::join_git_sync(store, git_remote(a.url, a.auth, a.token, a.github), a.code, a.trust_fingerprint).await.map_err(err)?;
            ok(s!().sync_status())
        }
        "sync_add_device" => ok::<Pairing>(sync::add_device(store).await),
        "sync_recovery_key" => ok(s!().sync_recovery_key()),
        "sync_disable" => ok(s!().sync_disable()),
        "sync_replace_server_key" => {
            let a = args!(method, p, { fingerprint: String });
            let changed = sync::replace_sync_server_key(store, a.fingerprint).await.map_err(err)?;
            synced(core, changed)
        }
        "sync_set_token" => {
            let a = args!(method, p, { token: String });
            s!().sync_set_token(&a.token).map_err(err)?;
            let changed = sync::sync_personal(store, None).await.map_err(err)?;
            synced(core, changed)
        }
        "sync_use_github" => {
            s!().sync_use_github().map_err(err)?;
            let changed = sync::sync_personal(store, None).await.map_err(err)?;
            synced(core, changed)
        }

        /* ---------- signing in with GitHub ---------- */
        "github_user" => ok(s!().github_login().map(|l| l.map(|l| l.user))),
        "github_start" => ok(github::start(&github::Endpoints::default()).await),
        "github_poll" => {
            let a = args!(method, p, { device_code: String });
            ok(github::poll(store, &github::Endpoints::default(), &a.device_code).await)
        }
        "github_repo_access" => {
            let a = args!(method, p, { url: String });
            ok(github::repo_access(store, &github::Endpoints::default(), &a.url).await)
        }
        "github_sign_out" => ok(s!().set_github_login(None)),

        /* ---------- hosts and keys ---------- */
        "host_set_password" => {
            let a = args!(method, p, { vault_id: String, host_id: String, password: Option<String> });
            let r = s!().set_host_password_in(&a.vault_id, &a.host_id, a.password);
            ok(pushed(core, &a.vault_id, r))
        }
        "host_password" => {
            let a = args!(method, p, { vault_id: String, host_id: String, own: bool });
            ok(s!().host_password_in(&a.vault_id, &a.host_id, a.own))
        }
        "host_forget_key" => {
            let a = args!(method, p, { vault_id: String, host_id: String });
            ok(s!().forget_host_key_in(&a.vault_id, &a.host_id))
        }
        "key_generate" => {
            let a = args!(method, p, { vault_id: String, name: String, algorithm: KeyAlgorithm, passphrase: Option<String>, remember: bool });
            let r = s!().generate_key_in(&a.vault_id, &a.name, a.algorithm, a.passphrase.as_deref(), a.remember);
            ok(pushed(core, &a.vault_id, r))
        }
        "key_import" => {
            let a = args!(method, p, { vault_id: String, name: String, text: String, passphrase: Option<String>, remember: bool });
            let r = s!().import_key_in(&a.vault_id, &a.name, &a.text, a.passphrase.as_deref(), a.remember);
            ok(pushed(core, &a.vault_id, r))
        }
        "key_export" => {
            let a = args!(method, p, { vault_id: String, key_id: String, passphrase: Option<String> });
            ok(s!().export_key_in(&a.vault_id, &a.key_id, a.passphrase.as_deref()))
        }
        "key_set_passphrase" => {
            let a = args!(method, p, { vault_id: String, key_id: String, current: Option<String>, new: Option<String>, remember: bool });
            let r = s!().set_key_passphrase_in(&a.vault_id, &a.key_id, a.current.as_deref(), a.new.as_deref(), a.remember);
            ok(pushed(core, &a.vault_id, r))
        }
        "secret_save" => {
            let a = args!(method, p, { vault_id: String, host_id: String, kind: SecretKindArg, value: String });
            let kind = match a.kind {
                SecretKindArg::Password => SecretKind::Password,
                SecretKindArg::Passphrase => SecretKind::Passphrase,
            };
            let r = s!().save_secret_in(&a.vault_id, &a.host_id, kind, a.value);
            ok(pushed(core, &a.vault_id, r))
        }

        /* ---------- teams ---------- */
        "my_id" => ok(s!().my_id()),
        "id_inspect" => {
            let a = args!(method, p, { detour_id: String });
            ok(Store::inspect_id(&a.detour_id))
        }
        "recovery_key_new" => Ok(json!(Store::new_recovery_key())),
        "teams_list" => Ok(serde_json::to_value(s!().teams()).unwrap_or_default()),
        "team_create" => {
            let a = args!(method, p, { path: String, name: String, my_name: String, recovery_key: String });
            ok(s!().create_team(&a.path, &a.name, &a.my_name, &a.recovery_key))
        }
        "team_inspect" => {
            let a = args!(method, p, { path: String });
            ok(Store::inspect_team(&a.path))
        }
        "team_join" => {
            let a = args!(method, p, { path: String, root: String });
            ok(s!().join_team(&a.path, &a.root))
        }
        "team_recover" => {
            let a = args!(method, p, { path: String, recovery_key: String, my_name: String });
            ok(s!().recover_team(&a.path, &a.recovery_key, &a.my_name))
        }
        "team_leave" => {
            let a = args!(method, p, { id: String });
            ok(s!().leave_team(&a.id))
        }
        "team_sync" => {
            let a = args!(method, p, { id: String, trust_fingerprint: Option<String> });
            ok(sync::sync_team(store, &a.id, a.trust_fingerprint).await)
        }
        "team_create_git" => {
            let a = args!(method, p, { url: String, auth: Auth, token: Option<String>, github: Option<bool>, name: String, my_name: String, recovery_key: String, trust_fingerprint: Option<String> });
            ok(sync::create_git_team(store, git_remote(a.url, a.auth, a.token, a.github), &a.name, &a.my_name, &a.recovery_key, a.trust_fingerprint).await)
        }
        "team_replace_server_key" => {
            let a = args!(method, p, { id: String, fingerprint: String });
            ok(sync::replace_server_key(store, &a.id, a.fingerprint).await)
        }
        "team_set_git_token" => {
            let a = args!(method, p, { id: String, token: String });
            s!().set_git_token(&a.id, &a.token).map_err(err)?;
            ok(sync::sync_team(store, &a.id, None).await)
        }
        "team_use_github" => {
            let a = args!(method, p, { id: String });
            s!().team_use_github(&a.id).map_err(err)?;
            ok(sync::sync_team(store, &a.id, None).await)
        }
        "team_inspect_git" => {
            let a = args!(method, p, { url: String, auth: Auth, token: Option<String>, github: Option<bool>, trust_fingerprint: Option<String> });
            let (info, clone) = sync::inspect_git_team(store, &git_remote(a.url, a.auth, a.token, a.github), a.trust_fingerprint).await.map_err(err)?;
            Ok(json!({ "info": info, "clone": clone }))
        }
        "team_join_git" => {
            let a = args!(method, p, { clone: String, root: String, url: String, auth: Auth, token: Option<String>, github: Option<bool> });
            ok(sync::join_git_team(store, &a.clone, &a.root, git_remote(a.url, a.auth, a.token, a.github)))
        }
        "team_recover_git" => {
            let a = args!(method, p, { url: String, auth: Auth, token: Option<String>, github: Option<bool>, recovery_key: String, my_name: String, trust_fingerprint: Option<String> });
            ok(sync::recover_git_team(store, git_remote(a.url, a.auth, a.token, a.github), &a.recovery_key, &a.my_name, a.trust_fingerprint).await)
        }
        "team_put" => {
            let a = args!(method, p, { id: String, update: VaultUpdate, base: Option<VaultUpdate> });
            let r = match a.base {
                Some(base) => s!().team_put_from(&a.id, &base, a.update),
                None => s!().team_put(&a.id, a.update),
            };
            ok(pushed(core, &a.id, r))
        }
        "team_rename" => {
            let a = args!(method, p, { id: String, name: String });
            let r = s!().team_rename(&a.id, &a.name);
            ok(pushed(core, &a.id, r))
        }
        "team_member_add" => {
            let a = args!(method, p, { id: String, detour_id: String, name: String, role: Role, access: Level });
            let r = s!().team_add_member(&a.id, &a.detour_id, &a.name, a.role, a.access);
            ok(pushed(core, &a.id, r))
        }
        "team_member_update" => {
            let a = args!(method, p, { id: String, member: String, role: Role });
            let r = s!().team_update_member(&a.id, &a.member, a.role);
            ok(pushed(core, &a.id, r))
        }
        "team_member_remove" => {
            let a = args!(method, p, { id: String, member: String });
            let r = s!().team_remove_member(&a.id, &a.member);
            ok(pushed(core, &a.id, r))
        }
        "team_access_set" => {
            let a = args!(method, p, { id: String, changes: Vec<AccessChange> });
            let r = s!().team_set_access(&a.id, &a.changes);
            ok(pushed(core, &a.id, r))
        }
        "team_own_login_set" => {
            // Own logins live in the personal vault.
            let a = args!(method, p, { id: String, host_id: String, login: Option<OwnLoginView> });
            let r = s!().team_own_login_set(&a.id, &a.host_id, a.login);
            ok(pushed(core, PERSONAL, r))
        }
        "team_resolve_field" => {
            let a = args!(method, p, { id: String, coll: Coll, record: String, field: String, pick: usize });
            let r = s!().team_resolve_field(&a.id, a.coll, &a.record, &a.field, a.pick);
            ok(pushed(core, &a.id, r))
        }
        "team_resolve_deleted" => {
            let a = args!(method, p, { id: String, coll: Coll, record: String, keep: bool });
            let r = s!().team_resolve_deleted(&a.id, a.coll, &a.record, a.keep);
            ok(pushed(core, &a.id, r))
        }

        /* ---------- terminal sessions ---------- */
        "session_open" => {
            let a = args!(method, p, { args: OpenArgs, stream: Option<u64>, close_with_client: Option<bool> });
            let o = a.args;
            let request = s!().connect_request_in(&o.vault_id, &o.host_id, o.secret, o.trust_fingerprint).map_err(err)?;
            let connected = ssh::connect(request, o.cols, o.rows).await.map_err(err)?;
            let pinned_fingerprint = connected.pinned_fingerprint.clone();
            s!().record_connection_in(&o.vault_id, &o.host_id, connected.new_keys).map_err(err)?;

            let id = format!("s{:x}{:x}", now_ms(), ctx.id);
            let transient = a.close_with_client.unwrap_or(false);
            let info = SessionInfo { id: id.clone(), vault_id: o.vault_id, host_id: o.host_id, owner: Owner::User, started_at: now_ms(), transient };
            let c = core.clone();
            let session = core.sessions.add(info, (o.cols, o.rows), connected.handle, connected.events, transient.then_some(ctx.id), move || c.emit("sessions.changed", json!({ "by": 0 })));
            if let Some(stream) = a.stream {
                core.sessions.attach(&session, ctx.id, stream, ctx.out.clone());
            }
            core.emit("sessions.changed", json!({ "by": ctx.id }));
            Ok(json!({ "sessionId": id, "pinnedFingerprint": pinned_fingerprint }))
        }
        "sessions_list" => Ok(serde_json::to_value(core.sessions.list()).unwrap_or_default()),
        // What an agent ran in a session, for its tab.
        "session_runs" => {
            let a = args!(method, p, { session_id: String });
            Ok(serde_json::to_value(core.sessions.get(&a.session_id).map(|s| s.runs()).unwrap_or_default()).unwrap_or_default())
        }
        "session_attach" => {
            let a = args!(method, p, { session_id: String, stream: u64 });
            let session = core.sessions.get(&a.session_id).ok_or_else(|| err(Error::NotFound { what: "session".into() }))?;
            core.sessions.attach(&session, ctx.id, a.stream, ctx.out.clone());
            Ok(Value::Null)
        }
        "session_detach" => {
            let a = args!(method, p, { session_id: String, stream: u64 });
            if let Some(session) = core.sessions.get(&a.session_id) {
                core.sessions.detach(&session, a.stream);
            }
            Ok(Value::Null)
        }
        "session_input" => {
            let a = args!(method, p, { session_id: String, data: String });
            core.sessions.get(&a.session_id).ok_or_else(|| err(Error::NotFound { what: "session".into() }))?.input(a.data.into_bytes());
            Ok(Value::Null)
        }
        "session_resize" => {
            let a = args!(method, p, { session_id: String, cols: u32, rows: u32 });
            core.sessions.get(&a.session_id).ok_or_else(|| err(Error::NotFound { what: "session".into() }))?.resize(a.cols, a.rows);
            Ok(Value::Null)
        }
        "session_close" => {
            let a = args!(method, p, { session_id: String });
            if let Some(s) = core.sessions.get(&a.session_id) {
                s.close();
            }
            Ok(Value::Null)
        }

        /* ---------- commands and export ---------- */
        // Runs a command: stdout to `stream`, stderr to `errStream`, as raw bytes; returns its exit status.
        "host_exec" => {
            let a = args!(method, p, { vault_id: String, host_id: String, command: String, secret: Option<String>, trust_fingerprint: Option<String>, stream: Option<u64>, err_stream: Option<u64> });
            let request = s!().connect_request_in(&a.vault_id, &a.host_id, a.secret, a.trust_fingerprint).map_err(err)?;
            let send = |stream: Option<u64>| {
                let out = ctx.out.clone();
                move |bytes: &[u8]| {
                    if let Some(stream) = stream {
                        let _ = out.send(Frame::Data { stream, bytes: bytes.to_vec() });
                    }
                }
            };
            let (status, new_keys) = ssh::run_command(&request, &a.command, send(a.stream), send(a.err_stream)).await.map_err(err)?;
            s!().record_connection_in(&a.vault_id, &a.host_id, new_keys).map_err(err)?;
            Ok(json!({ "exitStatus": status }))
        }
        // `~/.ssh/config` entries for hosts of a vault; with `keyDir` their keys are saved there as files.
        "hosts_export" => {
            let a = args!(method, p, { vault_id: String, host_ids: Option<Vec<String>>, key_dir: Option<String> });
            let s = s!();
            let personal = s.view().map_err(err)?;
            let (hosts, mut keys) = if a.vault_id == PERSONAL { (personal.hosts, personal.keys.clone()) } else {
                let team = s.team_view(&a.vault_id).map_err(err)?;
                (team.data.hosts, team.data.keys)
            };
            // An own login for a team host uses a personal key.
            if a.vault_id != PERSONAL {
                keys.extend(personal.keys);
            }
            let picked: Vec<_> = match &a.host_ids {
                Some(ids) => ids.iter().filter_map(|id| hosts.iter().find(|h| &h.id == id).cloned()).collect(),
                None => hosts.clone(),
            };
            let mut paths = std::collections::HashMap::new();
            if let Some(dir) = &a.key_dir {
                for k in detour_core::export::keys_of(&detour_core::export::with_jumps(&picked, &hosts), &keys) {
                    let vault = if a.vault_id != PERSONAL && s.team_view(&a.vault_id).is_ok_and(|t| t.data.keys.iter().any(|x| x.id == k.id)) { a.vault_id.as_str() } else { PERSONAL };
                    let text = s.export_key_in(vault, &k.id, None).map_err(err)?;
                    let path = detour_core::keys::save_file(std::path::Path::new(dir), &detour_core::export::key_file_name(&k.name), &text).map_err(err)?;
                    paths.insert(k.id.clone(), path.display().to_string());
                }
            }
            let config = detour_core::export::ssh_config(&picked, &hosts, &keys, &paths);
            Ok(json!({ "config": config, "keyFiles": paths.values().collect::<Vec<_>>() }))
        }

        /* ---------- tunnels ---------- */
        "tunnel_start" => {
            let a = args!(method, p, { vault_id: String, tunnel_id: String, secret: Option<String>, trust_fingerprint: Option<String> });
            ok(tunnel::start_in(store, &core.tunnels, &a.vault_id, &a.tunnel_id, a.secret, a.trust_fingerprint).await)
        }
        "tunnel_stop" => {
            let a = args!(method, p, { vault_id: String, tunnel_id: String });
            core.tunnels.stop(&format!("{}/{}", a.vault_id, a.tunnel_id));
            Ok(Value::Null)
        }
        "tunnels_status" => Ok(serde_json::to_value(core.tunnels.statuses()).unwrap_or_default()),

        /* ---------- files (SFTP) ---------- */
        "files_open" => {
            let a = args!(method, p, { vault_id: String, host_id: String, secret: Option<String>, trust_fingerprint: Option<String> });
            let (id, home) = sftp::open_in(store, &core.files, &a.vault_id, &a.host_id, a.secret, a.trust_fingerprint).await.map_err(err)?;
            Ok(json!({ "id": id, "home": home }))
        }
        "files_list" => {
            let a = args!(method, p, { id: String, path: String });
            ok(core.files.list(&a.id, &a.path).await)
        }
        "files_resolve" => {
            let a = args!(method, p, { id: String, path: String });
            ok(core.files.resolve(&a.id, &a.path).await)
        }
        "files_mkdir" => {
            let a = args!(method, p, { id: String, path: String });
            ok(core.files.mkdir(&a.id, &a.path).await)
        }
        "files_rename" => {
            let a = args!(method, p, { id: String, from: String, to: String });
            ok(core.files.rename(&a.id, &a.from, &a.to).await)
        }
        "files_remove" => {
            let a = args!(method, p, { id: String, path: String, transfer: String, stream: Option<u64> });
            ok(core.files.remove(&a.id, &a.path, &a.transfer, progress(ctx, a.stream)).await)
        }
        "files_download" => {
            let a = args!(method, p, { id: String, remote: String, local: String, transfer: String, stream: Option<u64> });
            ok(core.files.download(&a.id, &a.remote, std::path::Path::new(&a.local), &a.transfer, progress(ctx, a.stream)).await)
        }
        "files_upload" => {
            let a = args!(method, p, { id: String, local: String, remote: String, replace: bool, transfer: String, stream: Option<u64> });
            ok(core.files.upload(&a.id, std::path::Path::new(&a.local), &a.remote, a.replace, &a.transfer, progress(ctx, a.stream)).await)
        }
        "files_download_into" => {
            let a = args!(method, p, { id: String, remote: String, local_dir: String, transfer: String, stream: Option<u64> });
            let to = core.files.download_into(&a.id, &a.remote, std::path::Path::new(&a.local_dir), &a.transfer, progress(ctx, a.stream)).await.map_err(err)?;
            Ok(json!(to.display().to_string()))
        }
        "files_chmod" => {
            let a = args!(method, p, { id: String, path: String, mode: u32 });
            ok(core.files.chmod(&a.id, &a.path, a.mode).await)
        }
        "files_cancel" => {
            let a = args!(method, p, { transfer: String });
            core.files.cancel(&a.transfer);
            Ok(Value::Null)
        }
        "files_close" => {
            let a = args!(method, p, { id: String });
            core.files.close(&a.id);
            Ok(Value::Null)
        }

        _ => Err(json!({ "kind": "unknownMethod", "message": method })),
    }
}
