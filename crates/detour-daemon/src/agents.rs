//! AI agents in the core.
//!
//! An agent reaches the core like any program of this user, so nothing it says
//! about itself is trusted: every call names its token, and the token says which
//! hosts it may use and how. It gets a token by asking (`agent_request`); the
//! person answers in the window or with `detour approve`. Its sessions, file
//! browsers and tunnels are its own: another one's id is refused. Keys and
//! passwords are used for it, never sent to it, and it can't accept a server key.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use detour_core::model::{Millis, now_ms};
use detour_core::ssh;
use detour_core::store::{PERSONAL, Store};
use detour_core::tokens::{AgentToken, HostRef, Scope};
use detour_core::{Error, new_id};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::rpc::{self, Out, err, poisoned};
use crate::sessions::{Owner, SessionInfo};
use crate::{Core, Ctx};

/// How long a request waits for the person before it is dropped.
const KEEP: Duration = Duration::from_secs(10 * 60);
/// How long a command of a `connect` token waits for "Allow".
const ASK_FOR: Duration = Duration::from_secs(120);
/// The longest an agent may wait in one call.
const MAX_WAIT: u64 = 300;
/// How many requests may wait at once.
const MAX_PENDING: usize = 10;
/// How much of a command's output an agent gets back, per stream.
const MAX_OUTPUT: usize = 1024 * 1024;

/// For how long the person allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ttl {
    #[serde(rename = "2h")]
    Hours2,
    #[serde(rename = "1d")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "never")]
    Never,
}

impl Ttl {
    fn expires_at(self) -> Option<Millis> {
        let hours = match self {
            Ttl::Hours2 => 2,
            Ttl::Day => 24,
            Ttl::Week => 24 * 7,
            Ttl::Never => return None,
        };
        Some(now_ms() + hours * 3600 * 1000)
    }
}

/// A host as the person sees it when asked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLabel {
    pub vault_id: String,
    pub host_id: String,
    pub label: String,
    /// The team it is in; none for the personal vault.
    pub team: Option<String>,
}

/// What an agent asks the person for.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Ask {
    /// A token: hosts, what on them, for how long.
    Access { agent: String, hosts: Vec<HostLabel>, scopes: Vec<Scope>, ttl: Ttl, reason: Option<String> },
    /// To run a command (or type something) in its session, for a token without `exec`.
    Command { token_id: String, agent: String, host: HostLabel, command: String },
}

#[derive(Debug, Clone)]
enum Answer {
    Denied,
    Allowed,
    /// The new token's secret, until the agent takes it.
    Token { id: String, secret: String, expires_at: Option<Millis> },
}

struct Pending {
    ask: Ask,
    at: Millis,
    answer: watch::Sender<Option<Answer>>,
}

/// A request as the window and `detour approve` show it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingView {
    pub id: String,
    pub at: Millis,
    #[serde(flatten)]
    pub ask: Ask,
}

/// A token as Settings → Agents shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenView {
    id: String,
    agent: String,
    hosts: Vec<HostLabel>,
    scopes: BTreeSet<Scope>,
    created_at: Millis,
    expires_at: Option<Millis>,
    expired: bool,
    last_used: Option<Millis>,
}

#[derive(Default)]
pub(crate) struct Agents {
    pending: Mutex<HashMap<String, Pending>>,
    /// When each token was last used, since the core started (not worth a vault write each call).
    last_used: Mutex<HashMap<String, Millis>>,
    /// File browsers and tunnels agents opened: id → token id.
    files: Mutex<HashMap<String, String>>,
    tunnels: Mutex<HashMap<String, String>>,
}

impl Agents {
    fn ask(&self, core: &Core, ask: Ask) -> Result<(String, watch::Receiver<Option<Answer>>), Value> {
        let (tx, rx) = watch::channel(None);
        let id = new_id();
        let view = {
            let mut p = self.pending.lock().unwrap();
            let now = now_ms();
            p.retain(|_, x| now - x.at < KEEP.as_millis() as i64);
            if p.len() >= MAX_PENDING {
                return Err(json!({ "kind": "tooManyRequests" }));
            }
            p.insert(id.clone(), Pending { ask: ask.clone(), at: now, answer: tx });
            PendingView { id: id.clone(), at: now, ask }
        };
        core.emit("agents.requested", serde_json::to_value(view).unwrap_or_default());
        Ok((id, rx))
    }

