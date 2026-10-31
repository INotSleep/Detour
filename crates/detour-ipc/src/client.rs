//! A connection to the core: calls with replies, streams and events.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::{Frame, Message, local_error, read_frame, write_frame};

/// One item of a stream: raw bytes (terminal output) or JSON (progress, the end).
#[derive(Debug, Clone)]
pub enum StreamItem {
    Data(Vec<u8>),
    Json(Value),
}

#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>>;
type Streams = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<StreamItem>>>>;

/// Cheap to clone; the connection closes when the last clone is dropped.
#[derive(Clone)]
pub struct Client {
    out: mpsc::UnboundedSender<Frame>,
    pending: Pending,
    streams: Streams,
    events: broadcast::Sender<Event>,
    next: Arc<AtomicU64>,
    closed: watch::Receiver<bool>,
}

impl Client {
    /// Connects to the core of a vault folder (see [`crate::socket_name`]).
    pub async fn connect(socket: &str) -> std::io::Result<Self> {
        let conn = Arc::new(crate::name::connect(socket).await?);
        let reader = conn.clone();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Frame>();
        let pending: Pending = Default::default();
        let streams: Streams = Default::default();
        let (events, _) = broadcast::channel(256);
        let (gone, closed) = watch::channel(false);

        tokio::spawn(async move {
            let mut send = &*conn;
            while let Some(frame) = out_rx.recv().await {
                if write_frame(&mut send, &frame).await.is_err() {
                    break;
                }
            }
        });
        let (p, s, e) = (pending.clone(), streams.clone(), events.clone());
        tokio::spawn(async move {
            let mut recv = &*reader;
            while let Ok(frame) = read_frame(&mut recv).await {
                match frame {
                    Frame::Msg(Message::Reply { id, ok, err }) => {
                        if let Some(tx) = p.lock().unwrap().remove(&id) {
                            let _ = tx.send(match err {
                                Some(err) => Err(err),
                                None => Ok(ok),
                            });
                        }
                    }
                    Frame::Msg(Message::Event { name, data }) => {
                        let _ = e.send(Event { name, data });
                    }
                    Frame::Msg(Message::Stream { stream, data }) => deliver(&s, stream, StreamItem::Json(data)),
                    Frame::Data { stream, bytes } => deliver(&s, stream, StreamItem::Data(bytes)),
                    Frame::Msg(Message::Call { .. }) => {}
                }
            }
            // The core went away: every call still waiting fails, every stream ends.
            for (_, tx) in p.lock().unwrap().drain() {
                let _ = tx.send(Err(local_error("disconnected", "the Detour core closed the connection")));
            }
            s.lock().unwrap().clear();
            let _ = gone.send(true);
        });
        Ok(Self { out, pending, streams, events, next: Arc::new(AtomicU64::new(1)), closed })
    }

    /// Calls a method of the core. `Err` holds the core's error (`{ "kind": ... }`).
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.out.send(Frame::Msg(Message::Call { id, method: method.into(), params })).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(local_error("disconnected", "the Detour core is not running"));
        }
        rx.await.unwrap_or_else(|_| Err(local_error("disconnected", "the Detour core closed the connection")))
    }

    /// A new stream: pass its id to a call that streams, read what it sends from the receiver.
    pub fn stream(&self) -> (u64, mpsc::UnboundedReceiver<StreamItem>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.streams.lock().unwrap().insert(id, tx);
        (id, rx)
    }

    /// Stops delivering a stream (the core is told separately, e.g. by closing the session).
    pub fn drop_stream(&self, id: u64) {
        self.streams.lock().unwrap().remove(&id);
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.borrow() || self.out.is_closed()
    }

    /// Resolves when the connection to the core is gone.
    pub async fn closed(&self) {
        let _ = self.closed.clone().wait_for(|gone| *gone).await;
    }
}

fn deliver(streams: &Streams, id: u64, item: StreamItem) {
    let mut s = streams.lock().unwrap();
    if let Some(tx) = s.get(&id)
        && tx.send(item).is_err()
    {
        s.remove(&id);
    }
}
