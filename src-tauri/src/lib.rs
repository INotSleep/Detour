//! Thin Tauri shell: what belongs to the window (dialogs, other apps, this
//! computer's files) is here; everything else is the Detour core, a process of
//! its own (`detour-daemon`) that this shell starts when needed and forwards to.

use std::path::PathBuf;
use std::sync::Arc;

use detour_core::sftp::{self, FileEntry};
use detour_core::{Error, Result};
use detour_ipc::{Client, StreamItem};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, State};

/// The connection to the core, made on first use and again after the core went away.
struct Core {
    data_dir: PathBuf,
    link: tokio::sync::Mutex<Option<Link>>,
}

#[derive(Clone)]
struct Link {
    client: Client,
    /// This window's connection id: events it caused itself are not sent back to it.
    me: u64,
}

impl Core {
    async fn link<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) -> std::result::Result<Link, Value> {
        let mut link = self.link.lock().await;
        if let Some(l) = link.as_ref().filter(|l| !l.client.is_closed()) {
            return Ok(l.clone());
        }
        let start = || detour_ipc::connect_or_start_with(&self.data_dir, || start_core(&self.data_dir));
        let io = |e: std::io::Error| json!({ "kind": "io", "message": e.to_string() });
        let mut client = start().await.map_err(io)?;
        let mut hello = client.call("hello", json!({ "client": "desktop" })).await?;
        // A core of another version (the app was just updated) is replaced when nothing runs in it.
        if hello["coreVersion"] != json!(detour_core::VERSION) && client.call("sessions_list", json!({})).await?.as_array().is_some_and(|l| l.is_empty()) && client.call("tunnels_status", json!({})).await?.as_array().is_some_and(|l| l.is_empty()) {
            let _ = client.call("quit", json!({})).await;
            client.closed().await;
            client = start().await.map_err(io)?;
            hello = client.call("hello", json!({ "client": "desktop" })).await?;
        }
        let me = hello["connId"].as_u64().unwrap_or(0);
        // The core's events reach the webview, except those this window caused.
        let mut events = client.events();
        let (app2, c2) = (app.clone(), client.clone());
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) if ev.data.get("by").and_then(Value::as_u64) == Some(me) => {}
                        Ok(ev) => { let _ = app2.emit("core-event", json!({ "name": ev.name, "data": ev.data })); }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                    _ = c2.closed() => break,
                }
            }
            let _ = app2.emit("core-event", json!({ "name": "disconnected", "data": null }));
        });
        let l = Link { client, me };
        *link = Some(l.clone());
        Ok(l)
    }
}

/// Starts the core from this very program (`detour-desktop --core`), so the window works
/// without the command line installed. An AppImage starts itself again: its mount ends with the window.
fn start_core(data_dir: &std::path::Path) -> std::io::Result<()> {
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => PathBuf::from(p),
        None => detour_ipc::runnable_copy(&std::env::current_exe()?)?,
    };
    // The core trusts the window when it is this program, not the copy it runs from.
    let app = std::env::current_exe()?;
    detour_ipc::start_process(&exe, &["--core".into(), "--data-dir".into(), data_dir.as_os_str().to_owned(), "--app".into(), app.into_os_string()])
}

/// Runs the core instead of the window when started with `--core [--data-dir DIR]`.
pub fn run_core_if_asked() -> bool {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|a| a != "--core") {
        return false;
    }
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let dir = after("--data-dir").unwrap_or_else(data_dir);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _ = rt.block_on(detour_daemon::run(&dir, after("--app")));
    true
}

/// Calls a method of the core with the arguments the UI gives it.
#[tauri::command]
async fn core_call<R: tauri::Runtime>(app: tauri::AppHandle<R>, core: State<'_, Arc<Core>>, method: String, params: Value) -> std::result::Result<Value, Value> {
    core.link(&app).await?.client.call(&method, params).await
}

/// Calls a method that streams: what the stream carries goes to `channel` (raw bytes
/// as they are, JSON as JSON). A terminal's stream lasts until the session closes,
/// a transfer's until the call returns.
#[tauri::command]
async fn core_stream<R: tauri::Runtime>(app: tauri::AppHandle<R>, core: State<'_, Arc<Core>>, method: String, mut params: Value, channel: Channel<InvokeResponseBody>) -> std::result::Result<Value, Value> {
    let link = core.link(&app).await?;
    let (stream, mut rx) = link.client.stream();
    params["stream"] = json!(stream);
    let lasting = method == "session_open" || method == "session_attach";
    let client = link.client.clone();
    let session = params["sessionId"].clone();
    let forward = tauri::async_runtime::spawn(async move {
        let mut ended = false;
        while let Some(item) = rx.recv().await {
            let (sent, end) = match item {
                StreamItem::Data(bytes) => (channel.send(InvokeResponseBody::Raw(bytes)), false),
                StreamItem::Json(v) => (channel.send(InvokeResponseBody::Json(v.to_string())), v.get("closed").is_some()),
            };
            if end {
                ended = true;
                break;
            }
            // The webview stopped listening (the tab is gone): the core stops sending.
            if sent.is_err() {
                if !session.is_null() {
                    let _ = client.call("session_detach", json!({ "sessionId": session, "stream": stream })).await;
                }
                ended = true;
                break;
            }
        }
        // The core went away mid-session: the terminal hears that it closed.
        if !ended && lasting {
            let _ = channel.send(InvokeResponseBody::Json(json!({ "closed": true, "exitStatus": null }).to_string()));
        }
        client.drop_stream(stream);
    });
    let result = link.client.call(&method, params).await;
    if !lasting || result.is_err() {
        forward.abort();
        link.client.drop_stream(stream);
    }
    result
}

