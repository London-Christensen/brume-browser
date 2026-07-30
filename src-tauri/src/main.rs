// Brume - application entry point.
//
// At this stage Brume is a bare shell: a single window pointed at a hardcoded
// page, which exists to prove the WebView2 rendering path works end to end
// before any browser chrome is built on top of it.
//
// The window URL is configured declaratively in `tauri.conf.json` rather than
// here, so that this file stays the place where *behaviour* lives and the
// config stays the place where *shape* lives.

// Suppress the console window that Windows would otherwise open behind the app.
//
// This is conditional on the build profile on purpose: in debug builds the
// console is kept so that `println!`, `dbg!` and panic backtraces remain
// visible while developing. In release builds it is hidden, because an end user
// launching a browser should not get a stray black terminal window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod updater;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        // Three plugins, one feature between them: updater fetches and verifies
        // the release, dialog asks the user before anything is installed, and
        // process performs the relaunch afterwards.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let store = settings::SettingsState::load(app.handle());
            let auto_update = store.get().auto_update;
            app.manage(store);

            if auto_update {
                // Spawned rather than awaited: setup runs before the window is
                // shown, and a browser should never wait on a network round
                // trip to open. If an update is found the prompt simply appears
                // a moment later, over a window that is already usable.
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(updater::run_launch_check(handle));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::set_auto_update,
            updater::check_for_updates,
        ])
        // `generate_context!` reads tauri.conf.json at compile time and bakes the
        // window definitions, bundle identifier and asset manifest into the binary.
        .run(tauri::generate_context!())
        .expect("failed to start Brume");
}