    /// Stops asking: answered, or nobody waits any more.
    fn done(&self, core: &Core, id: &str) {
        if self.pending.lock().unwrap().remove(id).is_some() {
            core.emit("agents.answered", json!({ "id": id }));
        }
    }

    pub(crate) fn pending(&self) -> Vec<PendingView> {
        let now = now_ms();
        let mut out: Vec<PendingView> = self
            .pending
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p)| now - p.at < KEEP.as_millis() as i64 && p.answer.borrow().is_none())
            .map(|(id, p)| PendingView { id: id.clone(), at: p.at, ask: p.ask.clone() })
            .collect();
        out.sort_by_key(|p| p.at);
        out
    }

    /// Ends what a token opened (it was revoked).
    fn close_all(&self, core: &Core, token_id: &str) {
        core.sessions.close_owned_by(token_id);
        self.files.lock().unwrap().retain(|id, t| {
            if t == token_id {
                core.files.close(id);
            }
            t != token_id
        });
        self.tunnels.lock().unwrap().retain(|key, t| {
            if t == token_id {
                core.tunnels.stop(key);
            }
            t != token_id
        });
    }
}

async fn wait(mut rx: watch::Receiver<Option<Answer>>, secs: u64) -> Option<Answer> {
    let got = tokio::time::timeout(Duration::from_secs(secs), rx.wait_for(Option::is_some)).await;
    match got {
        Ok(Ok(a)) => a.clone(),
        _ => None,
    }
}

/* ---------- hosts by name ---------- */

/// Every host the person has, with the vault it is in.
fn all_hosts(s: &Store) -> Result<Vec<(HostLabel, Value)>, Value> {
    let mut out = Vec::new();
    for h in s.view().map_err(err)?.hosts {
        out.push((HostLabel { vault_id: PERSONAL.into(), host_id: h.id.clone(), label: h.label.clone(), team: None }, serde_json::to_value(&h).unwrap_or_default()));
    }
    for t in s.teams() {
        let Some(view) = t.view else { continue };
        for h in view.data.hosts {
            out.push((HostLabel { vault_id: t.id.clone(), host_id: h.id.clone(), label: h.label.clone(), team: Some(view.name.clone()) }, serde_json::to_value(&h).unwrap_or_default()));
        }
    }
    Ok(out)
}

fn label_of(s: &Store, r: &HostRef) -> HostLabel {
    all_hosts(s).ok().and_then(|all| all.into_iter().map(|(l, _)| l).find(|l| l.vault_id == r.vault_id && l.host_id == r.host_id)).unwrap_or(HostLabel {
        vault_id: r.vault_id.clone(),
        host_id: r.host_id.clone(),
        label: r.host_id.clone(),
        team: None,
    })
}

/// A host by its name or id; a name in several vaults needs `team/name`.
fn resolve(s: &Store, name: &str) -> Result<HostLabel, Value> {
    let all = all_hosts(s)?;
    let lower = name.to_lowercase();
    let matches = |l: &HostLabel| {
        l.host_id == name || l.label.to_lowercase() == lower || l.team.as_ref().is_some_and(|t| format!("{}/{}", t.to_lowercase(), l.label.to_lowercase()) == lower)
    };
    let found: Vec<HostLabel> = all.into_iter().map(|(l, _)| l).filter(matches).collect();
    match found.len() {
        1 => Ok(found.into_iter().next().unwrap()),
        0 => Err(json!({ "kind": "notFound", "what": format!("host {name}") })),
        _ => Err(json!({ "kind": "ambiguousHost", "name": name, "options": found.iter().map(|l| match &l.team { Some(t) => format!("{t}/{}", l.label), None => l.label.clone() }).collect::<Vec<_>>() })),
    }
}

/* ---------- checking a token ---------- */