/// This window's connection id to the core.
#[tauri::command]
async fn core_me<R: tauri::Runtime>(app: tauri::AppHandle<R>, core: State<'_, Arc<Core>>) -> std::result::Result<u64, Value> {
    Ok(core.link(&app).await?.me)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInfo {
    core_version: &'static str,
    os: &'static str,
}

#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo { core_version: detour_core::VERSION, os: std::env::consts::OS }
}

/// Writes a file the user picked in the native save dialog (key export).
#[tauri::command]
fn write_text_file(path: String, contents: String) -> std::result::Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/* ---------- opening files in other apps ---------- */

/// Opens a file on this computer with the app the system uses for it.
#[tauri::command]
fn open_local(path: String) -> Result<()> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| Error::Io { message: e.to_string() })
}

/// A program that can edit files, as this computer has it.
#[derive(Serialize)]
struct Editor {
    name: String,
    /// The program, or on macOS the app bundle.
    path: String,
}

/// Editors installed here, the common ones in their usual places.
#[tauri::command]
fn editors() -> Vec<Editor> {
    let mut found: Vec<(&str, std::path::PathBuf)> = Vec::new();
    #[cfg(windows)]
    {
        let env = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
        let (local, pf, pf86, win) = (env("LOCALAPPDATA"), env("ProgramFiles"), env("ProgramFiles(x86)"), env("SystemRoot"));
        let under = |base: &Option<std::path::PathBuf>, rel: &str| base.as_ref().map(|b| b.join(rel));
        let candidates = [
            ("VS Code", under(&local, r"Programs\Microsoft VS Code\Code.exe")),
            ("VS Code", under(&pf, r"Microsoft VS Code\Code.exe")),
            ("Cursor", under(&local, r"Programs\cursor\Cursor.exe")),
            ("Notepad++", under(&pf, r"Notepad++\notepad++.exe")),
            ("Notepad++", under(&pf86, r"Notepad++\notepad++.exe")),
            ("Sublime Text", under(&pf, r"Sublime Text\sublime_text.exe")),
            ("Notepad", under(&win, r"System32\notepad.exe")),
        ];
        for (name, path) in candidates {
            if let Some(p) = path.filter(|p| p.is_file()) {
                found.push((name, p));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for (name, app) in [
            ("VS Code", "/Applications/Visual Studio Code.app"),
            ("Cursor", "/Applications/Cursor.app"),
            ("Sublime Text", "/Applications/Sublime Text.app"),
            ("BBEdit", "/Applications/BBEdit.app"),
            ("TextEdit", "/System/Applications/TextEdit.app"),
        ] {
            if std::path::Path::new(app).exists() {
                found.push((name, app.into()));
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        for (name, bin) in [("VS Code", "code"), ("Cursor", "cursor"), ("Sublime Text", "subl"), ("Kate", "kate"), ("Text Editor", "gnome-text-editor"), ("gedit", "gedit"), ("Mousepad", "mousepad")] {
            if let Some(p) = dirs.iter().map(|d| d.join(bin)).find(|p| p.is_file()) {
                found.push((name, p));
            }
        }
    }
    let mut out: Vec<Editor> = Vec::new();
    for (name, path) in found {
        if !out.iter().any(|e| e.name == name) {
            out.push(Editor { name: name.into(), path: path.display().to_string() });
        }
    }
    out
}

/// Opens a file on this computer in a given program.
#[tauri::command]
fn open_with(path: String, program: String) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&program);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new(&program);
    cmd.arg(&path).spawn().map(|_| ()).map_err(|e| Error::Io { message: format!("{program}: {e}") })
}

/// A fresh place in the temp folder for the copy of a server's file being edited.
#[tauri::command]
fn edit_copy_path(name: String) -> Result<String> {
    let safe: String = name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join("detour-edit").join(format!("{stamp:x}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(safe).display().to_string())
}

/// When a file on this computer last changed (Unix ms); None once it is gone.
#[tauri::command]
fn local_modified(path: String) -> Option<i64> {
    let t = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64)
}

/// This person's home folder, where the side for this computer starts.
#[tauri::command]
fn local_home() -> String {
    sftp::local_home()
}

/// A folder on this computer; "" lists the drives.
#[tauri::command]
async fn local_list(path: String) -> Result<Vec<FileEntry>> {
    tauri::async_runtime::spawn_blocking(move || sftp::local_list(&path)).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

#[tauri::command]
fn local_mkdir(path: String) -> Result<()> {
    Ok(std::fs::create_dir(path)?)
}

/// `DETOUR_DATA_DIR` overrides where the vault lives (tests, portable installs).
fn data_dir() -> PathBuf {
    std::env::var_os("DETOUR_DATA_DIR").map(Into::into).unwrap_or_else(detour_core::store::Store::default_dir)
}

/// The app's commands and state, shared by the real window and the IPC tests.
#[doc(hidden)]
pub fn builder<R: tauri::Runtime>(builder: tauri::Builder<R>, data_dir: PathBuf) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(Core { data_dir, link: tokio::sync::Mutex::new(None) }))
        .invoke_handler(tauri::generate_handler![
            core_call,
            core_stream,
            core_me,
            core_info,
            write_text_file,
            local_home,
            local_list,
            local_mkdir,
            open_local,
            editors,
            open_with,
            edit_copy_path,
            local_modified,
        ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The updater reads its key and address from tauri.conf.json, which test builds don't have.
    builder(tauri::Builder::default().plugin(tauri_plugin_updater::Builder::new().build()), data_dir())
        .run(tauri::generate_context!())
        .expect("error while running Detour");
}
