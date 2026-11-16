//! The Detour core as a process of its own (`detour daemon`).
//!
//! It holds the unlocked vault, terminal sessions, tunnels, file browsers and
//! the background sync, and serves them over a local socket (see `detour-ipc`)
//! to the desktop app, the command line and agents. Closing a client doesn't
//! stop anything; the core ends on `quit`, or by itself once it is locked and idle.

mod agents;
mod background;
mod rpc;
pub mod sessions;
mod trust;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use detour_core::sftp::Files;
use detour_core::store::Store;
use detour_core::tunnel::Tunnels;
use detour_ipc::{Frame, Message, read_frame, write_frame};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Notify, mpsc};

use sessions::Sessions;

/// Settings of the core itself, kept next to the vault (nothing secret).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Lock after this many minutes without anyone using Detour; 0 never.
    #[serde(default)]
    pub auto_lock_minutes: u32,
}

/// What kind of program is on the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientKind {
    #[default]
    Desktop,
    Cli,
    Mcp,
}

struct Conn {
    kind: ClientKind,
    /// May do what the person can (see `trust`).
    trusted: bool,
    peer: trust::Peer,
    out: mpsc::UnboundedSender<Frame>,
}

/// Who the core trusts without asking.
#[derive(Debug, Clone)]
pub enum Trust {
    /// The window when it is this program (`detour-desktop`), terminals once allowed.
    App(Option<PathBuf>),
    /// Every client: only for tests in this process.
    #[doc(hidden)]
    Everyone,
}

pub struct Core {
    pub(crate) data_dir: PathBuf,
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) sessions: Sessions,
    pub(crate) tunnels: Tunnels,
    pub(crate) files: Files,
    pub(crate) settings: Mutex<Settings>,
    trust: Trust,
    pub(crate) grants: trust::Grants,
    pub(crate) agents: agents::Agents,
    /// Terminals waiting for "Allow" in the window.
    pub(crate) asking: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<bool>>>,
    conns: Mutex<HashMap<u64, Conn>>,
    next_conn: AtomicU64,
    /// When someone last did something (for auto-lock and for ending when idle).
    last_activity: Mutex<Instant>,
    pub(crate) quit: Notify,
}

impl Core {
    fn new(data_dir: PathBuf, trust: Trust) -> Arc<Self> {
        let settings = std::fs::read(data_dir.join("core.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Arc::new(Self {
            store: Arc::new(Mutex::new(Store::open(&data_dir))),
            data_dir,
            sessions: Sessions::default(),
            tunnels: Tunnels::new(),
            files: Files::new(),
            settings: Mutex::new(settings),
            trust,
            grants: trust::Grants::default(),
            agents: agents::Agents::default(),
            asking: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            last_activity: Mutex::new(Instant::now()),
            quit: Notify::new(),
        })
    }

    /// Tells every trusted client that something changed. `by` is the connection that did it (0: the core itself).
    pub(crate) fn emit(&self, name: &str, data: Value) {
        let frame = Frame::Msg(Message::Event { name: name.into(), data });
        self.conns.lock().unwrap().retain(|_, c| !c.trusted || c.out.send(frame.clone()).is_ok());
    }

    pub(crate) fn is_trusted(&self, conn: u64) -> bool {
        matches!(self.trust, Trust::Everyone) || self.conns.lock().unwrap().get(&conn).is_some_and(|c| c.trusted)
    }

    /// A connection proved it is the person (master password, "Allow"): it and its terminal are trusted.
    pub(crate) fn trust_conn(&self, conn: u64) {
        if let Some(c) = self.conns.lock().unwrap().get_mut(&conn) {
            c.trusted = true;
            // Only the command line is in a terminal: the window's parent is not one to trust.
            if c.kind == ClientKind::Cli
                && let Some(t) = &c.peer.terminal
            {
                self.grants.grant(t);
            }
        }
    }

    pub(crate) fn terminal_of(&self, conn: u64) -> Option<trust::Terminal> {
        self.conns.lock().unwrap().get(&conn).and_then(|c| c.peer.terminal.clone())
    }

    /// Is a trusted window open to ask in?
    pub(crate) fn has_window(&self) -> bool {
        self.conns.lock().unwrap().values().any(|c| c.kind == ClientKind::Desktop && c.trusted)
    }

    pub(crate) fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    pub(crate) fn idle_for(&self) -> std::time::Duration {
        self.last_activity.lock().unwrap().elapsed()
    }

