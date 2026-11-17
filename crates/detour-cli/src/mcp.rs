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
use std::time::{Duration as Span, Instant};

use detour_core::model::{Tunnel, TunnelKind};
use detour_ipc::Client;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::{Peer, RoleServer};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;

type Out = Result<String, String>;

/// How long a request for access waits for the person in one call (MCP clients time out).
const WAIT_SECS: u64 = 100;
/// How long the core keeps a request for the person (its `KEEP`): an answer given until then
/// still reaches this server, without the agent calling wait_for_access.
const REQUEST_KEEP: Span = Span::from_secs(10 * 60);
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
    /// Add and change hosts, snippets, tunnels and keys in the person's vault, and change a team's hosts.
    #[serde(rename = "vault:edit")]
    VaultEdit,
    /// Accept the key of a server no one has connected to yet (a changed key is never accepted).
    #[serde(rename = "hostkeys:new")]
    HostKeysNew,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::Connect => "connect",
            Scope::Exec => "exec",
            Scope::FilesRead => "files:read",
            Scope::FilesWrite => "files:write",
            Scope::Tunnels => "tunnels",
            Scope::VaultEdit => "vault:edit",
            Scope::HostKeysNew => "hostkeys:new",
        }
    }
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
    /// The team to add it to (its name); the personal vault if not given.
    team: Option<String>,
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
    /// Reach it through this saved proxy of the vault (its name).
    proxy: Option<String>,
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
    /// A saved proxy of the vault to go through (its name), or "none" for directly.
    proxy: Option<String>,
}

/// The id of a saved proxy by name, or null for "none".
fn proxy_id(vault: &Value, name: &str) -> std::result::Result<Value, String> {
    if name.eq_ignore_ascii_case("none") {
        return Ok(Value::Null);
    }
    vault["proxies"].as_array().into_iter().flatten().find(|p| s(&p["name"]).eq_ignore_ascii_case(name) || p["id"] == name).map(|p| p["id"].clone()).ok_or_else(|| format!("No proxy called {name}."))
}

#[derive(Deserialize, JsonSchema)]
struct AddSnippet {
    name: String,
    /// The command it runs.
    command: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Kind {
    /// `ssh -L`: a port on this computer leads to `target` as the host sees it.
    Local,
    /// `ssh -R`: a port on the host leads to `target` as this computer sees it.
    Remote,
    /// `ssh -D`: a SOCKS5 proxy on this computer that goes out through the host.
    Dynamic,
}

impl From<Kind> for TunnelKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Local => TunnelKind::Local,
            Kind::Remote => TunnelKind::Remote,
            Kind::Dynamic => TunnelKind::Dynamic,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct AddTunnel {
    name: String,
    kind: Kind,
    /// The host it goes through: a host of the person's vault, by name.
    via: String,
    /// The port it listens on: on this computer (local, dynamic) or on the host (remote). For local, default: the target's port.
    port: Option<u16>,
    /// host:port it leads to (local, remote). Not for dynamic.
    target: Option<String>,
    /// Start it right away (needs the tunnels scope on the host). Default false.
    start: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateTunnel {
    /// The tunnel to change.
    tunnel: String,
    name: Option<String>,
    /// Changing it to dynamic drops the target.
    kind: Option<Kind>,
    via: Option<String>,
    port: Option<u16>,
    target: Option<String>,
}

/// A host among agent_hosts by id, name or `team/name`.
fn find_host<'a>(all: &'a Value, host: &str) -> std::result::Result<&'a Value, String> {
    let n = host.to_lowercase();
    let found: Vec<&Value> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| s(&h["id"]) == host || s(&h["name"]).to_lowercase() == n || h["team"].as_str().is_some_and(|t| format!("{}/{}", t.to_lowercase(), s(&h["name"]).to_lowercase()) == n))
        .collect();
    match found.as_slice() {
        [h] => Ok(h),
        [] => Err(format!("No host called {host}.")),
        _ => Err(format!("Several hosts are called {host}: use team/name.")),
    }
}

/// A tunnel's name as list_tunnels shows it (`team/name` for a team's).
fn tunnel_label(t: &Value) -> String {
    match t["team"].as_str() {
        Some(team) => format!("{team}/{}", s(&t["tunnel"]["name"])),
        None => s(&t["tunnel"]["name"]),
    }
}

/// A tunnel among agent_tunnels by id, name or `team/name`.
fn find_tunnel<'a>(all: &'a [Value], name: &str) -> std::result::Result<&'a Value, String> {
    let n = name.to_lowercase();
    let mut found: Vec<&Value> = all.iter().filter(|t| t["tunnel"]["id"] == name || tunnel_label(t).to_lowercase() == n).collect();
    if found.is_empty() && !n.contains('/') {
        found = all.iter().filter(|t| s(&t["tunnel"]["name"]).to_lowercase() == n).collect();
    }
    match found.as_slice() {
        [t] => Ok(t),
        [] => Err(format!("No tunnel called {name} (list_tunnels shows them).")),
        _ => Err(format!("Several tunnels are called {name}: {}. Use one of these names.", found.iter().map(|t| tunnel_label(t)).collect::<Vec<_>>().join(", "))),
    }
}

/// The saved tunnels of one vault, as they are stored.
fn tunnels_in(all: &[Value], vault: &str) -> Vec<Value> {
    all.iter().filter(|t| t["vaultId"] == vault).map(|t| t["tunnel"].clone()).collect()
}

/// A tunnel's name, free among the other tunnels of its vault (they are found by name).
fn tunnel_name(tunnels: &[Value], name: &str, own_id: Option<&str>) -> std::result::Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') {
        return Err("Give the tunnel a name, without '/'.".into());
    }
    let taken = tunnels.iter().any(|t| s(&t["name"]).eq_ignore_ascii_case(name) && own_id.is_none_or(|id| t["id"] != id));
    if taken {
        return Err(format!("There is already a tunnel called {name} there."));
    }
    Ok(name.to_string())
}

