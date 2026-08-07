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

mod audio;
mod browser;
mod contextmenu;
mod downloads;
mod export;
mod find;
mod history;
mod import;
mod memory;
mod overlay;
mod permissions;
mod profile;
mod search;
mod settings;
mod shortcuts;
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
        .plugin(shortcuts::plugin())
        .setup(|app| {
            let store = settings::SettingsState::load(app.handle());
            let auto_update = store.get().auto_update;
            app.manage(store);
            app.manage(browser::Browser::default());
            // Registered before the window is built: the first tab's page-load
            // handler records a visit, and that runs as soon as it navigates.
            app.manage(store::Store::load(app.handle()));

            browser::build(app.handle())?;

            // The window is created focused, and the focus event that would
            // normally arm the shortcuts has already been and gone by now.
            shortcuts::set_active(app.handle(), true);

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
            browser::go_home,
            browser::reload,
            browser::stop_loading,
            browser::browser_state,
            browser::open_tab,
            browser::open_window,
            browser::move_tab_to_new_window,
            browser::close_tab,
            browser::activate_tab,
            browser::set_panel,
            browser::set_find_bar,
            browser::print_page,
            browser::open_devtools,
            browser::view_source,
            browser::set_split,
            browser::toggle_fullscreen,
            browser::reset_zoom,
            browser::set_zoom,
            browser::set_tab_pinned,
            audio::set_tab_muted,
            browser::duplicate_tab,
            browser::close_other_tabs,
            browser::close_tabs_to_right,
            browser::toggle_bookmark_active,
            browser::set_bookmarks_bar,
            browser::toggle_bookmarks_bar,
            store::history,
            store::clear_history,
            store::release_history,
            store::remove_visit,
            store::bookmarks,
            store::suggest,
            browser::set_chrome_overlay,
            store::toggle_bookmark,
            store::remove_bookmark,
            store::rename_bookmark,
            store::create_folder,
            store::move_bookmark,
            store::reorder_bookmark,
            import::import_sources,
            import::import_bookmarks,
            export::export_bookmarks,
            store::downloads,
            downloads::cancel_download,
            downloads::retry_download,
            store::clear_downloads,
            store::reveal_download,
            profile::clear_site_data,
            permissions::answer_permission,
            permissions::list_permissions,
            permissions::set_permission,
            search::search_engines,
            search::add_search_engine,
            search::remove_search_engine,
            settings::get_settings,
            settings::set_auto_update,
            settings::set_search_engine,
            settings::set_homepage,
            settings::engine_homepage,
            settings::set_theme,
            settings::app_version,
            settings::zoomed_site_count,
            settings::clear_site_zoom,
            find::find_start,
            find::find_next,
            find::find_previous,
            find::find_stop,
            updater::check_for_updates,
        ])
        // `generate_context!` reads tauri.conf.json at compile time and bakes the
        // bundle identifier and asset manifest into the binary.
        .run(tauri::generate_context!())
        .expect("failed to start Brume");
}
