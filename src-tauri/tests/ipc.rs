//! The window's commands with exactly the JSON the UI sends (src/lib/api.ts): calls
//! reach the core through `core_call`, and its errors come back with their kind.
//! Every core method itself is covered in crates/detour-daemon/tests.

use serde_json::{Value, json};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;

struct Ui {
    webview: tauri::WebviewWindow<MockRuntime>,
    _app: tauri::App<MockRuntime>,
}

impl Ui {
    /// A window on a vault folder whose core already runs (a test can't start one as the app does).
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("detour-ipc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let core_dir = dir.clone();
        std::thread::spawn(move || tokio::runtime::Runtime::new().unwrap().block_on(detour_daemon::run(&core_dir)).unwrap());
        let socket = detour_ipc::socket_name(&dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!((0..100).any(|_| rt.block_on(detour_ipc::Client::connect(&socket)).is_ok() || { std::thread::sleep(std::time::Duration::from_millis(20)); false }));
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

    fn core(&self, method: &str, params: Value) -> Result<Value, Value> {
        self.call("core_call", json!({ "method": method, "params": params }))
    }
}

#[test]
fn the_window_reaches_the_core() {
    let ui = Ui::new("window");
    assert_eq!(ui.core("vault_status", json!({})).unwrap()["initialized"], false);
    ui.core("vault_create", json!({ "masterPassword": "master" })).unwrap();
    let update = json!({
        "hosts": [{ "id": "h1", "group": "Prod", "label": "api-1", "address": "10.0.0.1", "port": 22, "user": "deploy",
                    "auth": { "type": "password" }, "tags": ["prod"], "hasPassword": false, "newPassword": "pw" }],
        "keys": [], "snippets": [], "tunnels": []
    });
    let view = ui.core("vault_put", json!({ "update": update })).unwrap();
    assert_eq!(view["hosts"][0]["hasPassword"], true);
    assert!(!view.to_string().contains("\"pw\""), "the password must not come back to the UI");
    assert!(ui.call("core_me", json!({})).unwrap().as_u64().unwrap() > 0);

    // Errors keep their kind for the UI.
    assert_eq!(ui.core("vault_unlock", json!({ "masterPassword": "x" })).map(|_| ()).err(), None, "unlocking an unlocked vault is a no-op");
    assert_eq!(ui.core("host_password", json!({ "vaultId": "personal", "hostId": "nope", "own": false })).unwrap_err()["kind"], "notFound");
    assert_eq!(ui.core("nope", json!({})).unwrap_err()["kind"], "unknownMethod");

    // What stays in the window: this computer's files.
    assert!(ui.call("local_home", json!({})).unwrap().as_str().is_some());
}