/// A tunnel as the core will run it, or what is wrong with it.
fn checked(t: Tunnel) -> std::result::Result<Value, String> {
    match (t.kind, &t.target) {
        (TunnelKind::Dynamic, Some(_)) => return Err("A dynamic tunnel takes no target: it is a SOCKS5 proxy to anywhere.".into()),
        (TunnelKind::Dynamic, None) => {}
        (_, None) => return Err("This kind of tunnel needs a target: host:port.".into()),
        (_, Some(x)) => {
            detour_core::tunnel::parse_target(x).map_err(|_| format!("{x} is not host:port."))?;
        }
    }
    if t.bind_port == 0 {
        return Err(match t.kind {
            TunnelKind::Remote => "Give port: the port to open on the host.",
            TunnelKind::Dynamic => "Give port: the port of the SOCKS5 proxy on this computer (1080 is usual).",
            TunnelKind::Local => "Give port: the port to open on this computer.",
        }
        .into());
    }
    serde_json::to_value(t).map_err(|e| e.to_string())
}

/// Where a running tunnel is reached, in a sentence.
fn tunnel_reach(t: &Value, via: &str) -> String {
    let (port, target) = (s(&t["bindPort"]), s(&t["target"]));
    match t["kind"].as_str() {
        Some("remote") => format!("port {port} on {via} leads to {target} on this computer"),
        Some("dynamic") => format!("a SOCKS5 proxy at localhost:{port} goes out through {via}"),
        _ => format!("localhost:{port} leads to {target} through {via}"),
    }
}

fn some_text(v: Option<String>) -> Option<String> {
    v.map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

#[derive(Deserialize, JsonSchema)]
struct GenerateKey {
    name: String,
    /// ed25519 (default), rsa or ecdsa.
    algorithm: Option<String>,
}

/// A file browser's id and the home directory it opened in.
type Browser = (String, String);

/// A request for access still waiting for the person. Its answer is collected in the
/// background, once, and every wait for it gets the same.
#[derive(Clone)]
struct Pending {
    answer: watch::Receiver<Option<Out>>,
    /// What a wait says while there is no answer yet.
    waiting: String,
    asked: Instant,
}

impl Pending {
    fn live(&self) -> bool {
        self.asked.elapsed() < REQUEST_KEEP
    }

    /// The answer, or what to say while there is none after one call's wait.
    async fn wait(mut self) -> Out {
        match tokio::time::timeout(Span::from_secs(WAIT_SECS), self.answer.wait_for(Option::is_some)).await {
            Ok(Ok(answer)) => answer.clone().unwrap_or(Ok(self.waiting)),
            Ok(Err(_)) => Err("Lost the connection to Detour. Try again.".into()),
            Err(_) => Ok(self.waiting),
        }
    }
}

/// What an agent's token reaches, as the core says.
#[derive(Clone)]
struct Grant {
    secret: String,
    hosts: Vec<(String, String, String)>,
    scopes: Vec<String>,
}

impl Grant {
    fn new(secret: String) -> Self {
        Self { secret, hosts: Vec::new(), scopes: Vec::new() }
    }

    /// Allows this on the host (`exec` covers `connect`).
    fn allows(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope) || scope == "connect" && self.scopes.iter().any(|s| s == "exec")
    }
}

