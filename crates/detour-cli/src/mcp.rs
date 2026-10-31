//! `detour mcp`: Detour for AI agents, as an MCP server on stdio.
//!
//! It is a client of the core like the window and the command line, but trusted with
//! nothing: everything goes through `agent_*` methods and the tokens the person grants
//! (see `detour_daemon::agents`). It asks for a token when the agent asks for access and
//! keeps the tokens it got (and those in `$DETOUR_TOKEN`) for as long as it runs.
//! What it says is read by a model, so it is in English and says what to do next.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use detour_ipc::Client;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::{Peer, RoleServer};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

type Out = Result<String, String>;

/// How long a request for access waits for the person in one call (MCP clients time out).
const WAIT_SECS: u64 = 100;
/// The most of a remote file `read_file` returns.
const MAX_READ: u64 = 1024 * 1024;

const INSTRUCTIONS: &str = "Detour is the person's SSH client. Through it you work on their servers without seeing \
their passwords or keys. Start with list_hosts. Before using a host, call request_access with the hosts, the scopes you \
need and why; the person approves it in the Detour window (or with `detour approve` in a terminal). Then open_session \
on the host and use run_command in it: the person sees your session and every command in Detour. With the `connect` \
scope each command waits for the person's approval; with `exec` it runs at once. Ask only for what the task needs.";

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, serde::Serialize)]
enum Scope {
    /// Open terminal sessions; each command waits for the person's approval.
    #[serde(rename = "connect")]
    Connect,
    /// Run commands and type into sessions without asking each time.
    #[serde(rename = "exec")]
    Exec,
    /// List, read and download files.
    #[serde(rename = "files:read")]
    FilesRead,
    /// Write, upload, rename, remove files and change permissions.
    #[serde(rename = "files:write")]
    FilesWrite,
    /// Start and stop the person's saved tunnels through the host.
    #[serde(rename = "tunnels")]
    Tunnels,
    /// Add and change hosts, snippets and keys in the person's vault.
    #[serde(rename = "vault:edit")]
    VaultEdit,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, serde::Serialize)]
enum Duration {
    #[serde(rename = "2h")]
    Hours2,
    #[serde(rename = "1d")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "never")]
    UntilRevoked,
}

#[derive(Deserialize, JsonSchema)]
struct RequestAccess {
    /// Hosts by name, as list_hosts shows them (`team/name` for a team's host).
    hosts: Vec<String>,
    /// What you need to do there. Prefer `connect` over `exec` unless you will run many commands.
    scopes: Vec<Scope>,
    /// For how long; the person may change it. Default: 2h.
    duration: Option<Duration>,
    /// What you are going to do and why, in a sentence the person reads before approving.
    reason: String,
}

