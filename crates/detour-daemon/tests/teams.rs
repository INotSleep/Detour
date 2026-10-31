//! Every team method with exactly the JSON the app sends (src/lib/api.ts), over the
//! core's socket: a renamed argument or a serde mismatch fails here, not in the window.

use std::time::Duration;

use detour_ipc::Client;
use serde_json::{Value, json};

async fn start(tag: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("detour-core-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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
    Client::connect(socket).await.unwrap()
}

async fn ok(c: &Client, method: &str, params: Value) -> Value {
    c.call(method, params).await.unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

#[tokio::test]
async fn team_flow_over_the_socket() {
    let alice = client(&start("alice").await).await;
    let bob = client(&start("bob").await).await;
    ok(&alice, "vault_create", json!({ "masterPassword": "alice-master" })).await;
    ok(&bob, "vault_create", json!({ "masterPassword": "bob-master" })).await;

    let shared = std::env::temp_dir().join(format!("detour-core-shared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&shared);
    let path = shared.display().to_string();

    let recovery = ok(&alice, "recovery_key_new", json!({})).await;
    let team = ok(&alice, "team_create", json!({ "path": path, "name": "Acme", "myName": "Alice", "recoveryKey": recovery })).await;
    let id = team["id"].as_str().unwrap().to_string();
    assert_eq!(team["myRole"], "owner");

    // Same shape as toView() in api.ts, including a typed password.
    let update = json!({
        "hosts": [{ "id": "h1", "group": "Prod", "label": "api-1", "address": "10.0.0.1", "port": 22, "user": "deploy",
                    "auth": { "type": "password" }, "tags": ["prod"], "hasPassword": false, "newPassword": "pw" }],
        "keys": [], "snippets": [{ "id": "s1", "name": "Disk", "command": "df -h" }],
        "tunnels": [{ "id": "t1", "name": "DB", "kind": "local", "bindPort": 5432, "viaHostId": "h1", "target": "db:5432" }]
    });
    let view = ok(&alice, "team_put", json!({ "id": id, "update": update })).await;
    assert_eq!(view["data"]["hosts"][0]["hasPassword"], true);
    assert!(!view.to_string().contains("\"pw\""), "the password must not come back to the UI");

    let key = ok(&alice, "key_generate", json!({ "vaultId": id, "name": "deploy", "algorithm": "ed25519", "passphrase": null, "remember": true })).await;
    assert!(key["publicKey"].as_str().unwrap().starts_with("ssh-ed25519"));

    // Bob: my id, inspect, join, then Alice adds him.
    let bob_id = ok(&bob, "my_id", json!({})).await;
    let fp = ok(&alice, "id_inspect", json!({ "detourId": bob_id["detourId"] })).await;
    assert_eq!(fp, bob_id["fingerprint"]);
    let info = ok(&bob, "team_inspect", json!({ "path": path })).await;
    assert_eq!(info["rootFingerprint"], ok(&alice, "my_id", json!({})).await["fingerprint"]);
    let joined = ok(&bob, "team_join", json!({ "path": path, "root": info["root"] })).await;
    assert_eq!(joined["joined"], false);
    ok(&alice, "team_member_add", json!({ "id": id, "detourId": bob_id["detourId"], "name": "Bob", "role": "member", "access": "view" })).await;
    let view = ok(&bob, "team_sync", json!({ "id": id })).await;
    assert_eq!(view["joined"], true);
    assert_eq!(view["data"]["snippets"][0]["command"], "df -h");
    let bob_member = view["members"].as_array().unwrap().iter().find(|m| m["isYou"] == true).unwrap()["id"].clone();

    // With View access Bob can't have the key or the password; Alice can.
    let key_id = key["id"].as_str().unwrap();
    let denied = bob.call("key_export", json!({ "vaultId": id, "keyId": key_id, "passphrase": null })).await.unwrap_err();
    assert_eq!(denied["kind"], "forbidden");
    assert_eq!(bob.call("host_password", json!({ "vaultId": id, "hostId": "h1", "own": false })).await.unwrap_err()["kind"], "forbidden");
    assert!(ok(&alice, "key_export", json!({ "vaultId": id, "keyId": key_id, "passphrase": null })).await.as_str().unwrap().contains("OPENSSH PRIVATE KEY"));
    assert_eq!(ok(&alice, "host_password", json!({ "vaultId": id, "hostId": "h1", "own": false })).await, "pw");

    // Bob's own login for the host stays on his side.
    let view = ok(&bob, "team_own_login_set", json!({ "id": id, "hostId": "h1", "login": { "user": "bob", "auth": { "type": "password" }, "newPassword": "bobs" } })).await;
    assert_eq!(view["data"]["hosts"][0]["own"]["hasPassword"], true);
    assert_eq!(ok(&bob, "host_password", json!({ "vaultId": id, "hostId": "h1", "own": true })).await, "bobs");

    // The matrix: Alice gives Bob Edit on the host.
    let view = ok(&alice, "team_access_set", json!({ "id": id, "changes": [{ "record": "h1", "member": bob_member, "level": "edit" }] })).await;
    assert!(view["access"].as_array().unwrap().iter().any(|a| a["member"] == bob_member && a["record"] == "h1" && a["level"] == "edit"));
    ok(&bob, "team_sync", json!({ "id": id })).await;

    // A conflict: both change the port without syncing, then Alice resolves it.
    let mut a_up = update.clone();
    a_up["hosts"][0]["port"] = json!(2200);
    a_up["hosts"][0]["newPassword"] = Value::Null;
    a_up["keys"] = json!([key]);
    let mut b_up = a_up.clone();
    b_up["hosts"][0]["port"] = json!(2300);
    ok(&alice, "team_put", json!({ "id": id, "update": a_up })).await;
    ok(&bob, "team_put", json!({ "id": id, "update": b_up })).await;
    let view = ok(&alice, "team_sync", json!({ "id": id })).await;
    let conflict = &view["conflicts"][0];
    assert_eq!(conflict["kind"], "field");
    assert_eq!(conflict["field"], "port");
    let view = ok(&alice, "team_resolve_field", json!({ "id": id, "coll": "host", "record": "h1", "field": "port", "pick": 0 })).await;
    assert!(view["conflicts"].as_array().unwrap().is_empty());

    // Roles and removal.
    ok(&alice, "team_member_update", json!({ "id": id, "member": bob_member, "role": "admin" })).await;
    let view = ok(&alice, "team_member_remove", json!({ "id": id, "member": bob_member })).await;
    assert!(view["members"].as_array().unwrap().iter().all(|m| m["name"] != "Bob"));

    // Listing, renaming, leaving.
    let list = ok(&alice, "teams_list", json!({})).await;
    assert_eq!(list[0]["view"]["name"], "Acme");
    ok(&alice, "team_rename", json!({ "id": id, "name": "Acme Infra" })).await;
    ok(&bob, "team_leave", json!({ "id": id })).await;
    assert!(ok(&bob, "teams_list", json!({})).await.as_array().unwrap().is_empty());

    // Recovery on a fresh install.
    let carol = client(&start("carol").await).await;
    ok(&carol, "vault_create", json!({ "masterPassword": "carol-master" })).await;
    let view = ok(&carol, "team_recover", json!({ "path": path, "recoveryKey": recovery, "myName": "Carol" })).await;
    assert_eq!(view["myRole"], "owner");

    // Errors keep their kind for the UI.
    let err = alice.call("team_inspect", json!({ "path": std::env::temp_dir().join("detour-core-missing").display().to_string() })).await.unwrap_err();
    assert!(err["kind"] == "notATeamVault" || err["kind"] == "io", "{err}");
}
