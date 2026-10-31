//! Who the core serves without asking: the app's window, and terminals the person allowed.

use std::time::Duration;

use detour_ipc::Client;
use serde_json::{Value, json};

async fn start(tag: &str, app: Option<std::path::PathBuf>) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("detour-trust-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = detour_ipc::socket_name(&dir);
    tokio::spawn(async move { detour_daemon::run(&dir, app).await.unwrap() });
    for _ in 0..100 {
        if Client::connect(&socket).await.is_ok() {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the core did not start");
}

async fn hello(socket: &str, client: &str) -> Client {
    let c = Client::connect(socket).await.unwrap();
    c.call("hello", json!({ "client": client })).await.unwrap();
    c
}

fn kind(r: Result<Value, Value>) -> String {
    r.err().map(|e| e["kind"].as_str().unwrap_or_default().to_string()).unwrap_or_else(|| "ok".into())
}

#[tokio::test]
async fn a_terminal_without_a_window_needs_the_master_password_once() {
    let socket = start("password", None).await;
    let first = hello(&socket, "cli").await;
    // A new vault: whoever creates it is the person.
    assert_eq!(kind(first.call("vault_create", json!({ "masterPassword": "master" })).await), "ok");
    assert_eq!(kind(first.call("vault_get", json!({})).await), "ok");

    // The vault locks: terminals are asked again.
    first.call("vault_lock", json!({})).await.unwrap();
    let cli = hello(&socket, "cli").await;
    assert_eq!(kind(cli.call("vault_get", json!({})).await), "notTrusted");
    assert_eq!(kind(cli.call("trust_request", json!({})).await), "needMasterPassword");
    assert_eq!(kind(cli.call("trust_with_password", json!({ "masterPassword": "nope" })).await), "wrongMasterPassword");
    assert_eq!(kind(cli.call("trust_with_password", json!({ "masterPassword": "master" })).await), "ok");
    assert_eq!(kind(cli.call("vault_get", json!({})).await), "ok", "the password also unlocked the vault");

    // The next command in the same terminal goes through.
    let again = hello(&socket, "cli").await;
    assert_eq!(kind(again.call("vault_get", json!({})).await), "ok");

    // Something pretending to be the window is not trusted, and hears no events.
    let fake = hello(&socket, "desktop").await;
    assert_eq!(kind(fake.call("vault_get", json!({})).await), "notTrusted");
}

#[tokio::test]
async fn the_window_allows_a_terminal() {
    // This test program plays the app: its window is trusted.
    let socket = start("window", Some(std::env::current_exe().unwrap())).await;
    let window = hello(&socket, "desktop").await;
    window.call("vault_create", json!({ "masterPassword": "master" })).await.unwrap();
    window.call("vault_lock", json!({})).await.unwrap();
    window.call("vault_unlock", json!({ "masterPassword": "master" })).await.unwrap();
    let mut events = window.events();

    let cli = hello(&socket, "cli").await;
    assert_eq!(kind(cli.call("vault_get", json!({})).await), "notTrusted");
    let asking = tokio::spawn({
        let cli = cli.clone();
        async move { cli.call("trust_request", json!({})).await }
    });
    let ev = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let e = events.recv().await.unwrap();
            if e.name == "trust.requested" {
                return e;
            }
        }
    })
    .await
    .unwrap();
    assert!(ev.data["terminal"]["name"].as_str().is_some_and(|n| !n.is_empty()), "{}", ev.data);
    window.call("trust_answer", json!({ "id": ev.data["id"], "allow": true })).await.unwrap();
    assert_eq!(kind(asking.await.unwrap()), "ok");
    assert_eq!(kind(cli.call("vault_get", json!({})).await), "ok");
}

#[tokio::test]
async fn anyone_may_stop_a_core_that_holds_nothing() {
    let socket = start("quit", None).await;
    let stranger = hello(&socket, "cli").await;
    assert_eq!(kind(stranger.call("quit", json!({})).await), "ok");

    // An unlocked vault is the person's: a stranger can't stop it (this test's terminal is the
    // person's once it made the vault, so the stranger is an agent's connection).
    let socket = start("quit-unlocked", None).await;
    let me = hello(&socket, "cli").await;
    me.call("vault_create", json!({ "masterPassword": "master" })).await.unwrap();
    let stranger = hello(&socket, "mcp").await;
    assert_eq!(kind(stranger.call("quit", json!({})).await), "notTrusted");
}