fn token(core: &Core, secret: &str) -> Result<AgentToken, Value> {
    let s = core.store.lock().map_err(poisoned)?;
    if !s.is_unlocked() {
        return Err(err(Error::Locked));
    }
    let t = s.agent_token_find(secret).map_err(err)?.ok_or_else(|| json!({ "kind": "tokenInvalid" }))?;
    core.agents.last_used.lock().unwrap().insert(t.id.clone(), now_ms());
    Ok(t)
}

fn need(t: &AgentToken, scope: Scope) -> Result<(), Value> {
    if t.may(scope) { Ok(()) } else { Err(json!({ "kind": "agentScope", "scope": scope })) }
}

fn need_host(t: &AgentToken, vault_id: &str, host_id: &str) -> Result<(), Value> {
    if t.reaches(vault_id, host_id) { Ok(()) } else { Err(json!({ "kind": "agentHost" })) }
}

/// The agent's own session; another one's is refused as if it didn't exist.
fn own_session(core: &Core, t: &AgentToken, id: &str) -> Result<Arc<crate::sessions::Session>, Value> {
    core.sessions.get(id).filter(|s| s.info.owner.token_id() == Some(t.id.as_str())).ok_or_else(|| json!({ "kind": "notYourSession" }))
}

/// Something that runs on a host: free with `exec`, asked each time with `connect`.
async fn may_run(core: &Core, t: &AgentToken, session: &SessionInfo, command: &str) -> Result<(), Value> {
    if t.may(Scope::Exec) {
        return Ok(());
    }
    need(t, Scope::Connect)?;
    let host = label_of(&*core.store.lock().map_err(poisoned)?, &HostRef { vault_id: session.vault_id.clone(), host_id: session.host_id.clone() });
    let (id, rx) = core.agents.ask(core, Ask::Command { token_id: t.id.clone(), agent: t.agent.clone(), host, command: command.to_string() })?;
    let answer = wait(rx, ASK_FOR.as_secs()).await;
    core.agents.done(core, &id);
    match answer {
        Some(Answer::Allowed) => Ok(()),
        Some(_) => Err(json!({ "kind": "denied" })),
        None => Err(json!({ "kind": "approvalTimedOut", "window": core.has_window() })),
    }
}

/// An agent's connection never forwards ssh-agent: from its host, the person's keys
/// would reach hosts its token doesn't name.
fn no_forwarding(mut r: ssh::ConnectRequest) -> ssh::ConnectRequest {
    r.host.forward_agent = false;
    r
}

fn strip_secrets(mut p: Value) -> Value {
    if let Some(o) = p.as_object_mut() {
        // An agent can't log in with a password of its own, nor accept a server key.
        o.remove("secret");
        o.remove("trustFingerprint");
    }
    p
}

fn str_arg<'a>(p: &'a Value, key: &str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or("")
}

/* ---------- what agents call ---------- */

