//! Calls every team command with exactly the JSON the UI sends (src/lib/api.ts),
//! so a renamed argument or a serde mismatch fails here instead of in the window.

use serde_json::{Value, json};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;

struct Ui {
    webview: tauri::WebviewWindow<MockRuntime>,
    _app: tauri::App<MockRuntime>,
}

impl Ui {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("detour-ipc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = detour_app_lib::builder(mock_builder(), dir).build(mock_context(noop_assets())).unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        Self { webview, _app: app }
    }

    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let req = InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // The app's own origin: http://tauri.localhost on Windows, tauri://localhost elsewhere.
            url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(&self.webview, req).map(|b| b.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }
}

#[test]
fn team_flow_over_ipc() {
    let alice = Ui::new("alice");
    let bob = Ui::new("bob");
    alice.ok("vault_create", json!({ "masterPassword": "alice-master" }));
    bob.ok("vault_create", json!({ "masterPassword": "bob-master" }));

    let shared = std::env::temp_dir().join(format!("detour-ipc-shared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&shared);
    let path = shared.display().to_string();

    let recovery = alice.ok("recovery_key_new", json!({}));
    let team = alice.ok("team_create", json!({ "path": path, "name": "Acme", "myName": "Alice", "recoveryKey": recovery }));
    let id = team["id"].as_str().unwrap().to_string();
    assert_eq!(team["myRole"], "owner");

    // Same shape as toView() in api.ts, including a typed password.
    let update = json!({
        "hosts": [{ "id": "h1", "group": "Prod", "label": "api-1", "address": "10.0.0.1", "port": 22, "user": "deploy",
                    "auth": { "type": "password" }, "tags": ["prod"], "hasPassword": false, "newPassword": "pw" }],
        "keys": [], "snippets": [{ "id": "s1", "name": "Disk", "command": "df -h" }],
        "tunnels": [{ "id": "t1", "name": "DB", "kind": "local", "bindPort": 5432, "viaHostId": "h1", "target": "db:5432" }]
    });
    let view = alice.ok("team_put", json!({ "id": id, "update": update }));
    assert_eq!(view["data"]["hosts"][0]["hasPassword"], true);
    assert!(!view.to_string().contains("\"pw\""), "the password must not come back to the UI");

    let key = alice.ok("key_generate", json!({ "vaultId": id, "name": "deploy", "algorithm": "ed25519", "passphrase": null, "remember": true }));
    assert!(key["publicKey"].as_str().unwrap().starts_with("ssh-ed25519"));

    // Bob: my id, inspect, join, then Alice adds him.
    let bob_id = bob.ok("my_id", json!({}));
    let fp = alice.ok("id_inspect", json!({ "detourId": bob_id["detourId"] }));
    assert_eq!(fp, bob_id["fingerprint"]);
    let info = bob.ok("team_inspect", json!({ "path": path }));
    assert_eq!(info["rootFingerprint"], alice.ok("my_id", json!({}))["fingerprint"]);
    let joined = bob.ok("team_join", json!({ "path": path, "root": info["root"] }));
    assert_eq!(joined["joined"], false);
    alice.ok("team_member_add", json!({ "id": id, "detourId": bob_id["detourId"], "name": "Bob", "role": "member", "access": "view" }));
    let view = bob.ok("team_sync", json!({ "id": id }));
    assert_eq!(view["joined"], true);
    assert_eq!(view["data"]["snippets"][0]["command"], "df -h");
    let bob_member = view["members"].as_array().unwrap().iter().find(|m| m["isYou"] == true).unwrap()["id"].clone();

    // With View access Bob can't have the key or the password; Alice can.
    let key_id = key["id"].as_str().unwrap();
    let denied = bob.call("key_export", json!({ "vaultId": id, "keyId": key_id, "passphrase": null })).unwrap_err();
    assert_eq!(denied["kind"], "forbidden");
    assert_eq!(bob.call("host_password", json!({ "vaultId": id, "hostId": "h1", "own": false })).unwrap_err()["kind"], "forbidden");
    assert!(alice.ok("key_export", json!({ "vaultId": id, "keyId": key_id, "passphrase": null })).as_str().unwrap().contains("OPENSSH PRIVATE KEY"));
    assert_eq!(alice.ok("host_password", json!({ "vaultId": id, "hostId": "h1", "own": false })), "pw");

    // Bob's own login for the host stays on his side.
    let view = bob.ok("team_own_login_set", json!({ "id": id, "hostId": "h1", "login": { "user": "bob", "auth": { "type": "password" }, "newPassword": "bobs" } }));
    assert_eq!(view["data"]["hosts"][0]["own"]["hasPassword"], true);
    assert_eq!(bob.ok("host_password", json!({ "vaultId": id, "hostId": "h1", "own": true })), "bobs");

    // The matrix: Alice gives Bob Edit on the host.
    let view = alice.ok("team_access_set", json!({ "id": id, "changes": [{ "record": "h1", "member": bob_member, "level": "edit" }] }));
    assert!(view["access"].as_array().unwrap().iter().any(|a| a["member"] == bob_member && a["record"] == "h1" && a["level"] == "edit"));
    bob.ok("team_sync", json!({ "id": id }));

    // A conflict: both change the port without syncing, then Alice resolves it.
    let mut a_up = update.clone();
    a_up["hosts"][0]["port"] = json!(2200);
    a_up["hosts"][0]["newPassword"] = Value::Null;
    a_up["keys"] = json!([key]);
    let mut b_up = a_up.clone();
    b_up["hosts"][0]["port"] = json!(2300);
    alice.ok("team_put", json!({ "id": id, "update": a_up }));
    bob.ok("team_put", json!({ "id": id, "update": b_up }));
    let view = alice.ok("team_sync", json!({ "id": id }));
    let conflict = &view["conflicts"][0];
    assert_eq!(conflict["kind"], "field");
    assert_eq!(conflict["field"], "port");
    let view = alice.ok("team_resolve_field", json!({ "id": id, "coll": "host", "record": "h1", "field": "port", "pick": 0 }));
    assert!(view["conflicts"].as_array().unwrap().is_empty());

    // Roles and removal.
    alice.ok("team_member_update", json!({ "id": id, "member": bob_member, "role": "admin" }));
    let view = alice.ok("team_member_remove", json!({ "id": id, "member": bob_member }));
    assert!(view["members"].as_array().unwrap().iter().all(|m| m["name"] != "Bob"));

    // Listing, renaming, leaving.
    let list = alice.ok("teams_list", json!({}));
    assert_eq!(list[0]["view"]["name"], "Acme");
    alice.ok("team_rename", json!({ "id": id, "name": "Acme Infra" }));
    bob.ok("team_leave", json!({ "id": id }));
    assert!(bob.ok("teams_list", json!({})).as_array().unwrap().is_empty());

    // Recovery on a fresh install.
    let carol = Ui::new("carol");
    carol.ok("vault_create", json!({ "masterPassword": "carol-master" }));
    let view = carol.ok("team_recover", json!({ "path": path, "recoveryKey": recovery, "myName": "Carol" }));
    assert_eq!(view["myRole"], "owner");

    // Errors keep their kind for the UI.
    let err = alice.call("team_inspect", json!({ "path": std::env::temp_dir().join("detour-ipc-missing").display().to_string() })).unwrap_err();
    assert!(err["kind"] == "notATeamVault" || err["kind"] == "io", "{err}");
}
