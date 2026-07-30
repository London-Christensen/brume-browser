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

fn main() {
    tauri::Builder::default()
        // `generate_context!` reads tauri.conf.json at compile time and bakes the
        // window definitions, bundle identifier and asset manifest into the binary.
        .run(tauri::generate_context!())
        .expect("failed to start Brume");
}