pub(crate) async fn call(core: &Arc<Core>, ctx: &Ctx, method: &str, p: Value) -> Out {
    macro_rules! a {
        ({ $($name:ident : $ty:ty),* $(,)? }) => {{
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A { $($name: $ty),* }
            rpc::args::<A>(method, p)?
        }};
    }
    match method {
        // Every host, so an agent knows what to ask for; nothing secret is in it.
        "agent_hosts" => {
            let a = a!({ token: Option<String> });
            let t = a.token.as_deref().map(|s| token(core, s)).transpose()?;
            let s = core.store.lock().map_err(poisoned)?;
            if !s.is_unlocked() {
                return Err(err(Error::Locked));
            }
            let hosts: Vec<Value> = all_hosts(&s)?
                .into_iter()
                .map(|(l, h)| {
                    let granted = t.as_ref().is_some_and(|t| t.reaches(&l.vault_id, &l.host_id));
                    json!({ "id": l.host_id, "vaultId": l.vault_id, "team": l.team, "name": l.label, "address": h["address"], "port": h["port"], "user": h["user"], "group": h["group"], "tags": h["tags"], "granted": granted })
                })
                .collect();
            Ok(json!(hosts))
        }
        "agent_request" => {
            let a = a!({ agent: String, hosts: Vec<String>, scopes: Vec<Scope>, ttl: Option<Ttl>, reason: Option<String>, wait_secs: Option<u64> });
            if a.hosts.is_empty() || a.scopes.is_empty() {
                return Err(json!({ "kind": "badArguments", "message": "hosts and scopes" }));
            }
            let hosts = {
                let s = core.store.lock().map_err(poisoned)?;
                if !s.is_unlocked() {
                    return Err(err(Error::Locked));
                }
                let mut hosts: Vec<HostLabel> = Vec::new();
                for name in &a.hosts {
                    let h = resolve(&s, name)?;
                    if !hosts.iter().any(|x| x.vault_id == h.vault_id && x.host_id == h.host_id) {
                        hosts.push(h);
                    }
                }
                hosts
            };
            let scopes: Vec<Scope> = a.scopes.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
            let agent = a.agent.trim().chars().take(64).collect::<String>();
            let reason = a.reason.map(|r| r.chars().take(500).collect());
            let (id, rx) = core.agents.ask(core, Ask::Access { agent, hosts, scopes, ttl: a.ttl.unwrap_or(Ttl::Hours2), reason })?;
            answered(core, &id, rx, a.wait_secs.unwrap_or(0)).await
        }
        "agent_wait" => {
            let a = a!({ request_id: String, wait_secs: Option<u64> });
            let rx = core.agents.pending.lock().unwrap().get(&a.request_id).filter(|p| matches!(p.ask, Ask::Access { .. })).map(|p| p.answer.subscribe());
            let Some(rx) = rx else { return Err(json!({ "kind": "notFound", "what": "request" })) };
            answered(core, &a.request_id, rx, a.wait_secs.unwrap_or(0)).await
        }
        "agent_whoami" => {
            let a = a!({ token: String });
            let t = token(core, &a.token)?;
            Ok(serde_json::to_value(view(core, &t)).unwrap_or_default())
        }

        /* ---------- sessions ---------- */
        "agent_session_open" => {
            let a = a!({ token: String, host: String, cols: Option<u32>, rows: Option<u32> });
            let t = token(core, &a.token)?;
            if !t.may(Scope::Connect) && !t.may(Scope::Exec) {
                return Err(json!({ "kind": "agentScope", "scope": Scope::Connect }));
            }
            let (h, request) = {
                let s = core.store.lock().map_err(poisoned)?;
                let h = resolve(&s, &a.host)?;
                need_host(&t, &h.vault_id, &h.host_id)?;
                let request = no_forwarding(s.connect_request_in(&h.vault_id, &h.host_id, None, None).map_err(err)?);
                (h, request)
            };
            let size = (a.cols.unwrap_or(120), a.rows.unwrap_or(40));
            let connected = ssh::connect(request, size.0, size.1).await.map_err(err)?;
            core.store.lock().map_err(poisoned)?.record_connection_in(&h.vault_id, &h.host_id, connected.new_keys).map_err(err)?;
            let id = format!("a{:x}{:x}", now_ms(), ctx.id);
            let info = SessionInfo { id: id.clone(), vault_id: h.vault_id, host_id: h.host_id, owner: Owner::Agent { token_id: t.id.clone(), agent: t.agent.clone() }, started_at: now_ms(), transient: false };
            let c = core.clone();
            core.sessions.add(info, size, connected.handle, connected.events, None, move || c.emit("sessions.changed", json!({ "by": 0 })));
            core.emit("sessions.changed", json!({ "by": ctx.id }));
            Ok(json!({ "sessionId": id, "host": h.label }))
        }
        "agent_sessions" => {
            let a = a!({ token: String });
            let t = token(core, &a.token)?;
            let mine: Vec<SessionInfo> = core.sessions.list().into_iter().filter(|s| s.owner.token_id() == Some(t.id.as_str())).collect();
            Ok(serde_json::to_value(mine).unwrap_or_default())
        }
        "agent_session_close" => {
            let a = a!({ token: String, session_id: String });
            let t = token(core, &a.token)?;
            own_session(core, &t, &a.session_id)?.close();
            Ok(Value::Null)
        }
        "agent_input" => {
            let a = a!({ token: String, session_id: String, data: String });
            let t = token(core, &a.token)?;
            let session = own_session(core, &t, &a.session_id)?;
            may_run(core, &t, &session.info, &a.data).await?;
            session.input(a.data.into_bytes());
            Ok(Value::Null)
        }
        // The session's recent output as it came (escape sequences included).
        "agent_screen" => {
            let a = a!({ token: String, session_id: String, bytes: Option<usize> });
            let t = token(core, &a.token)?;
            let session = own_session(core, &t, &a.session_id)?;
            let tail = session.recent(a.bytes.unwrap_or(64 * 1024).min(1024 * 1024));
            let (cols, rows) = session.size();
            Ok(json!({ "output": String::from_utf8_lossy(&tail), "cols": cols, "rows": rows }))
        }
        // A command on the session's host, on a channel of its own: its output comes back
        // whole. The person sees it in the session's tab, beside the terminal.
        "agent_run" => {
            let a = a!({ token: String, session_id: String, command: String, timeout_secs: Option<u64> });
            let t = token(core, &a.token)?;
            let session = own_session(core, &t, &a.session_id)?;
            may_run(core, &t, &session.info, &a.command).await?;
            let request = no_forwarding(core.store.lock().map_err(poisoned)?.connect_request_in(&session.info.vault_id, &session.info.host_id, None, None).map_err(err)?);
            let run = session.run_started(&a.command, now_ms());
            let ran = || core.emit("session.runs", json!({ "sessionId": session.info.id }));
            ran();
            let (out, errs) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
            let collect = |into: Arc<Mutex<Vec<u8>>>| {
                let s = session.clone();
                move |bytes: &[u8]| {
                    s.run_output(run, bytes);
                    let mut v = into.lock().unwrap();
                    let room = MAX_OUTPUT.saturating_sub(v.len());
                    v.extend_from_slice(&bytes[..bytes.len().min(room)]);
                }
            };
            let secs = a.timeout_secs.unwrap_or(60).clamp(1, 3600);
            let result = tokio::time::timeout(Duration::from_secs(secs), ssh::run_command(&request, &a.command, &[], collect(out.clone()), collect(errs.clone()))).await;
            let (status, timed_out) = match result {
                Ok(Ok((status, keys))) => {
                    let _ = core.store.lock().map_err(poisoned)?.record_connection_in(&session.info.vault_id, &session.info.host_id, keys);
                    (status, false)
                }
                Ok(Err(e)) => {
                    session.run_output(run, e.to_string().as_bytes());
                    session.run_finished(run, now_ms(), None, false);
                    ran();
                    return Err(err(e));
                }
                Err(_) => (None, true),
            };
            session.run_finished(run, now_ms(), status, timed_out);
            ran();
            let text = |v: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&v.lock().unwrap()).to_string();
            Ok(json!({ "stdout": text(&out), "stderr": text(&errs), "exitStatus": status, "timedOut": timed_out }))
        }

        /* ---------- everything else, through the person's methods ---------- */
        "agent_call" => {
            let a = a!({ token: String, method: String, params: Option<Value> });
            let t = token(core, &a.token)?;
            let (inner, params) = (a.method.as_str(), strip_secrets(a.params.unwrap_or_else(|| json!({}))));
            let owned_files = |p: &Value| -> Result<(), Value> {
                let id = str_arg(p, "id");
                if core.agents.files.lock().unwrap().get(id) == Some(&t.id) { Ok(()) } else { Err(json!({ "kind": "notYourFiles" })) }
            };
            match inner {
                "files_open" => {
                    need(&t, Scope::FilesRead)?;
                    need_host(&t, str_arg(&params, "vaultId"), str_arg(&params, "hostId"))?;
                }
                "files_list" | "files_resolve" | "files_download" | "files_download_into" => {
                    need(&t, Scope::FilesRead)?;
                    owned_files(&params)?;
                }
                "files_upload" | "files_mkdir" | "files_rename" | "files_remove" | "files_chmod" => {
                    need(&t, Scope::FilesWrite)?;
                    owned_files(&params)?;
                }
                "files_close" => owned_files(&params)?,
                "files_cancel" => need(&t, Scope::FilesRead)?,
                "tunnel_start" | "tunnel_stop" => {
                    need(&t, Scope::Tunnels)?;
                    let (vault, tunnel) = (str_arg(&params, "vaultId"), str_arg(&params, "tunnelId"));
                    let via = tunnel_host(core, vault, tunnel)?;
                    need_host(&t, vault, &via)?;
                    if inner == "tunnel_stop" && core.agents.tunnels.lock().unwrap().get(&format!("{vault}/{tunnel}")) != Some(&t.id) {
                        return Err(json!({ "kind": "notYourTunnel" }));
                    }
                }
                "tunnels_status" => need(&t, Scope::Tunnels)?,
                "vault_get" => {}
                "vault_put" => {
                    need(&t, Scope::VaultEdit)?;
                    // Only its own changes: an agent never sends the whole vault over what it didn't see.
                    if params.get("base").is_none_or(Value::is_null) {
                        return Err(json!({ "kind": "badArguments", "message": "base" }));
                    }
                }
                "key_generate" | "key_import" => {
                    need(&t, Scope::VaultEdit)?;
                    if str_arg(&params, "vaultId") != PERSONAL {
                        return Err(json!({ "kind": "forbidden" }));
                    }
                }
                "sync_now" => need(&t, Scope::VaultEdit)?,
                _ => return Err(json!({ "kind": "forbidden" })),
            }
            let tunnel_key = format!("{}/{}", str_arg(&params, "vaultId"), str_arg(&params, "tunnelId"));
            let files_id = str_arg(&params, "id").to_string();
            let r = rpc::dispatch(core, ctx, inner, params).await?;
            match inner {
                "files_open" => {
                    core.agents.files.lock().unwrap().insert(str_arg(&r, "id").to_string(), t.id.clone());
                }
                "files_close" => {
                    core.agents.files.lock().unwrap().remove(&files_id);
                }
                "tunnel_start" => {
                    core.agents.tunnels.lock().unwrap().insert(tunnel_key, t.id.clone());
                }
                _ => {}
            }
            if !rpc::READ_ONLY.contains(&inner) {
                core.emit("data.changed", json!({ "by": ctx.id, "method": inner }));
            }
            Ok(r)
        }
        _ => Err(json!({ "kind": "unknownMethod", "message": method })),
    }
}