#[derive(Clone)]
pub struct Detour {
    dir: PathBuf,
    link: Arc<tokio::sync::Mutex<Option<Client>>>,
    grants: Arc<Mutex<Vec<Grant>>>,
    /// Requests for access still waiting for the person, by their id.
    pending: Arc<Mutex<HashMap<String, Pending>>>,
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
        "hostKeyUnknown" => "Detour has not confirmed this server's key. Ask the person to connect to the host once in Detour and confirm it, or to let you accept new servers' keys (the hostkeys:new scope, which you may request).".into(),
        "hostKeyChanged" => "This server's key is not the one Detour knows: it may be another server. Only the person can accept it, in Detour.".into(),
        "needSecret" | "authFailed" | "jumpNeedsLogin" => "The host's login needs a password or passphrase Detour has not saved. Ask the person to save it for this host in Detour.".into(),
        "ambiguousHost" => format!("Several hosts are called that: {}. Use one of these names.", e["options"].as_array().map(|o| o.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
        "notFound" => format!("Not found: {}.", e["what"].as_str().unwrap_or("")),
        "unreachable" => format!("{} is not responding.", e["endpoint"].as_str().unwrap_or("The host")),
        "forbidden" => "Agents can't do that through Detour.".into(),
        "tooManyRequests" => "Too many requests are waiting for the person. Wait for an answer to those first.".into(),
        "disconnected" => "Lost the connection to Detour. Try again.".into(),
        // An older core refuses what it doesn't know as if we weren't trusted.
        "oldCore" | "notTrusted" | "unknownMethod" => "The Detour core running now is an older version, without support for agents. Ask the person to restart it: \
            `detour quit` in their terminal (or quit Detour), then try again."
            .into(),
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
        let grants = preset.split([',', ' ']).filter(|t| !t.is_empty()).map(|t| Grant::new(t.to_string())).collect();
        Self { dir, link: Arc::default(), grants: Arc::new(Mutex::new(grants)), pending: Arc::default(), files: Arc::default(), tool_router: Self::tool_router() }
    }

    /// A call to the core, starting it (or reconnecting) when needed.
    async fn core(&self, method: &str, params: Value) -> Result<Value, Value> {
        let mut link = self.link.lock().await;
        if link.as_ref().is_none_or(Client::is_closed) {
            let me = std::env::current_exe().map_err(|e| json!({ "message": e.to_string() }))?;
            let c = detour_ipc::connect_or_start(&me, &self.dir).await.map_err(|e| json!({ "kind": "disconnected", "message": e.to_string() }))?;
            let hello = c.call("hello", json!({ "client": "mcp" })).await?;
            if hello["protocol"].as_u64().unwrap_or(0) < detour_ipc::PROTOCOL {
                return Err(json!({ "kind": "oldCore" }));
            }
            *link = Some(c);
        }
        let c = link.as_ref().unwrap().clone();
        drop(link);
        c.call(method, params).await
    }

    /// The tokens this server holds, with what each reaches. Asked of the core every time:
    /// hosts are found by name, and a host renamed (or another given its old name) must not
    /// lead to the host the name used to mean.
    async fn grants(&self) -> Vec<Grant> {
        let all = self.grants.lock().unwrap().clone();
        let mut out = Vec::new();
        for mut g in all {
            match self.core("agent_whoami", json!({ "token": g.secret })).await {
                Ok(who) => {
                    g.hosts = who["hosts"].as_array().into_iter().flatten().map(|h| (s(&h["vaultId"]), s(&h["hostId"]), host_label(h))).collect();
                    g.scopes = who["scopes"].as_array().into_iter().flatten().map(s).collect();
                    let mut list = self.grants.lock().unwrap();
                    if let Some(x) = list.iter_mut().find(|x| x.secret == g.secret) {
                        (x.hosts, x.scopes) = (g.hosts.clone(), g.scopes.clone());
                    }
                }
                Err(e) if e["kind"] == "tokenInvalid" => {
                    self.grants.lock().unwrap().retain(|x| x.secret != g.secret);
                    continue;
                }
                // The core is away for a moment: what was known last.
                Err(_) => {}
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
                    let worth = !matches!(e["kind"].as_str(), Some("agentScope" | "agentHost" | "tokenInvalid" | "notYourSession" | "notYourFiles" | "notYourTunnel"));
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

    /// The vault a host to change is in, and that vault's hosts: a host by name, `team/name` or id.
    async fn hosts_of(&self, host: &str) -> Result<(String, Value), String> {
        let all = self.core("agent_hosts", json!({})).await.map_err(|e| explain(&e))?;
        let vault = s(&find_host(&all, host)?["vaultId"]);
        Ok((vault.clone(), self.vault_hosts(&vault).await?))
    }

    async fn vault_hosts(&self, vault: &str) -> Result<Value, String> {
        Ok(self.with_tokens(|_| true, "agent_vault_hosts", |token| json!({ "token": token, "vaultId": vault })).await?.1)
    }

    /// Saves changed hosts of a vault: the personal one, or a team (only the hosts change).
    async fn put_hosts(&self, vault: &str, base: Value, next: Value) -> Result<(), String> {
        if vault == detour_core::store::PERSONAL {
            let all = self.vault().await?;
            let (mut b, mut n) = (all.clone(), all);
            (b["hosts"], n["hosts"]) = (base, next);
            return self.put(&b, &n).await;
        }
        let with = |hosts: Value| json!({ "hosts": hosts, "keys": [], "snippets": [], "tunnels": [] });
        self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "team_put", "params": { "id": vault, "update": with(next.clone()), "base": with(base.clone()) } })).await?;
        Ok(())
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
        allow less than you asked. Waits up to 100 seconds for the answer; if it is still pending, ask the person to approve it: \
        their answer takes effect by itself, and wait_for_access waits for it.")]
    async fn request_access(&self, Parameters(a): Parameters<RequestAccess>, peer: Peer<RoleServer>) -> Out {
        // What the person gave ahead (or before) already covers it: no need to ask again.
        let grants = self.grants().await;
        let covered = |name: &str| grants.iter().any(|g| g.hosts.iter().any(|h| names(h, name)) && a.scopes.iter().all(|sc| g.allows(sc.name())));
        if !a.hosts.is_empty() && a.hosts.iter().all(|h| covered(h)) {
            let scopes: Vec<&str> = a.scopes.iter().map(|sc| sc.name()).collect();
            return Ok(format!("Already allowed: {} on {}. Open a session with open_session.", scopes.join(", "), a.hosts.join(", ")));
        }
        let agent = peer.peer_info().map(|i| i.client_info.title.clone().unwrap_or_else(|| i.client_info.name.clone())).filter(|n| !n.is_empty()).unwrap_or_else(|| "AI agent".into());
        let r = self
            .core("agent_request", json!({ "agent": agent, "hosts": a.hosts, "scopes": a.scopes, "ttl": a.duration, "reason": a.reason, "waitSecs": 0 }))
            .await
            .map_err(|e| explain(&e))?;
        self.access_response(r).await
    }

    #[tool(description = "Wait for the person's answer to a request_access that is still pending (up to 100 seconds).")]
    async fn wait_for_access(&self, Parameters(a): Parameters<RequestId>) -> Out {
        let pending = self.pending.lock().unwrap().get(&a.request_id).filter(|p| p.live()).cloned();
        if let Some(p) = pending {
            return p.wait().await;
        }
        let r = self.core("agent_wait", json!({ "requestId": a.request_id, "waitSecs": 0 })).await.map_err(|e| explain(&e))?;
        self.access_response(r).await
    }

    /// The core's answer to a request; one still pending is waited for (up to one call's wait)
    /// and collected in the background after that, so a late "Allow" works at once.
    async fn access_response(&self, r: Value) -> Out {
        if r["status"] != "pending" {
            return self.answered(r).await;
        }
        let id = s(&r["requestId"]);
        let waiting = self.answered(r).await?;
        let p = {
            let mut pending = self.pending.lock().unwrap();
            pending.retain(|_, p| p.live());
            pending.entry(id.clone()).or_insert_with(|| self.collect(id, waiting)).clone()
        };
        p.wait().await
    }

    /// Waits for the person's answer to a request until the core drops it, and takes it in.
    fn collect(&self, id: String, waiting: String) -> Pending {
        let (tx, rx) = watch::channel(None);
        let this = self.clone();
        tokio::spawn(async move {
            let answer = async {
                loop {
                    match this.core("agent_wait", json!({ "requestId": id, "waitSecs": WAIT_SECS })).await {
                        Ok(r) if r["status"] == "pending" => continue,
                        Ok(r) => return this.answered(r).await,
                        Err(e) => {
                            this.pending.lock().unwrap().remove(&id);
                            return Err(explain(&e));
                        }
                    }
                }
            };
            let answer = tokio::time::timeout(REQUEST_KEEP, answer).await.unwrap_or_else(|_| Err("The request for access expired. Call request_access again.".into()));
            tx.send_replace(Some(answer));
        });
        Pending { answer: rx, waiting, asked: Instant::now() }
    }

    async fn answered(&self, r: Value) -> Out {
        match r["status"].as_str() {
            Some("approved") => {
                let secret = s(&r["token"]);
                self.grants.lock().unwrap().push(Grant::new(secret.clone()));
                let who = self.core("agent_whoami", json!({ "token": secret })).await.map_err(|e| explain(&e))?;
                let hosts: Vec<String> = who["hosts"].as_array().into_iter().flatten().map(host_label).collect();
                let scopes: Vec<String> = who["scopes"].as_array().into_iter().flatten().map(s).collect();
                Ok(format!("Allowed: {} on {}. Open a session with open_session.", scopes.join(", "), hosts.join(", ")))
            }
            Some("denied") => Err("The person denied the request.".into()),
            _ => {
                let how = if r["window"] == true { "in the Detour window" } else { "by running `detour approve` in their terminal (Detour's window is not open)" };
                Ok(format!(
                    "Still waiting for the person. Ask them to approve it {how}. Once they do, the access works at once (list_hosts shows it); to wait for the answer, call wait_for_access with request_id \"{}\".",
                    s(&r["requestId"])
                ))
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

    /// The person's tunnels and their teams' (see `agent_tunnels`).
    async fn all_tunnels(&self) -> Result<Vec<Value>, String> {
        let r = self.core("agent_tunnels", json!({})).await.map_err(|e| explain(&e))?;
        Ok(r.as_array().cloned().unwrap_or_default())
    }

    /// The tunnels running now (empty without the tunnels scope).
    async fn tunnel_states(&self) -> Value {
        self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "tunnels_status" })).await.map(|r| r.1).unwrap_or(json!([]))
    }

    async fn tunnel_on(&self, vault: &str, id: &str) -> bool {
        self.tunnel_states().await.as_array().into_iter().flatten().any(|x| x["key"] == format!("{vault}/{id}") && x["state"] == "on")
    }

    /// Starts or stops a tunnel of a vault.
    async fn tunnel_call(&self, vault: &str, id: &str, method: &str) -> Result<Value, String> {
        let params = json!({ "vaultId": vault, "tunnelId": id });
        Ok(self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": method, "params": params })).await?.1)
    }

    async fn tunnel(&self, name: &str, method: &str) -> Result<Value, String> {
        let all = self.all_tunnels().await?;
        let t = find_tunnel(&all, name)?;
        self.tunnel_call(&s(&t["vaultId"]), &s(&t["tunnel"]["id"]), method).await
    }

    /// Saves changed tunnels of a vault: the personal one, or a team (only the tunnels change).
    async fn put_tunnels(&self, vault: &str, base: Vec<Value>, next: Vec<Value>) -> Result<(), String> {
        if vault == detour_core::store::PERSONAL {
            let all = self.vault().await?;
            let (mut b, mut n) = (all.clone(), all);
            (b["tunnels"], n["tunnels"]) = (json!(base), json!(next));
            return self.put(&b, &n).await;
        }
        let with = |tunnels: &Vec<Value>| json!({ "hosts": [], "keys": [], "snippets": [], "tunnels": tunnels });
        self.with_tokens(|_| true, "agent_call", |token| json!({ "token": token, "method": "team_put", "params": { "id": vault, "update": with(&next), "base": with(&base) } })).await?;
        Ok(())
    }

    #[tool(description = "The person's saved tunnels and their teams', and whether they run.")]
    async fn list_tunnels(&self) -> Out {
        let all = self.all_tunnels().await?;
        let status = self.tunnel_states().await;
        let list: Vec<Value> = all
            .iter()
            .map(|t| {
                let x = &t["tunnel"];
                let key = format!("{}/{}", s(&t["vaultId"]), s(&x["id"]));
                let state = status.as_array().into_iter().flatten().find(|st| st["key"] == key).map(|st| st["state"].clone()).unwrap_or(json!("off"));
                let via = match t["team"].as_str() {
                    Some(team) => format!("{team}/{}", s(&t["via"])),
                    None => s(&t["via"]),
                };
                json!({ "name": tunnel_label(t), "kind": x["kind"], "port": x["bindPort"], "target": x["target"], "via": via, "state": state })
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

    #[tool(description = "Save a tunnel through a host (needs vault:edit; for a team's host, one your access reaches): \
        local (ssh -L), remote (ssh -R) or dynamic (SOCKS5, ssh -D).")]
    async fn add_tunnel(&self, Parameters(a): Parameters<AddTunnel>) -> Out {
        let hosts = self.core("agent_hosts", json!({})).await.map_err(|e| explain(&e))?;
        let via = find_host(&hosts, &a.via)?;
        let vault = s(&via["vaultId"]);
        let all = self.all_tunnels().await?;
        let base = tunnels_in(&all, &vault);
        let name = tunnel_name(&base, &a.name, None)?;
        let target = some_text(a.target);
        let kind = TunnelKind::from(a.kind);
        let from_target = || target.as_deref().and_then(|x| detour_core::tunnel::parse_target(x).ok()).map(|x| x.1);
        let port = a.port.or_else(|| if kind == TunnelKind::Local { from_target() } else { None }).unwrap_or(0);
        let t = Tunnel { id: detour_core::new_id(), name: name.clone(), kind, bind_port: port, via_host_id: s(&via["id"]), target };
        let id = t.id.clone();
        let t = checked(t)?;
        let reach = tunnel_reach(&t, &a.via);
        let mut next = base.clone();
        next.push(t);
        self.put_tunnels(&vault, base, next).await?;
        let label = match via["team"].as_str() {
            Some(team) => format!("{team}/{name}"),
            None => name,
        };
        if a.start != Some(true) {
            return Ok(format!("Added {label}. Start it with start_tunnel."));
        }
        match self.tunnel_call(&vault, &id, "tunnel_start").await {
            Ok(_) => Ok(format!("Added and started {label}: {reach}.")),
            Err(e) => Err(format!("Added {label}, but it did not start: {e}")),
        }
    }

    #[tool(description = "Change a saved tunnel (needs vault:edit; for a team's, through a host your access reaches).")]
    async fn update_tunnel(&self, Parameters(a): Parameters<UpdateTunnel>) -> Out {
        let all = self.all_tunnels().await?;
        let found = find_tunnel(&all, &a.tunnel)?;
        let vault = s(&found["vaultId"]);
        let base = tunnels_in(&all, &vault);
        let mut t: Tunnel = serde_json::from_value(found["tunnel"].clone()).map_err(|e| e.to_string())?;
        if let Some(n) = &a.name {
            t.name = tunnel_name(&base, n, Some(&t.id))?;
        }
        if let Some(k) = a.kind {
            t.kind = k.into();
            if t.kind == TunnelKind::Dynamic && a.target.is_none() {
                t.target = None;
            }
        }
        if let Some(v) = &a.via {
            let hosts = self.core("agent_hosts", json!({})).await.map_err(|e| explain(&e))?;
            let h = find_host(&hosts, v)?;
            if h["vaultId"] != vault.as_str() {
                return Err(format!("{v} is not where {} is: a tunnel goes through a host of its own vault (or team).", a.tunnel));
            }
            t.via_host_id = s(&h["id"]);
        }
        if let Some(p) = a.port {
            t.bind_port = p;
        }
        if a.target.is_some() {
            t.target = some_text(a.target);
        }
        let id = t.id.clone();
        let t = checked(t)?;
        let next: Vec<Value> = base.iter().map(|x| if x["id"] == id.as_str() { t.clone() } else { x.clone() }).collect();
        self.put_tunnels(&vault, base, next).await?;
        if self.tunnel_on(&vault, &id).await {
            return Ok("Saved. It is running with the old settings until it is started again (stop_tunnel, then start_tunnel).".into());
        }
        Ok("Saved.".into())
    }

    #[tool(description = "Remove a saved tunnel (needs vault:edit; for a team's, through a host your access reaches). A tunnel you started is stopped first.")]
    async fn remove_tunnel(&self, Parameters(a): Parameters<NameArg>) -> Out {
        let all = self.all_tunnels().await?;
        let found = find_tunnel(&all, &a.name)?;
        let (vault, id) = (s(&found["vaultId"]), s(&found["tunnel"]["id"]));
        if self.tunnel_on(&vault, &id).await && self.tunnel_call(&vault, &id, "tunnel_stop").await.is_err() {
            return Err(format!("{} is running and you didn't start it: ask the person to stop it first.", tunnel_label(found)));
        }
        let base = tunnels_in(&all, &vault);
        let next = base.iter().filter(|x| x["id"] != id.as_str()).cloned().collect();
        self.put_tunnels(&vault, base, next).await?;
        Ok("Removed.".into())
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

    #[tool(description = "Add a host to the person's vault, or to a team (needs vault:edit; in a team, over all its hosts). To use it, request access to it.")]
    async fn add_host(&self, Parameters(a): Parameters<AddHost>) -> Out {
        let personal = self.vault().await?;
        let vault = match &a.team {
            Some(team) => {
                let all = self.core("agent_hosts", json!({})).await.map_err(|e| explain(&e))?;
                all.as_array().into_iter().flatten().find(|h| h["team"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(team))).map(|h| s(&h["vaultId"])).ok_or_else(|| format!("No team called {team} (or it has no hosts yet)."))?
            }
            None => detour_core::store::PERSONAL.to_string(),
        };
        // Keys and proxies are the vault's own: a personal key can't log in to a team's host.
        let source = if vault == detour_core::store::PERSONAL { personal.clone() } else { json!({}) };
        let auth = match &a.key {
            Some(k) => {
                let key = source["keys"].as_array().into_iter().flatten().find(|x| s(&x["name"]).eq_ignore_ascii_case(k) || x["id"] == k.as_str()).ok_or_else(|| format!("No key called {k}."))?;
                json!({ "type": "key", "keyId": key["id"] })
            }
            None => json!({ "type": "agent" }),
        };
        let proxy = match &a.proxy {
            Some(p) => proxy_id(&source, p)?,
            None => Value::Null,
        };
        let host = json!({
            "id": detour_core::new_id(), "group": a.group.unwrap_or_default(), "label": a.name, "address": a.address, "port": a.port.unwrap_or(22),
            "user": a.user.unwrap_or_else(|| "root".into()), "auth": auth, "tags": [], "hasPassword": false, "forwardAgent": false, "proxyId": proxy,
        });
        let base = self.vault_hosts(&vault).await?;
        let mut next = base.clone();
        next.as_array_mut().ok_or("The vault has no host list.")?.push(host);
        self.put_hosts(&vault, base, next).await?;
        Ok(format!("Added {}.", a.name))
    }

    #[tool(description = "Change a host of the person's vault or of a team (needs vault:edit; in a team, a host your access reaches).")]
    async fn update_host(&self, Parameters(a): Parameters<UpdateHost>) -> Out {
        let (vault, base) = self.hosts_of(&a.host).await?;
        let mut next = base.clone();
        let wanted = a.host.rsplit('/').next().unwrap_or(&a.host).to_lowercase();
        let h = next.as_array_mut().into_iter().flatten().find(|h| s(&h["label"]).to_lowercase() == wanted || h["id"] == a.host.as_str()).ok_or_else(|| format!("No host called {}.", a.host))?;
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
        if let Some(p) = &a.proxy {
            let source = if vault == detour_core::store::PERSONAL { self.vault().await? } else { json!({}) };
            h["proxyId"] = if p.eq_ignore_ascii_case("none") { Value::Null } else { proxy_id(&source, p)? };
            h["proxy"] = Value::Null;
        }
        self.put_hosts(&vault, base, next).await?;
        Ok("Saved.".into())
    }

    #[tool(description = "Remove a host from the person's vault or from a team (needs vault:edit; in a team, a host your access reaches).")]
    async fn remove_host(&self, Parameters(a): Parameters<HostArg>) -> Out {
        let (vault, base) = self.hosts_of(&a.host).await?;
        let mut next = base.clone();
        let wanted = a.host.rsplit('/').next().unwrap_or(&a.host).to_lowercase();
        let hosts = next.as_array_mut().ok_or("The vault has no host list.")?;
        let before = hosts.len();
        hosts.retain(|h| !(s(&h["label"]).to_lowercase() == wanted || h["id"] == a.host.as_str()));
        if hosts.len() == before {
            return Err(format!("No host called {}.", a.host));
        }
        self.put_hosts(&vault, base, next).await?;
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

/* ---------- adding the server to AI clients (`detour mcp add`) ---------- */

/// `detour mcp clients | add | remove`: this `detour` in the configs of the AI clients here.
pub fn setup(action: crate::McpAction, dir: &Path, json: bool) -> crate::conn::Res<()> {
    use crate::conn::Fail;
    use crate::i18n::{error_text, t};
    use detour_core::mcp_clients::{self, Dirs, Server};

    let dirs = Dirs::of_user().ok_or_else(|| Fail("home directory".into()))?;
    let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
    // A vault folder of its own goes with it: the client won't have $DETOUR_DATA_DIR.
    let own_dir = (dir != detour_core::store::Store::default_dir()).then_some(dir);
    let server = Server::new(me, own_dir);
    let fail = |e: detour_core::Error| Fail(error_text(&serde_json::to_value(&e).unwrap_or_default()));
    let all = mcp_clients::clients(&dirs, &server);
    let find = |id: &str| all.iter().find(|c| c.id.eq_ignore_ascii_case(id) || c.name.eq_ignore_ascii_case(id)).ok_or_else(|| Fail(format!("{}: {id} ({})", t().unknown_client, all.iter().map(|c| c.id).collect::<Vec<_>>().join(", "))));
    match action {
        crate::McpAction::Clients => {
            if json {
                println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
                return Ok(());
            }
            let state = |c: &mcp_clients::ClientState| match (c.installed, c.added, c.current) {
                (_, true, true) => t().mcp_added,
                (_, true, false) => t().mcp_outdated,
                (true, false, _) => t().mcp_not_added,
                (false, false, _) => t().mcp_not_installed,
            };
            let rows = all.iter().map(|c| vec![c.id.to_string(), c.name.to_string(), state(c).to_string(), if c.installed { c.config.clone() } else { String::new() }]).collect();
            crate::cmds::table(&t().headers_mcp, rows);
        }
        crate::McpAction::Add { client } => {
            let picked: Vec<_> = match client {
                Some(id) => vec![find(&id)?.clone()],
                // All the clients here that don't have it yet.
                None => all.iter().filter(|c| c.installed && !c.current).cloned().collect(),
            };
            if picked.is_empty() {
                eprintln!("{}", if all.iter().any(|c| c.installed) { t().mcp_all_added } else { t().mcp_no_clients });
                return Ok(());
            }
            for c in picked {
                mcp_clients::add(&dirs, c.id, &server, true).map_err(fail)?;
                eprintln!("{} {}", t().mcp_added_to, c.name);
            }
            eprintln!("{}", t().mcp_restart);
        }
        crate::McpAction::Remove { client } => {
            let c = find(&client)?;
            let removed = mcp_clients::remove(&dirs, c.id, true).map_err(fail)?;
            eprintln!("{} {}", if removed { t().mcp_removed_from } else { t().mcp_was_not_in }, c.name);
        }
    }
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

    /// A core of its own with two hosts, the person's connection to it and an MCP server.
    struct Core {
        dir: PathBuf,
        socket: String,
        me: Client,
        agent: Detour,
        daemon: tokio::task::JoinHandle<std::io::Result<()>>,
    }

    impl Core {
        async fn start() -> Self {
            let dir = std::env::temp_dir().join(format!("detour-mcp-{}", detour_core::new_id()));
            std::fs::create_dir_all(&dir).unwrap();
            let socket = detour_ipc::socket_name(&dir);
            let daemon = tokio::spawn({
                let dir = dir.clone();
                async move { detour_daemon::run(&dir, None).await }
            });
            let me = tokio::time::timeout(Span::from_secs(5), async {
                loop {
                    if let Ok(c) = Client::connect(&socket).await {
                        break c;
                    }
                    tokio::time::sleep(Span::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            me.call("hello", json!({ "client": "cli" })).await.unwrap();
            me.call("vault_create", json!({ "masterPassword": "test-only" })).await.unwrap();
            let host = |name: &str| json!({ "id": name, "label": name, "group": "", "address": "127.0.0.1", "port": 9, "user": "test", "auth": { "type": "agent" }, "tags": [], "hasPassword": false });
            me.call("vault_put", json!({ "update": { "hosts": [host("web"), host("db")], "keys": [], "snippets": [], "tunnels": [] } })).await.unwrap();
            let agent = Self::agent(&socket).await;
            Self { dir, socket, me, agent, daemon }
        }

        /// An MCP server on this core, holding no tokens.
        async fn agent(socket: &str) -> Detour {
            let c = Client::connect(socket).await.unwrap();
            c.call("hello", json!({ "client": "mcp" })).await.unwrap();
            let agent = Detour::new(PathBuf::new());
            agent.grants.lock().unwrap().clear();
            *agent.link.lock().await = Some(c);
            agent
        }

        /// Asks for `web` without waiting: the request stays pending.
        async fn request(&self) -> String {
            let r = self.agent.core("agent_request", json!({ "agent": "Test", "hosts": ["web"], "scopes": ["connect"], "waitSecs": 0 })).await.unwrap();
            assert_eq!(r["status"], "pending");
            s(&r["requestId"])
        }

        /// The person answers, allowing every right on all hosts.
        async fn answer(&self, id: &str, allow: bool) -> Value {
            let scopes = ["connect", "exec", "files:read", "files:write", "tunnels", "vault:edit", "hostkeys:new"];
            self.me.call("agents_answer", json!({ "id": id, "allow": allow, "ttl": "never", "hosts": [], "reach": [{ "type": "all" }], "scopes": scopes })).await.unwrap()
        }

        async fn hosts(agent: &Detour) -> Vec<Value> {
            serde_json::from_str(&agent.list_hosts().await.unwrap()).unwrap()
        }

        /// Starts a wait for the request and leaves it after a moment, as an MCP client that times out.
        async fn wait_a_moment(&self, id: &str) {
            let wait = self.agent.wait_for_access(Parameters(RequestId { request_id: id.into() }));
            assert!(tokio::time::timeout(Span::from_millis(50), wait).await.is_err());
        }

        async fn wait(&self, id: &str) -> Out {
            self.agent.wait_for_access(Parameters(RequestId { request_id: id.into() })).await
        }
    }

    impl Drop for Core {
        fn drop(&mut self) {
            self.daemon.abort();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn an_approval_given_after_the_wait_reaches_the_agent_by_itself() {
        let core = Core::start().await;
        let id = core.request().await;
        core.wait_a_moment(&id).await;
        let given = core.answer(&id, true).await;

        // No wait_for_access: the next tool already has the access.
        let hosts = tokio::time::timeout(Span::from_secs(2), async {
            loop {
                let hosts = Core::hosts(&core.agent).await;
                if hosts.iter().all(|h| h["access"].as_array().unwrap().len() == 7) {
                    break hosts;
                }
                tokio::time::sleep(Span::from_millis(10)).await;
            }
        })
        .await
        .expect("the approved access never reached the MCP server");
        assert_eq!(hosts.len(), 2);
        assert_eq!(core.agent.grants.lock().unwrap().len(), 1);

        // Another MCP server on the same core gets nothing of it.
        let other = Core::agent(&core.socket).await;
        assert!(Core::hosts(&other).await.iter().all(|h| h["access"] == json!([])));

        // Waits after the answer (two at once) say the same and take the token in only once.
        let (a, b) = tokio::join!(core.wait(&id), core.wait(&id));
        assert_eq!(a, b);
        assert!(a.unwrap().starts_with("Allowed:"));
        assert_eq!(core.agent.grants.lock().unwrap().len(), 1);

        core.me.call("agents_revoke", json!({ "id": given["tokenId"] })).await.unwrap();
        assert!(Core::hosts(&core.agent).await.iter().all(|h| h["access"] == json!([])));
    }

    #[tokio::test]
    async fn a_denial_given_after_the_wait_reaches_every_wait_and_gives_nothing() {
        let core = Core::start().await;
        let id = core.request().await;
        core.wait_a_moment(&id).await;
        core.answer(&id, false).await;
        let (a, b) = tokio::join!(core.wait(&id), core.wait(&id));
        assert_eq!(a, Err("The person denied the request.".into()));
        assert_eq!(a, b);
        assert!(core.agent.grants.lock().unwrap().is_empty());
        assert!(Core::hosts(&core.agent).await.iter().all(|h| h["access"] == json!([])));
    }

    #[tokio::test]
    async fn waiting_for_a_request_that_does_not_exist_is_an_error() {
        let core = Core::start().await;
        assert_eq!(core.wait("missing").await, Err("Not found: request.".into()));
        assert!(core.agent.grants.lock().unwrap().is_empty());
    }

    /// An SSH server that lets `u` in with `pw` and does nothing else: enough for a tunnel to start.
    #[derive(Clone)]
    struct Sshd;

    impl russh::server::Server for Sshd {
        type Handler = Self;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
            self.clone()
        }
    }

    impl russh::server::Handler for Sshd {
        type Error = russh::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<russh::server::Auth, Self::Error> {
            Ok(if user == "u" && password == "pw" { russh::server::Auth::Accept } else { russh::server::Auth::reject() })
        }
    }

    async fn sshd() -> u16 {
        use russh::server::Server as _;
        let config = Arc::new(russh::server::Config {
            keys: vec![russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap()],
            auth_rejection_time: Span::from_millis(10),
            auth_rejection_time_initial: Some(Span::from_millis(0)),
            ..Default::default()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = Sshd.run_on_socket(config, &listener).await;
        });
        port
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    impl Core {
        /// The agent with every right on all hosts, as the person allowed it.
        async fn granted(&self) {
            let id = self.request().await;
            self.answer(&id, true).await;
            assert!(self.wait(&id).await.unwrap().starts_with("Allowed:"));
        }

        async fn tunnels(&self) -> Vec<Value> {
            serde_json::from_str(&self.agent.list_tunnels().await.unwrap()).unwrap()
        }

        async fn add(&self, name: &str, kind: Kind, via: &str, port: Option<u16>, target: Option<&str>) -> Out {
            let a = AddTunnel { name: name.into(), kind, via: via.into(), port, target: target.map(Into::into), start: None };
            self.agent.add_tunnel(Parameters(a)).await
        }
    }

    fn update(tunnel: &str) -> UpdateTunnel {
        UpdateTunnel { tunnel: tunnel.into(), name: None, kind: None, via: None, port: None, target: None }
    }

    #[tokio::test]
    async fn an_agent_saves_starts_lists_and_removes_a_tunnel() {
        let core = Core::start().await;
        // `jump` is a real server: a tunnel through it starts.
        let mut jump = json!({ "id": "jump", "label": "jump", "group": "", "address": "127.0.0.1", "port": sshd().await, "user": "u", "auth": { "type": "password" }, "tags": [], "hasPassword": false, "newPassword": "pw" });
        let base = core.me.call("vault_get", json!({})).await.unwrap();
        let mut hosts = base["hosts"].as_array().unwrap().clone();
        hosts.push(jump.take());
        core.me.call("vault_put", json!({ "update": { "hosts": hosts, "keys": [], "snippets": [], "tunnels": [] } })).await.unwrap();
        core.granted().await;

        let port = free_port();
        let a = AddTunnel { name: "proxy".into(), kind: Kind::Dynamic, via: "jump".into(), port: Some(port), target: None, start: Some(true) };
        let added = core.agent.add_tunnel(Parameters(a)).await.unwrap();
        assert_eq!(added, format!("Added and started proxy: a SOCKS5 proxy at localhost:{port} goes out through jump."));
        tokio::net::TcpStream::connect(("127.0.0.1", port)).await.expect("the proxy listens");

        let list = core.tunnels().await;
        assert_eq!(list, vec![json!({ "name": "proxy", "kind": "dynamic", "port": port, "target": null, "via": "jump", "state": "on" })]);
        // The person sees it in their vault.
        assert_eq!(core.me.call("vault_get", json!({})).await.unwrap()["tunnels"][0]["name"], "proxy");

        // Changed while it runs: saved, and the agent is told it runs as it was.
        let mut u = update("proxy");
        u.name = Some("socks".into());
        assert!(core.agent.update_tunnel(Parameters(u)).await.unwrap().starts_with("Saved. It is running"));

        // Removed: stopped first, gone from the vault.
        assert_eq!(core.agent.remove_tunnel(Parameters(NameArg { name: "socks".into() })).await.unwrap(), "Removed.");
        assert!(core.tunnels().await.is_empty());
        let status = core.me.call("tunnels_status", json!({})).await.unwrap();
        assert!(status.as_array().unwrap().iter().all(|x| x["state"] != "on"), "{status}");
    }

    #[tokio::test]
    async fn tunnel_arguments_are_checked_before_anything_is_saved() {
        let core = Core::start().await;
        core.granted().await;
        let err = |r: Out| r.unwrap_err();

        assert_eq!(err(core.add("pg", Kind::Local, "web", None, None).await), "This kind of tunnel needs a target: host:port.");
        assert_eq!(err(core.add("pg", Kind::Local, "web", None, Some("db")).await), "db is not host:port.");
        assert_eq!(err(core.add("pg", Kind::Local, "web", Some(5432), Some("db:0")).await), "db:0 is not host:port.");
        assert_eq!(err(core.add("s", Kind::Dynamic, "web", Some(1080), Some("db:22")).await), "A dynamic tunnel takes no target: it is a SOCKS5 proxy to anywhere.");
        assert_eq!(err(core.add("s", Kind::Dynamic, "web", None, None).await), "Give port: the port of the SOCKS5 proxy on this computer (1080 is usual).");
        assert_eq!(err(core.add("r", Kind::Remote, "web", None, Some("localhost:3000")).await), "Give port: the port to open on the host.");
        assert_eq!(err(core.add("pg", Kind::Local, "nowhere", None, Some("db:5432")).await), "No host called nowhere.");
        assert_eq!(err(core.add(" ", Kind::Local, "web", None, Some("db:5432")).await), "Give the tunnel a name, without '/'.");
        assert!(core.tunnels().await.is_empty());

        // A local tunnel listens on the target's port unless told otherwise.
        assert_eq!(core.add("pg", Kind::Local, "web", None, Some(" db:5432 ")).await.unwrap(), "Added pg. Start it with start_tunnel.");
        assert_eq!(err(core.add("PG", Kind::Remote, "db", Some(8080), Some("localhost:80")).await), "There is already a tunnel called PG there.");
        let t = &core.tunnels().await[0];
        assert_eq!((t["port"].clone(), t["target"].clone(), t["via"].clone(), t["state"].clone()), (json!(5432), json!("db:5432"), json!("web"), json!("off")));

        // A change that would break it is refused and nothing changes; dynamic drops the target.
        let mut u = update("pg");
        u.port = Some(0);
        assert_eq!(err(core.agent.update_tunnel(Parameters(u)).await), "Give port: the port to open on this computer.");
        let mut u = update("pg");
        u.kind = Some(Kind::Dynamic);
        u.via = Some("db".into());
        assert_eq!(core.agent.update_tunnel(Parameters(u)).await.unwrap(), "Saved.");
        let t = &core.tunnels().await[0];
        assert_eq!((t["kind"].clone(), t["target"].clone(), t["via"].clone()), (json!("dynamic"), Value::Null, json!("db")));
        let mut u = update("pg");
        u.kind = Some(Kind::Remote);
        assert_eq!(err(core.agent.update_tunnel(Parameters(u)).await), "This kind of tunnel needs a target: host:port.");
        assert_eq!(err(core.agent.update_tunnel(Parameters(update("nope"))).await), "No tunnel called nope (list_tunnels shows them).");
        assert_eq!(err(core.agent.remove_tunnel(Parameters(NameArg { name: "nope".into() })).await), "No tunnel called nope (list_tunnels shows them).");
    }

    #[tokio::test]
    async fn a_teams_tunnels_are_listed_and_changed_as_team_name() {
        let core = Core::start().await;
        let shared = core.dir.join("team");
        let recovery = core.me.call("recovery_key_new", json!({})).await.unwrap();
        let team = core.me.call("team_create", json!({ "path": shared.display().to_string(), "name": "Acme", "myName": "Me", "recoveryKey": recovery })).await.unwrap();
        let tid = team["id"].clone();
        let host = json!({ "id": "a", "group": "", "label": "gitlab-box", "address": "10.0.0.1", "port": 22, "user": "root", "auth": { "type": "agent" }, "tags": [], "hasPassword": false });
        // The person saved a tunnel in the team, before the agent came.
        let tunnel = json!({ "id": "t1", "name": "gitlab", "kind": "local", "bindPort": 8443, "viaHostId": "a", "target": "localhost:443" });
        core.me.call("team_put", json!({ "id": tid, "update": { "hosts": [host], "keys": [], "snippets": [], "tunnels": [tunnel] } })).await.unwrap();
        core.granted().await;

        let list = core.tunnels().await;
        assert_eq!(list, vec![json!({ "name": "Acme/gitlab", "kind": "local", "port": 8443, "target": "localhost:443", "via": "Acme/gitlab-box", "state": "off" })]);

        // One of the same name in the personal vault: `gitlab` is that one, the team's is `Acme/gitlab`.
        assert_eq!(core.add("gitlab", Kind::Local, "web", Some(9443), Some("localhost:443")).await.unwrap(), "Added gitlab. Start it with start_tunnel.");
        assert_eq!(core.add("ui", Kind::Local, "Acme/gitlab-box", None, Some("localhost:3000")).await.unwrap(), "Added Acme/ui. Start it with start_tunnel.");
        let mut u = update("Acme/gitlab");
        u.port = Some(8444);
        assert_eq!(core.agent.update_tunnel(Parameters(u)).await.unwrap(), "Saved.");
        let mut u = update("Acme/gitlab");
        u.via = Some("web".into());
        assert_eq!(core.agent.update_tunnel(Parameters(u)).await.unwrap_err(), "web is not where Acme/gitlab is: a tunnel goes through a host of its own vault (or team).");
        assert_eq!(core.agent.remove_tunnel(Parameters(NameArg { name: "gitlab".into() })).await.unwrap(), "Removed.");

        let teams = core.me.call("teams_list", json!({})).await.unwrap();
        let saved = teams[0]["view"]["data"]["tunnels"].as_array().unwrap().clone();
        let mut ports: Vec<(String, u64)> = saved.iter().map(|t| (s(&t["name"]), t["bindPort"].as_u64().unwrap())).collect();
        ports.sort();
        assert_eq!(ports, vec![("gitlab".into(), 8444), ("ui".into(), 3000)]);
        assert!(core.me.call("vault_get", json!({})).await.unwrap()["tunnels"].as_array().unwrap().is_empty());
    }
}