    pub(crate) fn has_clients(&self) -> bool {
        !self.conns.lock().unwrap().is_empty()
    }

    pub(crate) fn save_settings(&self) -> std::io::Result<()> {
        let s = self.settings.lock().unwrap().clone();
        std::fs::write(self.data_dir.join("core.json"), serde_json::to_vec_pretty(&s)?)
    }

    /// Locks the vault: tunnels and file browsers close; terminals stay, as they did in the app.
    pub(crate) fn lock(&self) {
        self.tunnels.stop_all();
        self.files.close_all();
        if let Ok(mut s) = self.store.lock()
            && s.is_unlocked()
        {
            s.lock();
            drop(s);
            self.emit("locked", Value::Null);
        }
        // Terminals are asked again after a lock; the window is still the app.
        self.grants.clear();
        for c in self.conns.lock().unwrap().values_mut().filter(|c| c.kind != ClientKind::Desktop) {
            c.trusted = false;
        }
    }
}

/// The calling connection, as a method sees it.
pub(crate) struct Ctx {
    pub id: u64,
    pub out: mpsc::UnboundedSender<Frame>,
}

/// Runs the core of a vault folder until it is told to quit or ends by itself.
/// Fails at once when another core already serves this folder.
/// `app` is the window's program, which is trusted when it connects.
pub async fn run(data_dir: &Path, app: Option<PathBuf>) -> std::io::Result<()> {
    serve(data_dir, Trust::App(app), false).await
}

/// The core as the command line or an agent's MCP server starts it. On a computer with a
/// desktop the app comes up in the tray (its window hidden): what the core holds shows
/// there, and an agent's or a terminal's request reaches the person.
pub async fn run_with_tray(data_dir: &Path, app: Option<PathBuf>) -> std::io::Result<()> {
    serve(data_dir, Trust::App(app), true).await
}

#[doc(hidden)]
pub async fn serve_with(data_dir: &Path, trust: Trust) -> std::io::Result<()> {
    serve(data_dir, trust, false).await
}

/// Whether a window can show here: not over SSH, and on Linux with a display.
fn desktop_session() -> bool {
    let var = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if var("DETOUR_NO_TRAY") || var("SSH_CONNECTION") || var("SSH_TTY") {
        return false;
    }
    cfg!(any(windows, target_os = "macos")) || var("DISPLAY") || var("WAYLAND_DISPLAY")
}

/// Starts the app in the tray unless a window joins by itself first.
fn tray_app(core: Arc<Core>, app: PathBuf) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if core.has_window() {
            return;
        }
        // A vault folder of its own (a test vault, a portable copy) goes with it.
        let dir = core.data_dir.clone();
        let envs: Vec<(&str, &std::ffi::OsStr)> = if dir != detour_core::store::Store::default_dir() { vec![("DETOUR_DATA_DIR", dir.as_os_str())] } else { vec![] };
        let _ = detour_ipc::start_process_with(&app, &["--tray".into()], &envs);
    });
}

async fn serve(data_dir: &Path, trust: Trust, tray: bool) -> std::io::Result<()> {
    let listener = detour_ipc::listen(&detour_ipc::socket_name(data_dir)).await?;
    detour_core::logfile::init(data_dir, "core");
    detour_core::logfile::info(&format!("Detour core {} started ({} {})", detour_core::VERSION, std::env::consts::OS, std::env::consts::ARCH));
    let app = match &trust {
        Trust::App(app) => app.clone(),
        Trust::Everyone => None,
    };
    let core = Core::new(data_dir.to_path_buf(), trust);
    background::spawn(core.clone());
    if let Some(app) = app.filter(|a| tray && a.is_file() && desktop_session()) {
        tray_app(core.clone(), app);
    }
    loop {
        tokio::select! {
            conn = listener.accept() => {
                let Ok(conn) = conn else { continue };
                tokio::spawn(serve_conn(core.clone(), conn));
            }
            _ = core.quit.notified() => break,
        }
    }
    core.tunnels.stop_all();
    core.files.close_all();
    for s in core.sessions.list() {
        if let Some(s) = core.sessions.get(&s.id) {
            s.close();
        }
    }
    Ok(())
}

/// The core inside an app's own process (the iOS app, where nothing runs beside the app):
/// no socket, and the app is its one client, trusted as the window is. What the core sends
/// the app (events, terminal output, a session's end) goes to `deliver`.
pub struct Embedded {
    core: Arc<Core>,
    id: u64,
    out: mpsc::UnboundedSender<Frame>,
}