/// Tells the agent how its request went, waiting for the person up to `secs`.
async fn answered(core: &Core, id: &str, rx: watch::Receiver<Option<Answer>>, secs: u64) -> Out {
    let answer = if secs == 0 { rx.borrow().clone() } else { wait(rx, secs.min(MAX_WAIT)).await };
    match answer {
        None => Ok(json!({ "status": "pending", "requestId": id, "window": core.has_window() })),
        Some(a) => {
            core.agents.done(core, id);
            match a {
                Answer::Token { id, secret, expires_at } => Ok(json!({ "status": "approved", "token": secret, "tokenId": id, "expiresAt": expires_at })),
                _ => Ok(json!({ "status": "denied" })),
            }
        }
    }
}

fn tunnel_host(core: &Core, vault: &str, tunnel: &str) -> Result<String, Value> {
    let s = core.store.lock().map_err(poisoned)?;
    let tunnels = if vault == PERSONAL { s.view().map_err(err)?.tunnels } else { s.team_view(vault).map_err(err)?.data.tunnels };
    tunnels.into_iter().find(|t| t.id == tunnel).map(|t| t.via_host_id).ok_or_else(|| err(Error::NotFound { what: "tunnel".into() }))
}

fn view(core: &Core, t: &AgentToken) -> TokenView {
    let hosts = core.store.lock().map(|s| t.hosts.iter().map(|r| label_of(&s, r)).collect()).unwrap_or_default();
    TokenView {
        id: t.id.clone(),
        agent: t.agent.clone(),
        hosts,
        scopes: t.scopes.clone(),
        created_at: t.created_at,
        expires_at: t.expires_at,
        expired: t.is_expired(),
        last_used: core.agents.last_used.lock().unwrap().get(&t.id).copied(),
    }
}

