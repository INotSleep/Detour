//! Who may use the core without asking.
//!
//! Any program of this user can reach the socket, an agent with a shell included.
//! So the command line works only in a terminal the person trusted: by "Allow" in
//! the window, or by typing the master password there. The core finds the terminal
//! itself, from the process on the other end of the socket (its parent is the shell),
//! so a program can't claim to be another terminal. The window is trusted when it is
//! the app's own program. Agents get tokens instead (see `agents`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// How long a terminal stays trusted (while its shell runs).
const GRANT: Duration = Duration::from_secs(12 * 3600);

/// The shell a client runs in, as the core sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    /// Its process id and start time: a reused id is another terminal.
    #[serde(skip)]
    pub key: String,
    /// The shell's program, to show when asking (`pwsh.exe`, `bash`).
    pub name: String,
    pub pid: u32,
    /// The program the shell runs in (`WindowsTerminal.exe`, `Code.exe`, an agent's), if known.
    pub host: Option<String>,
}

/// What the core knows of the program on the other end of a connection.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub exe: Option<PathBuf>,
    pub terminal: Option<Terminal>,
}

pub fn peer(pid: Option<u32>) -> Peer {
    let Some(pid) = pid else { return Peer::default() };
    let Ok(mut sys) = System::new() else { return Peer::default() };
    let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, refresh);
    let Some(me) = sys.process(Pid::from_u32(pid)) else { return Peer::default() };
    let (exe, parent) = (me.exe().map(Path::to_path_buf), me.parent());
    // The terminal is the shell the command runs in. Shells that start each command through
    // another copy of themselves (Git Bash, a subshell) count as one: the outermost of them.
    let mut proc = |pid: Pid| {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh);
        sys.process(pid).map(|p| (p.name().to_string_lossy().to_string(), p.parent(), p.start_time()))
    };
    let terminal = parent.and_then(|mut shell| {
        let (name, mut up, mut started) = {
            let (n, u, s) = proc(shell)?;
            (n, u, s)
        };
        let mut host = None;
        for _ in 0..16 {
            let Some(next) = up else { break };
            let Some((n, u, s)) = proc(next) else { break };
            if n != name {
                host = Some(n);
                break;
            }
            (shell, up, started) = (next, u, s);
        }
        Some(Terminal { key: format!("{}:{started}", shell.as_u32()), name, pid: shell.as_u32(), host })
    });
    Peer { exe, terminal }
}

/// Is `exe` the app's own program (the window), next to where the core came from?
pub fn is_app(exe: &Path, app: Option<&Path>) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    app.is_some_and(|a| canon(a) == canon(exe))
}

/// Terminals trusted until a moment.
#[derive(Default)]
pub struct Grants(Mutex<HashMap<String, Instant>>);

impl Grants {
    pub fn grant(&self, t: &Terminal) {
        self.0.lock().unwrap().insert(t.key.clone(), Instant::now() + GRANT);
    }

    pub fn holds(&self, t: &Terminal) -> bool {
        let mut g = self.0.lock().unwrap();
        g.retain(|_, until| *until > Instant::now());
        g.contains_key(&t.key)
    }

    /// Forgets every terminal (when the vault locks).
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}