impl Embedded {
    /// Opens the core of a vault folder. Must run inside a Tokio runtime.
    pub fn start(data_dir: &Path, deliver: impl Fn(Frame) + Send + Sync + 'static) -> Self {
        detour_core::logfile::init(data_dir, "app");
        detour_core::logfile::info(&format!("Detour core {} started in the app ({} {})", detour_core::VERSION, std::env::consts::OS, std::env::consts::ARCH));
        let core = Core::new(data_dir.to_path_buf(), Trust::App(None));
        background::spawn(core.clone());
        let id = core.next_conn.fetch_add(1, Ordering::Relaxed);
        let (out, mut rx) = mpsc::unbounded_channel::<Frame>();
        core.conns.lock().unwrap().insert(id, Conn { kind: ClientKind::Desktop, trusted: true, peer: trust::Peer::default(), out: out.clone() });
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                deliver(frame);
            }
        });
        Self { core, id, out }
    }

    /// A call as a client makes it over the socket (`vault_unlock`, `session_open`, ...).
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
        let ctx = Ctx { id: self.id, out: self.out.clone() };
        let r = rpc::call(&self.core, &ctx, method, params).await;
        if let Err(e) = &r {
            logged(method, e);
        }
        r
    }

    /// Ends the sessions, tunnels and file browsers (the app is going away).
    pub fn stop(&self) {
        self.core.tunnels.stop_all();
        self.core.files.close_all();
        for s in self.core.sessions.list() {
            if let Some(s) = self.core.sessions.get(&s.id) {
                s.close();
            }
        }
    }
}

/// A call that failed, by its method and error (never its arguments). What is asked of the
/// person on the way (a password, a key to trust, unlocking) is not an error.
fn logged(method: &str, err: &Value) {
    const ASKS: &[&str] = &["needSecret", "hostKeyUnknown", "locked", "notTrusted", "denied"];
    let kind = err["kind"].as_str().unwrap_or("");
    if ASKS.contains(&kind) || method == "log_write" {
        return;
    }
    detour_core::logfile::write("warn", &format!("{method}: {err}"));
}

async fn serve_conn(core: Arc<Core>, stream: detour_ipc::Stream) {
    let pid = detour_ipc::peer_pid(&stream);
    let peer = tokio::task::spawn_blocking(move || trust::peer(pid)).await.unwrap_or_default();
    let stream = Arc::new(stream);
    let id = core.next_conn.fetch_add(1, Ordering::Relaxed);
    let (out, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    core.conns.lock().unwrap().insert(id, Conn { kind: ClientKind::Desktop, trusted: false, peer, out: out.clone() });

    let writer = stream.clone();
    let write_task = tokio::spawn(async move {
        let mut w = &*writer;
        while let Some(frame) = out_rx.recv().await {
            if write_frame(&mut w, &frame).await.is_err() {
                break;
            }
        }
    });

    let mut r = &*stream;
    while let Ok(frame) = read_frame(&mut r).await {
        let Frame::Msg(Message::Call { id: call, method, params }) = frame else { continue };
        // The first call says who is on the other end.
        if method == "hello" {
            let kind = params.get("client").and_then(|k| serde_json::from_value(k.clone()).ok()).unwrap_or_default();
            let app = match &core.trust {
                Trust::App(app) => app.clone(),
                Trust::Everyone => None,
            };
            if let Some(c) = core.conns.lock().unwrap().get_mut(&id) {
                c.kind = kind;
                c.trusted = match kind {
                    ClientKind::Desktop => c.peer.exe.as_deref().is_some_and(|exe| trust::is_app(exe, app.as_deref())),
                    ClientKind::Cli => c.peer.terminal.as_ref().is_some_and(|t| core.grants.holds(t)),
                    ClientKind::Mcp => false,
                };
            }
        }
        let (core, out) = (core.clone(), out.clone());
        tokio::spawn(async move {
            let ctx = Ctx { id, out: out.clone() };
            let reply = match rpc::call(&core, &ctx, &method, params).await {
                Ok(ok) => Message::Reply { id: call, ok, err: None },
                Err(err) => {
                    logged(&method, &err);
                    Message::Reply { id: call, ok: Value::Null, err: Some(err) }
                }
            };
            let _ = out.send(Frame::Msg(reply));
        });
    }

    core.conns.lock().unwrap().remove(&id);
    core.sessions.drop_conn(id);
    write_task.abort();
}
