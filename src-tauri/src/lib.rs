//! Thin Tauri shell: every command here only translates between the webview
//! and `detour_core`. No business logic belongs in this crate.

use serde::Serialize;

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
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![core_info, write_text_file])
        .run(tauri::generate_context!())
        .expect("error while running Detour");
}
