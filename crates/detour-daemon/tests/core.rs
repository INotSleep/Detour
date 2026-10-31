//! The core as a process serves: a real core on a temporary vault folder and
//! clients talking to it over its socket, as the app and the CLI will.

use std::sync::Arc;
use std::time::Duration;

use detour_ipc::{Client, StreamItem};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use serde_json::{Value, json};
use tokio::net::TcpListener;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-core-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Starts a core on its own folder and returns a way to connect to it.
async fn start(tag: &str) -> String {
    let dir = temp_dir(tag);
    let socket = detour_ipc::socket_name(&dir);
    tokio::spawn(async move { detour_daemon::serve_with(&dir, detour_daemon::Trust::Everyone).await.unwrap() });
    for _ in 0..100 {
        if Client::connect(&socket).await.is_ok() {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the core did not start");
}

async fn client(socket: &str) -> Client {
    let c = Client::connect(socket).await.unwrap();
    c.call("hello", json!({ "client": "desktop" })).await.unwrap();
    c
}

async fn ok(c: &Client, method: &str, params: Value) -> Value {
    c.call(method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn snippet(id: &str, command: &str) -> Value {
    json!({ "id": id, "name": id, "command": command })
}

fn update(snippets: Vec<Value>) -> Value {
    json!({ "hosts": [], "keys": [], "snippets": snippets, "tunnels": [] })
}

#[tokio::test]
async fn second_client_needs_no_password_and_edits_merge() {
    let socket = start("merge").await;
    let app = client(&socket).await;
    assert_eq!(ok(&app, "vault_status", json!({})).await["initialized"], false);
    ok(&app, "vault_create", json!({ "masterPassword": "master" })).await;

    // The command line comes later: the core is already unlocked.
    let cli = Client::connect(&socket).await.unwrap();
    let cli_id = ok(&cli, "hello", json!({ "client": "cli" })).await["connId"].clone();
    assert_eq!(ok(&cli, "vault_status", json!({})).await["unlocked"], true);
    let mut events = app.events();

    // Both start from the same vault.
    let base = update(vec![snippet("a", "ls")]);
    ok(&app, "vault_put", json!({ "update": base })).await;
    // The CLI adds a snippet...
    ok(&cli, "vault_put", json!({ "base": base, "update": update(vec![snippet("a", "ls"), snippet("b", "df")]) })).await;
    // ...and the app, not having seen it, edits another one.
    let view = ok(&app, "vault_put", json!({ "base": base, "update": update(vec![snippet("a", "ls -la")]) })).await;
    let got: Vec<(String, String)> = view["snippets"].as_array().unwrap().iter().map(|s| (s["id"].as_str().unwrap().into(), s["command"].as_str().unwrap().into())).collect();
    assert_eq!(got, [("a".into(), "ls -la".into()), ("b".into(), "df".into())]);

    // The app heard about the CLI's edit.
    let ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let e = events.recv().await.unwrap();
            if e.name == "data.changed" && e.data["by"] == cli_id {
                return e;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ev.data["method"], "vault_put");

    // Wrong arguments and unknown methods are errors, not crashes.
    assert_eq!(cli.call("vault_unlock", json!({})).await.unwrap_err()["kind"], "badArguments");
    assert_eq!(cli.call("nope", json!({})).await.unwrap_err()["kind"], "unknownMethod");
}

#[derive(Clone)]
struct Sshd;

impl server::Server for Sshd {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Sshd {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
        Ok(if user == "u" && password == "pw" { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, b"ready$ ".to_vec())?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if data == b"exit\r" {
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
            return Ok(());
        }
        session.data(channel, [b"echo:".as_slice(), data].concat())?;
        Ok(())
    }
}

async fn sshd() -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = Sshd.run_on_socket(config, &listener).await;
    });
    port
}

/// Reads a stream until its text contains `needle`; returns all of it.
async fn read_until(rx: &mut tokio::sync::mpsc::UnboundedReceiver<StreamItem>, needle: &str) -> String {
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !text.contains(needle) {
            match rx.recv().await {
                Some(StreamItem::Data(d)) => text.push_str(&String::from_utf8_lossy(&d)),
                Some(StreamItem::Json(_)) => {}
                None => break,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {needle:?} in {text:?}"));
    text
}

#[tokio::test]
async fn a_session_outlives_its_window_and_comes_back_with_its_output() {
    let socket = start("session").await;
    let port = sshd().await;
    let app = client(&socket).await;
    ok(&app, "vault_create", json!({ "masterPassword": "master" })).await;
    let host = json!({ "id": "h1", "group": "", "label": "test", "address": "127.0.0.1", "port": port, "user": "u",
                       "auth": { "type": "password" }, "tags": [], "hasPassword": false, "newPassword": "pw" });
    ok(&app, "vault_put", json!({ "update": { "hosts": [host], "keys": [], "snippets": [], "tunnels": [] } })).await;

    // A new server's key is asked about first, as in the app.
    let (stream, mut out) = app.stream();
    let open = json!({ "args": { "vaultId": "personal", "hostId": "h1", "cols": 80, "rows": 24 }, "stream": stream });
    let fp = app.call("session_open", open.clone()).await.unwrap_err();
    assert_eq!(fp["kind"], "hostKeyUnknown");
    let mut open = open;
    open["args"]["trustFingerprint"] = fp["fingerprint"].clone();
    let opened = ok(&app, "session_open", open).await;
    let id = opened["sessionId"].as_str().unwrap().to_string();
    read_until(&mut out, "ready$ ").await;
    ok(&app, "session_input", json!({ "sessionId": id, "data": "hello" })).await;
    read_until(&mut out, "echo:hello").await;

    // The window closes; the session stays in the core.
    drop(out);
    drop(app);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let again = client(&socket).await;
    let list = ok(&again, "sessions_list", json!({})).await;
    assert_eq!(list[0]["id"], json!(id));
    assert_eq!(list[0]["hostId"], "h1");
    let (stream, mut out) = again.stream();
    ok(&again, "session_attach", json!({ "sessionId": id, "stream": stream })).await;
    let replay = read_until(&mut out, "echo:hello").await;
    assert!(replay.starts_with("ready$ "), "{replay:?}");

    // It ends as before: the stream gets the exit status, the core forgets the session.
    ok(&again, "session_input", json!({ "sessionId": id, "data": "exit\r" })).await;
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(StreamItem::Json(j)) = out.recv().await {
                return j;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(closed, json!({ "closed": true, "exitStatus": 0 }));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ok(&again, "sessions_list", json!({})).await, json!([]));
}

#[tokio::test]
async fn only_one_core_serves_a_folder() {
    let dir = temp_dir("one");
    let d = dir.clone();
    tokio::spawn(async move { detour_daemon::serve_with(&d, detour_daemon::Trust::Everyone).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = detour_daemon::serve_with(&dir, detour_daemon::Trust::Everyone).await;
    assert_eq!(second.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
}
