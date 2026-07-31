// Brume - application entry point.
//
// The window is built in Rust rather than declared in tauri.conf.json, because
// it is not one webview but two: the chrome on top and the page beneath it.
// Config can only describe a window with a single webview, so `browser::build`
// owns the window's shape and this file owns the wiring.

// Suppress the console window that Windows would otherwise open behind the app.
//
// This is conditional on the build profile on purpose: in debug builds the
// console is kept so that `println!`, `dbg!` and panic backtraces remain
// visible while developing. In release builds it is hidden, because an end user
// launching a browser should not get a stray black terminal window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod search;
mod settings;
mod store;
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
            app.manage(browser::Browser::default());
            // Registered before the window is built: the first tab's page-load
            // handler records a visit, and that runs as soon as it navigates.
            app.manage(store::Store::load(app.handle()));

            browser::build(app.handle())?;

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
            browser::navigate,
            browser::go_back,
            browser::go_forward,
            browser::reload,
            browser::stop_loading,
            browser::browser_state,
            browser::open_tab,
            browser::close_tab,
            browser::activate_tab,
            browser::set_panel,
            browser::toggle_bookmark_active,
            store::history,
            store::clear_history,
            store::bookmarks,
            store::toggle_bookmark,
            store::remove_bookmark,
            search::search_engines,
            settings::get_settings,
            settings::set_auto_update,
            updater::check_for_updates,
        ])
        // `generate_context!` reads tauri.conf.json at compile time and bakes the
        // bundle identifier and asset manifest into the binary.
        .run(tauri::generate_context!())
        .expect("failed to start Brume");
}
