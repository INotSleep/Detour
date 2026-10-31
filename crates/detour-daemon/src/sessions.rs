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
    Agent { token_id: String },
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
}

#[derive(Clone, Default)]
pub struct Sessions(Arc<Mutex<HashMap<String, Arc<Session>>>>);

impl Sessions {
    /// Keeps a session that just connected and starts passing its output on.
    /// `on_closed` runs once it ends.
    pub fn add(&self, info: SessionInfo, handle: SessionHandle, mut events: UnboundedReceiver<SessionEvent>, close_with: Option<u64>, on_closed: impl FnOnce() + Send + 'static) -> Arc<Session> {
        let session = Arc::new(Session { info, handle, close_with, state: Mutex::new(State { scrollback: VecDeque::new(), watchers: Vec::new() }) });
        self.0.lock().unwrap().insert(session.info.id.clone(), session.clone());
        let (all, s) = (self.0.clone(), session.clone());
        tokio::spawn(async move {
            let mut exit_status = None;
            while let Some(ev) = events.recv().await {
                match ev {
                    SessionEvent::Data(bytes) => {
                        let mut st = s.state.lock().unwrap();
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
    pub fn close_owned_by(&self, owner: &Owner) {
        for s in self.0.lock().unwrap().values().filter(|s| &s.info.owner == owner) {
            s.close();
        }
    }
}
