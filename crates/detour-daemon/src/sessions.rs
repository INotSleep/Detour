//! Terminal sessions kept in the core. They outlive the client that opened them
//! (the window can close and come back), keep their recent output for whoever
//! attaches later, and send what arrives to every attached stream.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use detour_core::ssh::{SessionEvent, SessionHandle};
use detour_ipc::{Frame, Message};
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// How much recent output a session keeps for a client attaching later.
const SCROLLBACK: usize = 1024 * 1024;

/// Who a session belongs to: the person (any of their clients) or an agent's token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Owner {
    User,
    Agent { token_id: String, agent: String },
}

impl Owner {
    pub fn token_id(&self) -> Option<&str> {
        match self {
            Owner::Agent { token_id, .. } => Some(token_id),
            Owner::User => None,
        }
    }
}

/// What a client sees of a session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub vault_id: String,
    pub host_id: String,
    pub owner: Owner,
    pub started_at: i64,
    /// Ends with the client that opened it (`detour ssh`): not something to show elsewhere.
    pub transient: bool,
}

/// How many of an agent's commands a session remembers, and how much of each one's output.
const RUNS: usize = 100;
const RUN_OUTPUT: usize = 64 * 1024;

/// A command an agent ran on the session's host (on a channel of its own), for the person to see.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: u64,
    pub command: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub exit_status: Option<u32>,
    pub timed_out: bool,
    /// Its output, stdout and stderr as they came; the end of it when long.
    pub output: String,
}

struct Watcher {
    conn: u64,
    stream: u64,
    out: UnboundedSender<Frame>,
}

struct State {
    scrollback: VecDeque<u8>,
    watchers: Vec<Watcher>,
}

pub struct Session {
    pub info: SessionInfo,
    handle: SessionHandle,
    /// Closed when the connection that opened it goes (`detour ssh`), not kept like the app's.
    close_with: Option<u64>,
    state: Mutex<State>,
    runs: Mutex<VecDeque<Run>>,
}

impl Session {
    pub fn input(&self, bytes: Vec<u8>) {
        self.handle.input(bytes);
    }
    pub fn resize(&self, cols: u32, rows: u32) {
        self.handle.resize(cols, rows);
    }
    pub fn close(&self) {
        self.handle.close();
    }

    /// Output for whoever watches, kept for whoever attaches later.
    pub fn output(&self, bytes: Vec<u8>) {
        let mut st = self.state.lock().unwrap();
        st.scrollback.extend(&bytes);
        if st.scrollback.len() > SCROLLBACK {
            let cut = st.scrollback.len() - SCROLLBACK;
            st.scrollback.drain(..cut);
            // Start at a line, not in the middle of one.
            if let Some(nl) = st.scrollback.iter().take(4096).position(|&b| b == b'\n') {
                st.scrollback.drain(..=nl);
            }
        }
        st.watchers.retain(|w| w.out.send(Frame::Data { stream: w.stream, bytes: bytes.clone() }).is_ok());
    }

    pub fn run_started(&self, command: &str, at: i64) -> u64 {
        let mut runs = self.runs.lock().unwrap();
        let id = runs.back().map_or(1, |r| r.id + 1);
        runs.push_back(Run { id, command: command.to_string(), started_at: at, finished_at: None, exit_status: None, timed_out: false, output: String::new() });
        if runs.len() > RUNS {
            runs.pop_front();
        }
        id
    }

    pub fn run_output(&self, id: u64, bytes: &[u8]) {
        if let Some(r) = self.runs.lock().unwrap().iter_mut().find(|r| r.id == id) {
            r.output.push_str(&String::from_utf8_lossy(bytes));
            if r.output.len() > RUN_OUTPUT {
                let mut cut = r.output.len() - RUN_OUTPUT;
                while !r.output.is_char_boundary(cut) {
                    cut += 1;
                }
                r.output.drain(..cut);
            }
        }
    }

    pub fn run_finished(&self, id: u64, at: i64, exit_status: Option<u32>, timed_out: bool) {
        if let Some(r) = self.runs.lock().unwrap().iter_mut().find(|r| r.id == id) {
            (r.finished_at, r.exit_status, r.timed_out) = (Some(at), exit_status, timed_out);
        }
    }

    pub fn runs(&self) -> Vec<Run> {
        self.runs.lock().unwrap().iter().cloned().collect()
    }

    /// The last `max` bytes of output.
    pub fn recent(&self, max: usize) -> Vec<u8> {
        let st = self.state.lock().unwrap();
        let skip = st.scrollback.len().saturating_sub(max);
        st.scrollback.iter().skip(skip).copied().collect()
    }
}

#[derive(Clone, Default)]
pub struct Sessions(Arc<Mutex<HashMap<String, Arc<Session>>>>);

impl Sessions {
    /// Keeps a session that just connected and starts passing its output on.
    /// `on_closed` runs once it ends.
    pub fn add(&self, info: SessionInfo, handle: SessionHandle, mut events: UnboundedReceiver<SessionEvent>, close_with: Option<u64>, on_closed: impl FnOnce() + Send + 'static) -> Arc<Session> {
        let session = Arc::new(Session { info, handle, close_with, state: Mutex::new(State { scrollback: VecDeque::new(), watchers: Vec::new() }), runs: Mutex::default() });
        self.0.lock().unwrap().insert(session.info.id.clone(), session.clone());
        let (all, s) = (self.0.clone(), session.clone());
        tokio::spawn(async move {
            let mut exit_status = None;
            while let Some(ev) = events.recv().await {
                match ev {
                    SessionEvent::Data(bytes) => s.output(bytes),
                    SessionEvent::Closed { exit_status: e } => {
                        exit_status = e;
                        break;
                    }
                }
            }
            all.lock().unwrap().remove(&s.info.id);
            let data = serde_json::json!({ "closed": true, "exitStatus": exit_status });
            for w in s.state.lock().unwrap().watchers.drain(..) {
                let _ = w.out.send(Frame::Msg(Message::Stream { stream: w.stream, data: data.clone() }));
            }
            on_closed();
        });
        session
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.0.lock().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut out: Vec<SessionInfo> = self.0.lock().unwrap().values().map(|s| s.info.clone()).collect();
        out.sort_by_key(|i| i.started_at);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }

    /// Sends a session's recent output to a stream, then everything that follows.
    pub fn attach(&self, session: &Session, conn: u64, stream: u64, out: UnboundedSender<Frame>) {
        let mut st = session.state.lock().unwrap();
        if !st.scrollback.is_empty() {
            let _ = out.send(Frame::Data { stream, bytes: st.scrollback.iter().copied().collect() });
        }
        st.watchers.push(Watcher { conn, stream, out });
    }

    pub fn detach(&self, session: &Session, stream: u64) {
        session.state.lock().unwrap().watchers.retain(|w| w.stream != stream);
    }

    /// A connection went away: it no longer watches anything, and what it opened to close with it closes.
    pub fn drop_conn(&self, conn: u64) {
        for s in self.0.lock().unwrap().values() {
            s.state.lock().unwrap().watchers.retain(|w| w.conn != conn);
            if s.close_with == Some(conn) {
                s.close();
            }
        }
    }

    /// Closes every session of an agent's token (when it is revoked).
    pub fn close_owned_by(&self, token_id: &str) {
        for s in self.0.lock().unwrap().values().filter(|s| s.info.owner.token_id() == Some(token_id)) {
            s.close();
        }
    }
}