/* ---------- what the person calls ---------- */

pub(crate) async fn person(core: &Arc<Core>, method: &str, p: Value) -> Out {
    match method {
        "agents_pending" => Ok(serde_json::to_value(core.agents.pending()).unwrap_or_default()),
        "agents_list" => {
            let tokens = core.store.lock().map_err(poisoned)?.agent_tokens().map_err(err)?;
            Ok(json!(tokens.iter().map(|t| view(core, t)).collect::<Vec<_>>()))
        }
        // Allows or denies a request; for access, the person may keep fewer hosts and scopes than asked.
        "agents_answer" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                id: String,
                allow: bool,
                ttl: Option<Ttl>,
                scopes: Option<Vec<Scope>>,
                hosts: Option<Vec<HostRef>>,
                /// For a command: allow its token every command from now on.
                always: Option<bool>,
            }
            let a: A = rpc::args(method, p)?;
            let (ask, tx) = {
                let pending = core.agents.pending.lock().unwrap();
                let x = pending.get(&a.id).ok_or_else(|| json!({ "kind": "notFound", "what": "request" }))?;
                (x.ask.clone(), x.answer.clone())
            };
            if !a.allow {
                tx.send_replace(Some(Answer::Denied));
                if matches!(ask, Ask::Command { .. }) {
                    core.agents.done(core, &a.id);
                } else {
                    core.emit("agents.answered", json!({ "id": a.id }));
                }
                return Ok(Value::Null);
            }
            match ask {
                Ask::Access { agent, hosts, scopes, ttl, .. } => {
                    let asked: BTreeSet<HostRef> = hosts.iter().map(|h| HostRef { vault_id: h.vault_id.clone(), host_id: h.host_id.clone() }).collect();
                    let keep_hosts: BTreeSet<HostRef> = a.hosts.map(|h| h.into_iter().filter(|r| asked.contains(r)).collect()).unwrap_or(asked);
                    let keep_scopes: BTreeSet<Scope> = a.scopes.map(|s| s.into_iter().filter(|x| scopes.contains(x)).collect()).unwrap_or_else(|| scopes.into_iter().collect());
                    if keep_hosts.is_empty() || keep_scopes.is_empty() {
                        return Err(json!({ "kind": "badArguments", "message": "hosts and scopes" }));
                    }
                    let expires_at = a.ttl.unwrap_or(ttl).expires_at();
                    let (token, secret) = core.store.lock().map_err(poisoned)?.agent_token_add(&agent, keep_hosts, keep_scopes, expires_at).map_err(err)?;
                    tx.send_replace(Some(Answer::Token { id: token.id.clone(), secret: secret.clone(), expires_at }));
                    core.emit("agents.answered", json!({ "id": a.id }));
                    core.emit("agents.changed", Value::Null);
                    Ok(json!({ "tokenId": token.id, "token": secret }))
                }
                Ask::Command { token_id, .. } => {
                    if a.always == Some(true) {
                        core.store.lock().map_err(poisoned)?.agent_token_scope_add(&token_id, Scope::Exec).map_err(err)?;
                        core.emit("agents.changed", Value::Null);
                    }
                    tx.send_replace(Some(Answer::Allowed));
                    core.agents.done(core, &a.id);
                    Ok(Value::Null)
                }
            }
        }
        // Takes a token away; what it opened closes at once.
        "agents_revoke" => {
            let a: TokenId = rpc::args(method, p)?;
            let removed = core.store.lock().map_err(poisoned)?.agent_token_remove(&a.id).map_err(err)?;
            core.agents.close_all(core, &a.id);
            if removed {
                core.emit("agents.changed", Value::Null);
            }
            Ok(json!(removed))
        }
        _ => Err(json!({ "kind": "unknownMethod", "message": method })),
    }
}

#[derive(Deserialize)]
struct TokenId {
    id: String,
}