#[derive(Deserialize, JsonSchema)]
struct RequestId {
    /// The request_id request_access returned.
    request_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct HostArg {
    /// The host's name, as list_hosts shows it.
    host: String,
}

#[derive(Deserialize, JsonSchema)]
struct SessionArg {
    /// A session_id from open_session or list_sessions.
    session_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct RunCommand {
    /// The session to run it in (its host). Or give `host` and a session of yours there is used, or opened.
    session_id: Option<String>,
    /// The host, when you don't name a session.
    host: Option<String>,
    /// A shell command. It runs in a fresh shell in the home directory: `cd` does not carry over to the next command.
    command: String,
    /// Give up after this many seconds (default 60, at most 3600).
    timeout_secs: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct SendInput {
    session_id: String,
    /// What to type. Control keys as characters: "\u0003" is Ctrl+C, "\u001b" is Esc.
    text: String,
    /// Press Enter after the text (default true).
    enter: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct HostPath {
    host: String,
    /// A path on the host; relative paths start at the home directory.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteFile {
    host: String,
    /// A path on the host; relative paths start at the home directory.
    path: String,
    /// The whole new content of the file, as text.
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct Transfer {
    host: String,
    /// The path on the host.
    remote_path: String,
    /// The path on this computer.
    local_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct Rename {
    host: String,
    from: String,
    to: String,
}

#[derive(Deserialize, JsonSchema)]
struct Chmod {
    host: String,
    path: String,
    /// Octal permissions, like "755" or "644".
    mode: String,
}

#[derive(Deserialize, JsonSchema)]
struct NameArg {
    /// Its name.
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct RunSnippet {
    /// The snippet's name.
    name: String,
    /// The session to run it in. Or give `host`.
    session_id: Option<String>,
    host: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AddHost {
    /// A name for the host.
    name: String,
    /// Its address: a name or an IP.
    address: String,
    /// SSH port (default 22).
    port: Option<u16>,
    /// The user to log in as (default root).
    user: Option<String>,
    /// Log in with this key of the vault (its name). Without it, ssh-agent is used.
    key: Option<String>,
    /// The group in the host list.
    group: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateHost {
    /// The host to change.
    host: String,
    name: Option<String>,
    address: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    group: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AddSnippet {
    name: String,
    /// The command it runs.
    command: String,
}

#[derive(Deserialize, JsonSchema)]
struct GenerateKey {
    name: String,
    /// ed25519 (default), rsa or ecdsa.
    algorithm: Option<String>,
}

/// A file browser's id and the home directory it opened in.
type Browser = (String, String);

/// What an agent's token reaches, as the core says.
#[derive(Clone)]
struct Grant {
    secret: String,
    hosts: Vec<(String, String, String)>,
}

#[derive(Clone)]
pub struct Detour {
    dir: PathBuf,
    link: Arc<tokio::sync::Mutex<Option<Client>>>,
    grants: Arc<Mutex<Vec<Grant>>>,
    /// File browsers opened, by token and host (`vault/host`): their id and home directory.
    files: Arc<Mutex<HashMap<(String, String), Browser>>>,
    tool_router: ToolRouter<Self>,
}

/// What went wrong, as the agent should read it: what happened and what to do about it.
fn explain(e: &Value) -> String {
    let kind = e["kind"].as_str().unwrap_or("");
    match kind {
        "locked" => "Detour is locked. Ask the person to unlock it (in the Detour window, or `detour unlock` in a terminal), then try again.".into(),
        "notInitialized" => "Detour isn't set up on this computer yet. Ask the person to open Detour and create a vault.".into(),
        "tokenInvalid" => "Your access has expired or was revoked. Call request_access again.".into(),
        "agentScope" => format!("Your access doesn't include the `{}` scope. Call request_access asking for it.", e["scope"].as_str().unwrap_or("")),
        "agentHost" => "You have no access to this host. Call request_access for it first.".into(),
        "notYourSession" => "There is no session of yours with this id. Open one with open_session; list yours with list_sessions.".into(),
        "notYourFiles" | "notYourTunnel" => "That isn't yours: it was opened by someone else.".into(),
        "denied" => "The person said no.".into(),
        "approvalTimedOut" => "Nobody approved the command in time. Ask the person to watch the Detour window (or run `detour approve`), then try again.".into(),
        "hostKeyUnknown" | "hostKeyChanged" => "Detour has not confirmed this server's key. Ask the person to connect to the host once in Detour and confirm its key; you can't do that.".into(),
        "needSecret" | "authFailed" | "jumpNeedsLogin" => "The host's login needs a password or passphrase Detour has not saved. Ask the person to save it for this host in Detour.".into(),
        "ambiguousHost" => format!("Several hosts are called that: {}. Use one of these names.", e["options"].as_array().map(|o| o.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
        "notFound" => format!("Not found: {}.", e["what"].as_str().unwrap_or("")),
        "unreachable" => format!("{} is not responding.", e["endpoint"].as_str().unwrap_or("The host")),
        "forbidden" => "Agents can't do that through Detour.".into(),
        "tooManyRequests" => "Too many requests are waiting for the person. Wait for an answer to those first.".into(),
        "disconnected" => "Lost the connection to Detour. Try again.".into(),
        _ => match e["message"].as_str() {
            Some(m) if !kind.is_empty() => format!("{kind}: {m}"),
            Some(m) => m.to_string(),
            None => e.to_string(),
        },
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn host_label(h: &Value) -> String {
    match h["team"].as_str() {
        Some(team) => format!("{team}/{}", s(&h["label"])),
        None => s(&h["label"]),
    }
}

/// Does `name` mean this host (by id, name, or team/name)?
fn names(host: &(String, String, String), name: &str) -> bool {
    let (_, id, label) = host;
    let n = name.to_lowercase();
    id == name || label.to_lowercase() == n || label.to_lowercase().rsplit('/').next() == Some(n.as_str()) && !n.contains('/')
}

/// A remote path: relative ones start at the home directory.
fn remote(home: &str, path: &str) -> String {
    if path.starts_with('/') || path.starts_with('~') && path.len() == 1 {
        return if path == "~" { home.to_string() } else { path.to_string() };
    }
    let rel = path.strip_prefix("~/").unwrap_or(path);
    format!("{}/{rel}", home.trim_end_matches('/'))
}

fn temp_path() -> PathBuf {
    std::env::temp_dir().join(format!("detour-mcp-{}", detour_core::new_id()))
}

#[tool_router]
impl Detour {
    pub fn new(dir: PathBuf) -> Self {
        let preset = std::env::var("DETOUR_TOKEN").unwrap_or_default();
        let grants = preset.split([',', ' ']).filter(|t| !t.is_empty()).map(|t| Grant { secret: t.to_string(), hosts: Vec::new() }).collect();
        Self { dir, link: Arc::default(), grants: Arc::new(Mutex::new(grants)), files: Arc::default(), tool_router: Self::tool_router() }
    }

    /// A call to the core, starting it (or reconnecting) when needed.
    async fn core(&self, method: &str, params: Value) -> Result<Value, Value> {
        let mut link = self.link.lock().await;
        if link.as_ref().is_none_or(Client::is_closed) {
            let me = std::env::current_exe().map_err(|e| json!({ "message": e.to_string() }))?;
            let c = detour_ipc::connect_or_start(&me, &self.dir).await.map_err(|e| json!({ "kind": "disconnected", "message": e.to_string() }))?;
            c.call("hello", json!({ "client": "mcp" })).await?;
            *link = Some(c);
        }
        let c = link.as_ref().unwrap().clone();
        drop(link);
        c.call(method, params).await
    }

    /// The tokens this server holds, with what each reaches (asked again when not known yet).
    async fn grants(&self) -> Vec<Grant> {
        let all = self.grants.lock().unwrap().clone();
        let mut out = Vec::new();
        for mut g in all {
            if g.hosts.is_empty() {
                match self.core("agent_whoami", json!({ "token": g.secret })).await {
                    Ok(who) => {
                        g.hosts = who["hosts"].as_array().into_iter().flatten().map(|h| (s(&h["vaultId"]), s(&h["hostId"]), host_label(h))).collect();
                        let mut list = self.grants.lock().unwrap();
                        if let Some(x) = list.iter_mut().find(|x| x.secret == g.secret) {
                            x.hosts = g.hosts.clone();
                        }
                    }
                    Err(e) if e["kind"] == "tokenInvalid" => {
                        self.grants.lock().unwrap().retain(|x| x.secret != g.secret);
                        continue;
                    }
                    Err(_) => {}
                }
            }
            out.push(g);
        }
        out
    }

    /// Tries each token that may fit until one works (it and the answer); the error worth reading when none does.
    async fn with_tokens(&self, fits: impl Fn(&Grant) -> bool, method: &str, params: impl Fn(&str) -> Value) -> Result<(String, Value), String> {
        let grants: Vec<Grant> = self.grants().await.into_iter().filter(|g| fits(g)).collect();
        if grants.is_empty() {
            return Err(explain(&json!({ "kind": "agentHost" })));
        }
        let mut last = json!({ "kind": "agentHost" });
        for g in grants {
            match self.core(method, params(&g.secret)).await {
                Ok(v) => return Ok((g.secret, v)),
                Err(e) => {
                    if e["kind"] == "tokenInvalid" {
                        self.grants.lock().unwrap().retain(|x| x.secret != g.secret);
                    }
                    let worth = !matches!(e["kind"].as_str(), Some("agentScope" | "agentHost" | "tokenInvalid" | "notYourSession" | "notYourFiles"));
                    last = e;
                    if worth {
                        break;
                    }
                }
            }
        }
        Err(explain(&last))
    }

    /// A host the tokens reach, by its name.
    async fn host(&self, name: &str) -> Result<(String, String, String), String> {
        for g in self.grants().await {
            if let Some(h) = g.hosts.iter().find(|h| names(h, name)) {
                return Ok(h.clone());
            }
        }
        Err(format!("You have no access to {name}. Call request_access for it first (list_hosts shows the names)."))
    }

    async fn on_host(&self, host: &(String, String, String), method: &str, params: impl Fn(&str) -> Value) -> Result<(String, Value), String> {
        let key = (host.0.clone(), host.1.clone());
        self.with_tokens(|g| g.hosts.iter().any(|h| (h.0.clone(), h.1.clone()) == key), method, params).await
    }

    /// A session of this agent: the token that owns it.
    async fn session_token(&self, id: &str) -> Result<String, String> {
        for g in self.grants().await {
            if let Ok(list) = self.core("agent_sessions", json!({ "token": g.secret })).await
                && list.as_array().into_iter().flatten().any(|x| x["id"] == id)
            {
                return Ok(g.secret);
            }
        }
        Err(explain(&json!({ "kind": "notYourSession" })))
    }

    /// The session to run something in: the one named, or one of this agent's on the host (opened if none).
    async fn session_for(&self, session_id: Option<String>, host: Option<String>) -> Result<(String, String), String> {
        if let Some(id) = session_id {
            return Ok((self.session_token(&id).await?, id));
        }
        let Some(host) = host else { return Err("Give session_id, or host.".into()) };
        let h = self.host(&host).await?;
        for g in self.grants().await {
            if let Ok(list) = self.core("agent_sessions", json!({ "token": g.secret })).await
                && let Some(x) = list.as_array().into_iter().flatten().find(|x| x["vaultId"] == h.0.as_str() && x["hostId"] == h.1.as_str())
            {
                return Ok((g.secret, s(&x["id"])));
            }
        }
        let opened = self.open(&h).await?;
        Ok((opened.0, opened.1))
    }

    async fn open(&self, h: &(String, String, String)) -> Result<(String, String), String> {
        let (used, r) = self.on_host(h, "agent_session_open", |token| json!({ "token": token, "host": h.1 })).await?;
        Ok((used, s(&r["sessionId"])))
    }

    /// A file browser on the host for some token: its id, home directory and the token.
    async fn files(&self, host: &str) -> Result<(String, String, String, (String, String, String)), String> {
        let h = self.host(host).await?;
        for g in self.grants().await {
            if let Some((id, home)) = self.files.lock().unwrap().get(&(g.secret.clone(), format!("{}/{}", h.0, h.1))).cloned() {
                return Ok((g.secret, id, home, h));
            }
        }
        let (used, r) = self.on_host(&h, "agent_call", |token| json!({ "token": token, "method": "files_open", "params": { "vaultId": h.0, "hostId": h.1 } })).await?;
        let (id, home) = (s(&r["id"]), s(&r["home"]));
        self.files.lock().unwrap().insert((used.clone(), format!("{}/{}", h.0, h.1)), (id.clone(), home.clone()));
        Ok((used, id, home, h))
    }

    /// A file operation through the person's methods, with a browser opened again if the old one closed.
    async fn file_call(&self, host: &str, method: &str, params: impl Fn(&str, &str) -> Value) -> Result<Value, String> {
        for attempt in 0..2 {
            let (token, id, home, h) = self.files(host).await?;
            let mut p = params(&id, &home);
            p["id"] = json!(id);
            match self.core("agent_call", json!({ "token": token, "method": method, "params": p })).await {
                Ok(v) => return Ok(v),
                // The browser closed (the core restarted, the token changed): open another once.
                Err(e) if attempt == 0 && (e["kind"] == "notYourFiles" || e["kind"] == "notFound" && e["what"] == "file browser") => {
                    self.files.lock().unwrap().remove(&(token, format!("{}/{}", h.0, h.1)));
                }
                Err(e) => return Err(explain(&e)),
            }
        }
        Err("The host's files could not be opened.".into())
    }

    async fn vault(&self) -> Result<Value, String> {
        Ok(self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "vault_get" })).await?.1)
    }

    /// Saves an edit of the personal vault made on `base`: only what changed goes in.
    async fn put(&self, base: &Value, next: &Value) -> Result<(), String> {
        let pick = |d: &Value| json!({ "hosts": d["hosts"], "keys": d["keys"], "snippets": d["snippets"], "tunnels": d["tunnels"] });
        let (base, next) = (pick(base), pick(next));
        self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "vault_put", "params": { "update": next, "base": base } })).await?;
        Ok(())
    }

    async fn run(&self, token: &str, session: &str, command: &str, timeout: Option<u64>) -> Out {
        let r = self.core("agent_run", json!({ "token": token, "sessionId": session, "command": command, "timeoutSecs": timeout })).await.map_err(|e| explain(&e))?;
        let mut out = match (r["timedOut"].as_bool(), r["exitStatus"].as_u64()) {
            (Some(true), _) => "Timed out; the command was stopped.\n".to_string(),
            (_, Some(code)) => format!("exit status {code}\n"),
            _ => "ended without an exit status\n".to_string(),
        };
        let (stdout, stderr) = (s(&r["stdout"]), s(&r["stderr"]));
        if !stdout.is_empty() {
            out.push_str(&format!("--- stdout ---\n{stdout}"));
            if !stdout.ends_with('\n') {
                out.push('\n');
            }
        }
        if !stderr.is_empty() {
            out.push_str(&format!("--- stderr ---\n{stderr}"));
        }
        Ok(out)
    }

    /* ---------- hosts and access ---------- */

    #[tool(description = "The person's hosts in Detour, with the access you have to each. Never shows passwords or keys.")]
    async fn list_hosts(&self) -> Out {
        let hosts = self.core("agent_hosts", json!({})).await.map_err(|e| explain(&e))?;
        let grants = self.grants().await;
        let mut scopes: HashMap<(String, String), Vec<String>> = HashMap::new();
        for g in &grants {
            if let Ok(who) = self.core("agent_whoami", json!({ "token": g.secret })).await {
                for h in g.hosts.iter() {
                    let e = scopes.entry((h.0.clone(), h.1.clone())).or_default();
                    e.extend(who["scopes"].as_array().into_iter().flatten().map(s));
                }
            }
        }
        let list: Vec<Value> = hosts
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| {
                let name = match h["team"].as_str() {
                    Some(t) => format!("{t}/{}", s(&h["name"])),
                    None => s(&h["name"]),
                };
                let mut access = scopes.get(&(s(&h["vaultId"]), s(&h["id"]))).cloned().unwrap_or_default();
                access.sort();
                access.dedup();
                json!({ "name": name, "address": h["address"], "port": h["port"], "user": h["user"], "group": h["group"], "tags": h["tags"], "access": access })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    #[tool(description = "Ask the person for access to hosts. They see your name, the hosts, the scopes and your reason, and may \
        allow less than you asked. Waits up to 100 seconds for the answer; if it is still pending, ask the person to approve it \
        and call wait_for_access.")]
    async fn request_access(&self, Parameters(a): Parameters<RequestAccess>, peer: Peer<RoleServer>) -> Out {
        let agent = peer.peer_info().map(|i| i.client_info.title.clone().unwrap_or_else(|| i.client_info.name.clone())).filter(|n| !n.is_empty()).unwrap_or_else(|| "AI agent".into());
        let r = self
            .core("agent_request", json!({ "agent": agent, "hosts": a.hosts, "scopes": a.scopes, "ttl": a.duration, "reason": a.reason, "waitSecs": WAIT_SECS }))
            .await
            .map_err(|e| explain(&e))?;
        self.answered(r).await
    }

    #[tool(description = "Wait for the person's answer to a request_access that is still pending (up to 100 seconds).")]
    async fn wait_for_access(&self, Parameters(a): Parameters<RequestId>) -> Out {
        let r = self.core("agent_wait", json!({ "requestId": a.request_id, "waitSecs": WAIT_SECS })).await.map_err(|e| explain(&e))?;
        self.answered(r).await
    }

    async fn answered(&self, r: Value) -> Out {
        match r["status"].as_str() {
            Some("approved") => {
                let secret = s(&r["token"]);
                self.grants.lock().unwrap().push(Grant { secret: secret.clone(), hosts: Vec::new() });
                let who = self.core("agent_whoami", json!({ "token": secret })).await.map_err(|e| explain(&e))?;
                let hosts: Vec<String> = who["hosts"].as_array().into_iter().flatten().map(host_label).collect();
                let scopes: Vec<String> = who["scopes"].as_array().into_iter().flatten().map(s).collect();
                Ok(format!("Allowed: {} on {}. Open a session with open_session.", scopes.join(", "), hosts.join(", ")))
            }
            Some("denied") => Err("The person denied the request.".into()),
            _ => {
                let how = if r["window"] == true { "in the Detour window" } else { "by running `detour approve` in their terminal (Detour's window is not open)" };
                Ok(format!("Still waiting for the person. Ask them to approve it {how}, then call wait_for_access with request_id \"{}\".", s(&r["requestId"])))
            }
        }
    }

    /* ---------- sessions and commands ---------- */

    #[tool(description = "Open a terminal session on a host. The person sees it as a tab in Detour.")]
    async fn open_session(&self, Parameters(a): Parameters<HostArg>) -> Out {
        let h = self.host(&a.host).await?;
        let (_, id) = self.open(&h).await?;
        Ok(format!("session_id: {id} (on {}). Run commands with run_command.", h.2))
    }

    #[tool(description = "Your open sessions.")]
    async fn list_sessions(&self) -> Out {
        let mut out = Vec::new();
        for g in self.grants().await {
            if let Ok(list) = self.core("agent_sessions", json!({ "token": g.secret })).await {
                for x in list.as_array().into_iter().flatten() {
                    let host = g.hosts.iter().find(|h| x["vaultId"] == h.0.as_str() && x["hostId"] == h.1.as_str()).map(|h| h.2.clone()).unwrap_or_else(|| s(&x["hostId"]));
                    out.push(json!({ "session_id": x["id"], "host": host }));
                }
            }
        }
        Ok(serde_json::to_string_pretty(&out).unwrap_or_default())
    }

    #[tool(description = "Close a session of yours.")]
    async fn close_session(&self, Parameters(a): Parameters<SessionArg>) -> Out {
        let token = self.session_token(&a.session_id).await?;
        self.core("agent_session_close", json!({ "token": token, "sessionId": a.session_id })).await.map_err(|e| explain(&e))?;
        Ok("Closed.".into())
    }

    #[tool(description = "Run a shell command on the session's host and get its exit status, stdout and stderr. It runs \
        non-interactively, beside the terminal (not typed into it); the person sees it in the session's tab. Don't run \
        commands that wait for input; use send_input for interactive programs.")]
    async fn run_command(&self, Parameters(a): Parameters<RunCommand>) -> Out {
        let (token, session) = self.session_for(a.session_id, a.host).await?;
        self.run(&token, &session, &a.command, a.timeout_secs).await
    }

    #[tool(description = "Type into the session's terminal, as the person would: for interactive programs. Read the result with read_screen.")]
    async fn send_input(&self, Parameters(a): Parameters<SendInput>) -> Out {
        let token = self.session_token(&a.session_id).await?;
        let mut text = a.text;
        if a.enter.unwrap_or(true) {
            text.push('\r');
        }
        self.core("agent_input", json!({ "token": token, "sessionId": a.session_id, "data": text })).await.map_err(|e| explain(&e))?;
        Ok("Sent.".into())
    }

    #[tool(description = "What the session's terminal shows now, as text.")]
    async fn read_screen(&self, Parameters(a): Parameters<SessionArg>) -> Out {
        let token = self.session_token(&a.session_id).await?;
        let r = self.core("agent_screen", json!({ "token": token, "sessionId": a.session_id, "bytes": 256 * 1024 })).await.map_err(|e| explain(&e))?;
        let (cols, rows) = (r["cols"].as_u64().unwrap_or(120).clamp(10, 1000) as u16, r["rows"].as_u64().unwrap_or(40).clamp(5, 500) as u16);
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN);
        let mut parser = vt100::Parser::new(size(rows), size(cols), 0);
        parser.process(s(&r["output"]).as_bytes());
        let text = parser.screen().contents();
        Ok(text.trim_end().trim_start_matches('\n').to_string())
    }

    /* ---------- files ---------- */

    #[tool(description = "List a folder on a host.")]
    async fn list_dir(&self, Parameters(a): Parameters<HostPath>) -> Out {
        let path = a.path.clone();
        let r = self.file_call(&a.host, "files_list", |_, home| json!({ "path": remote(home, &path) })).await?;
        let lines: Vec<String> = r
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                let dir = e["kind"] == "dir";
                format!("{}{}\t{}", s(&e["name"]), if dir { "/" } else { "" }, if dir { String::new() } else { s(&e["size"]) })
            })
            .collect();
        Ok(if lines.is_empty() { "(empty)".into() } else { lines.join("\n") })
    }

    #[tool(description = "Read a text file on a host (up to 1 MB).")]
    async fn read_file(&self, Parameters(a): Parameters<HostPath>) -> Out {
        let local = temp_path();
        let path = a.path.clone();
        let l = local.display().to_string();
        let stat = self.file_call(&a.host, "files_list", |_, home| json!({ "path": parent(&remote(home, &path)) })).await;
        if let Ok(list) = stat {
            let name = path.rsplit('/').next().unwrap_or(&path);
            if let Some(e) = list.as_array().into_iter().flatten().find(|e| e["name"] == name)
                && e["size"].as_u64().unwrap_or(0) > MAX_READ
            {
                return Err(format!("The file is {} bytes, more than read_file returns. Use download, or run_command with head/tail/grep.", s(&e["size"])));
            }
        }
        self.file_call(&a.host, "files_download", |_, home| json!({ "remote": remote(home, &path), "local": l, "transfer": detour_core::new_id() })).await?;
        let bytes = std::fs::read(&local).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&local);
        let bytes = bytes?;
        if bytes.contains(&0) {
            return Err("It is a binary file. Use download to get it.".into());
        }
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    #[tool(description = "Write a text file on a host, replacing it if it exists.")]
    async fn write_file(&self, Parameters(a): Parameters<WriteFile>) -> Out {
        let local = temp_path();
        std::fs::write(&local, a.content.as_bytes()).map_err(|e| e.to_string())?;
        let (path, l) = (a.path.clone(), local.display().to_string());
        let r = self.file_call(&a.host, "files_upload", |_, home| json!({ "local": l, "remote": remote(home, &path), "replace": true, "transfer": detour_core::new_id() })).await;
        let _ = std::fs::remove_file(&local);
        r?;
        Ok("Written.".into())
    }

    #[tool(description = "Download a file or folder from a host to this computer.")]
    async fn download(&self, Parameters(a): Parameters<Transfer>) -> Out {
        let (path, local) = (a.remote_path.clone(), absolute(&a.local_path));
        self.file_call(&a.host, "files_download", |_, home| json!({ "remote": remote(home, &path), "local": local, "transfer": detour_core::new_id() })).await?;
        Ok(format!("Downloaded to {}.", absolute(&a.local_path)))
    }

    #[tool(description = "Upload a file or folder from this computer to a host. An existing file there is replaced.")]
    async fn upload(&self, Parameters(a): Parameters<Transfer>) -> Out {
        let (path, local) = (a.remote_path.clone(), absolute(&a.local_path));
        self.file_call(&a.host, "files_upload", |_, home| json!({ "local": local, "remote": remote(home, &path), "replace": true, "transfer": detour_core::new_id() })).await?;
        Ok("Uploaded.".into())
    }

    #[tool(description = "Create a folder on a host.")]
    async fn make_dir(&self, Parameters(a): Parameters<HostPath>) -> Out {
        let path = a.path.clone();
        self.file_call(&a.host, "files_mkdir", |_, home| json!({ "path": remote(home, &path) })).await?;
        Ok("Created.".into())
    }

    #[tool(description = "Rename or move a file or folder on a host.")]
    async fn rename(&self, Parameters(a): Parameters<Rename>) -> Out {
        let (from, to) = (a.from.clone(), a.to.clone());
        self.file_call(&a.host, "files_rename", |_, home| json!({ "from": remote(home, &from), "to": remote(home, &to) })).await?;
        Ok("Renamed.".into())
    }

    #[tool(description = "Remove a file, or a folder with everything in it, on a host.")]
    async fn remove(&self, Parameters(a): Parameters<HostPath>) -> Out {
        let path = a.path.clone();
        self.file_call(&a.host, "files_remove", |_, home| json!({ "path": remote(home, &path), "transfer": detour_core::new_id() })).await?;
        Ok("Removed.".into())
    }

    #[tool(description = "Change a file's permissions on a host.")]
    async fn chmod(&self, Parameters(a): Parameters<Chmod>) -> Out {
        let mode = u32::from_str_radix(a.mode.trim_start_matches('0'), 8).map_err(|_| format!("{} is not an octal mode like 755.", a.mode))?;
        let path = a.path.clone();
        self.file_call(&a.host, "files_chmod", |_, home| json!({ "path": remote(home, &path), "mode": mode })).await?;
        Ok("Changed.".into())
    }

    /* ---------- tunnels and snippets ---------- */

    #[tool(description = "The person's saved tunnels and whether they run.")]
    async fn list_tunnels(&self) -> Out {
        let d = self.vault().await?;
        let status = self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "tunnels_status" })).await.map(|r| r.1).unwrap_or(json!([]));
        let list: Vec<Value> = d["tunnels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let state = status.as_array().into_iter().flatten().find(|x| x["key"] == format!("personal/{}", s(&t["id"]))).map(|x| x["state"].clone()).unwrap_or(json!("off"));
                let via = d["hosts"].as_array().into_iter().flatten().find(|h| h["id"] == t["viaHostId"]).map(|h| h["label"].clone()).unwrap_or_default();
                json!({ "name": t["name"], "kind": t["kind"], "port": t["bindPort"], "target": t["target"], "via": via, "state": state })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    #[tool(description = "Start a saved tunnel (needs the tunnels scope on its host).")]
    async fn start_tunnel(&self, Parameters(a): Parameters<NameArg>) -> Out {
        self.tunnel(&a.name, "tunnel_start").await?;
        Ok("Started.".into())
    }

    #[tool(description = "Stop a tunnel you started.")]
    async fn stop_tunnel(&self, Parameters(a): Parameters<NameArg>) -> Out {
        self.tunnel(&a.name, "tunnel_stop").await?;
        Ok("Stopped.".into())
    }

    async fn tunnel(&self, name: &str, method: &str) -> Result<Value, String> {
        let d = self.vault().await?;
        let t = d["tunnels"].as_array().into_iter().flatten().find(|t| s(&t["name"]).eq_ignore_ascii_case(name) || t["id"] == name).cloned().ok_or_else(|| format!("No tunnel called {name}."))?;
        let params = json!({ "vaultId": "personal", "tunnelId": t["id"] });
        Ok(self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": method, "params": params })).await?.1)
    }

    #[tool(description = "The person's saved snippets: named commands.")]
    async fn list_snippets(&self) -> Out {
        let d = self.vault().await?;
        let list: Vec<Value> = d["snippets"].as_array().into_iter().flatten().map(|x| json!({ "name": x["name"], "command": x["command"] })).collect();
        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    #[tool(description = "Run a saved snippet in a session, like run_command.")]
    async fn run_snippet(&self, Parameters(a): Parameters<RunSnippet>) -> Out {
        let d = self.vault().await?;
        let command = d["snippets"].as_array().into_iter().flatten().find(|x| s(&x["name"]).eq_ignore_ascii_case(&a.name)).map(|x| s(&x["command"])).ok_or_else(|| format!("No snippet called {}.", a.name))?;
        let (token, session) = self.session_for(a.session_id, a.host).await?;
        self.run(&token, &session, &command, None).await
    }

    /* ---------- the vault (vault:edit) ---------- */

    #[tool(description = "Add a host to the person's vault (needs vault:edit). To use it, request access to it.")]
    async fn add_host(&self, Parameters(a): Parameters<AddHost>) -> Out {
        let base = self.vault().await?;
        let mut next = base.clone();
        let auth = match &a.key {
            Some(k) => {
                let key = base["keys"].as_array().into_iter().flatten().find(|x| s(&x["name"]).eq_ignore_ascii_case(k) || x["id"] == k.as_str()).ok_or_else(|| format!("No key called {k}."))?;
                json!({ "type": "key", "keyId": key["id"] })
            }
            None => json!({ "type": "agent" }),
        };
        let host = json!({
            "id": detour_core::new_id(), "group": a.group.unwrap_or_default(), "label": a.name, "address": a.address, "port": a.port.unwrap_or(22),
            "user": a.user.unwrap_or_else(|| "root".into()), "auth": auth, "tags": [], "hasPassword": false, "forwardAgent": false,
        });
        next["hosts"].as_array_mut().ok_or("The vault has no host list.")?.push(host);
        self.put(&base, &next).await?;
        Ok(format!("Added {}.", a.name))
    }

    #[tool(description = "Change a host of the person's vault (needs vault:edit).")]
    async fn update_host(&self, Parameters(a): Parameters<UpdateHost>) -> Out {
        let base = self.vault().await?;
        let mut next = base.clone();
        let hosts = next["hosts"].as_array_mut().ok_or("The vault has no host list.")?;
        let h = hosts.iter_mut().find(|h| s(&h["label"]).eq_ignore_ascii_case(&a.host) || h["id"] == a.host.as_str()).ok_or_else(|| format!("No host called {} in the personal vault.", a.host))?;
        if let Some(v) = a.name {
            h["label"] = json!(v);
        }
        if let Some(v) = a.address {
            h["address"] = json!(v);
        }
        if let Some(v) = a.port {
            h["port"] = json!(v);
        }
        if let Some(v) = a.user {
            h["user"] = json!(v);
        }
        if let Some(v) = a.group {
            h["group"] = json!(v);
        }
        self.put(&base, &next).await?;
        Ok("Saved.".into())
    }

    #[tool(description = "Remove a host from the person's vault (needs vault:edit).")]
    async fn remove_host(&self, Parameters(a): Parameters<HostArg>) -> Out {
        let base = self.vault().await?;
        let mut next = base.clone();
        let hosts = next["hosts"].as_array_mut().ok_or("The vault has no host list.")?;
        let before = hosts.len();
        hosts.retain(|h| !(s(&h["label"]).eq_ignore_ascii_case(&a.host) || h["id"] == a.host.as_str()));
        if hosts.len() == before {
            return Err(format!("No host called {} in the personal vault.", a.host));
        }
        self.put(&base, &next).await?;
        Ok("Removed.".into())
    }

    #[tool(description = "Save a snippet: a named command (needs vault:edit).")]
    async fn add_snippet(&self, Parameters(a): Parameters<AddSnippet>) -> Out {
        let base = self.vault().await?;
        let mut next = base.clone();
        next["snippets"].as_array_mut().ok_or("The vault has no snippet list.")?.push(json!({ "id": detour_core::new_id(), "name": a.name, "command": a.command }));
        self.put(&base, &next).await?;
        Ok("Saved.".into())
    }

    #[tool(description = "Remove a snippet (needs vault:edit).")]
    async fn remove_snippet(&self, Parameters(a): Parameters<NameArg>) -> Out {
        let base = self.vault().await?;
        let mut next = base.clone();
        let list = next["snippets"].as_array_mut().ok_or("The vault has no snippet list.")?;
        let before = list.len();
        list.retain(|x| !s(&x["name"]).eq_ignore_ascii_case(&a.name));
        if list.len() == before {
            return Err(format!("No snippet called {}.", a.name));
        }
        self.put(&base, &next).await?;
        Ok("Removed.".into())
    }

    #[tool(description = "Create a key in the person's vault (needs vault:edit). Returns its public key, e.g. for authorized_keys; the private key never leaves Detour.")]
    async fn generate_key(&self, Parameters(a): Parameters<GenerateKey>) -> Out {
        let algorithm = a.algorithm.unwrap_or_else(|| "ed25519".into()).to_lowercase();
        let params = json!({ "vaultId": "personal", "name": a.name, "algorithm": algorithm, "passphrase": null, "remember": true });
        let (_, k) = self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "key_generate", "params": params })).await?;
        Ok(s(&k["publicKey"]))
    }

    #[tool(description = "Sync the person's vault with their other devices now (needs vault:edit).")]
    async fn sync_now(&self) -> Out {
        self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "sync_now", "params": {} })).await?;
        Ok("Synced.".into())
    }
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((p, _)) => p.into(),
        None => ".".into(),
    }
}

fn absolute(p: &str) -> String {
    let path = Path::new(p);
    if path.is_absolute() { p.to_string() } else { std::env::current_dir().map(|d| d.join(path).display().to_string()).unwrap_or_else(|_| p.to_string()) }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Detour {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("detour", detour_core::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serves MCP on stdin and stdout until the client goes away.
pub async fn serve(dir: &Path) -> std::io::Result<()> {
    let service = Detour::new(dir.to_path_buf()).serve(rmcp::transport::stdio()).await.map_err(std::io::Error::other)?;
    service.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_start_at_home_unless_absolute() {
        assert_eq!(remote("/home/u", "/etc/hosts"), "/etc/hosts");
        assert_eq!(remote("/home/u", "notes.txt"), "/home/u/notes.txt");
        assert_eq!(remote("/home/u/", "~/a/b"), "/home/u/a/b");
        assert_eq!(remote("/home/u", "~"), "/home/u");
        assert_eq!(parent("/etc/hosts"), "/etc");
        assert_eq!(parent("/hosts"), "/");
    }

    #[test]
    fn hosts_by_id_name_or_team_name() {
        let team = ("t1".to_string(), "h9".to_string(), "Acme/web".to_string());
        assert!(names(&team, "h9") && names(&team, "acme/WEB") && names(&team, "web"));
        assert!(!names(&team, "other/web") && !names(&team, "db"));
    }
}
