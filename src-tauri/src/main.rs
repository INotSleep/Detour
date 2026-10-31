// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `detour-desktop --core` is the Detour core, started by the window when none runs.
    if detour_app_lib::run_core_if_asked() {
        return;
    }
    detour_app_lib::run()
}
