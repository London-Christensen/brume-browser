//! Keyboard shortcuts.
//!
//! # Why not a keydown listener
//!
//! A keystroke goes to whichever webview has focus. While the user is reading a
//! page that is the *content* webview, which is deliberately outside every
//! capability and cannot call a Brume command. A `keydown` handler in the chrome
//! therefore never fires during most of the time the browser is in use - exactly
//! when Ctrl+T needs to work.
//!
//! # Why not a menu accelerator table
//!
//! This was tried first, because it is the platform's own mechanism and would
//! have needed no extra dependency. **It does not work in this stack.** muda
//! builds an accelerator table and exposes `haccel()`, but translating it
//! requires someone to call `TranslateAcceleratorW` in the message loop, and
//! nothing does: not tao, not wry, not tauri-runtime-wry. Verified by grepping
//! all three for a non-doc call site, and then by pressing Ctrl+T at a focused
//! window with a menu attached and watching nothing happen.
//!
//! Do not reintroduce the menu approach without first checking whether that call
//! has appeared upstream.
//!
//! # What is used instead
//!
//! Global shortcuts, registered when the window gains focus and released when it
//! loses it. "Global" is doing less work than it sounds: while Brume is focused,
//! no other application is receiving keystrokes anyway, so the practical effect
//! is an application shortcut. The focus gating is what keeps Ctrl+T from being
//! stolen from every other program on the machine.
//!
//! # Anything that touches a webview must be spawned
//!
//! Shortcut handlers run on the main thread, and `add_child` blocks its caller
//! while dispatching to the main thread - so calling it here deadlocks exactly as
//! it would from a synchronous command. See BUILD_NOTES.md.

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::browser;

/// Sent to the chrome for actions it owns, such as the address bar caret.
pub const FOCUS_ADDRESS_EVENT: &str = "brume://focus-address";
pub const OPEN_PANEL_EVENT: &str = "brume://open-panel";

/// Accelerator string paired with the action it triggers.
///
/// Plain strings so adding a binding is a one-line change. The plugin parses
/// them; an unparseable one is reported at registration rather than silently
/// doing nothing.
const BINDINGS: &[(&str, &str)] = &[
    ("CmdOrCtrl+T", "new_tab"),
    ("CmdOrCtrl+W", "close_tab"),
    ("CmdOrCtrl+Tab", "next_tab"),
    ("CmdOrCtrl+Shift+Tab", "prev_tab"),
    ("CmdOrCtrl+L", "focus_address"),
    ("CmdOrCtrl+R", "reload"),
    ("F5", "reload"),
    ("Alt+Left", "back"),
    ("Alt+Right", "forward"),
    ("CmdOrCtrl+D", "bookmark"),
    ("CmdOrCtrl+H", "history"),
    ("CmdOrCtrl+Comma", "settings"),
];

fn action_for(shortcut: &Shortcut) -> Option<&'static str> {
    BINDINGS.iter().find_map(|(accel, action)| {
        accel
            .parse::<Shortcut>()
            .ok()
            .filter(|parsed| parsed == shortcut)
            .map(|_| *action)
    })
}

/// Builds the plugin, wiring the handler that dispatches shortcuts.
///
/// Concrete in `Wry` rather than generic over the runtime, because the plugin's
/// builder is itself Wry-specific. Brume only ever targets desktop WebView2, so
/// the generality would be fictional anyway.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            // Fires on both press and release; acting on both would run every
            // action twice.
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if let Some(action) = action_for(shortcut) {
                handle(app, action);
            }
        })
        .build()
}

/// Registers or releases the shortcuts as the window gains and loses focus.
pub fn set_active(app: &AppHandle, active: bool) {
    let manager = app.global_shortcut();

    if !active {
        // Released the moment focus leaves, so Brume is not holding Ctrl+T
        // hostage while the user is in another application.
        let _ = manager.unregister_all();
        return;
    }

    for (accel, action) in BINDINGS {
        if let Err(e) = manager.register(*accel) {
            // Worth reporting: a shortcut that silently fails to register looks
            // identical to one whose action is broken.
            eprintln!("[shortcuts] could not register {accel} for {action}: {e}");
        }
    }
}

/// Runs the action behind a binding.
pub fn handle(app: &AppHandle, action: &str) {
    match action {
        "new_tab" => spawn(app, |app| async move { browser::open_tab(app, None).await }),

        "close_tab" => {
            if let Some(tab) = browser::active_tab_id(app) {
                spawn(app, move |app| async move { browser::close_tab(app, tab).await });
            }
        }

        "next_tab" | "prev_tab" => {
            let forward = action == "next_tab";
            if let Some(tab) = browser::neighbour_tab_id(app, forward) {
                spawn(app, move |app| async move { browser::activate_tab(app, tab).await });
            }
        }

        // Owned by the chrome: it holds the address bar.
        "focus_address" => {
            let _ = app.emit_to(browser::CHROME_LABEL, FOCUS_ADDRESS_EVENT, ());
        }

        // These only talk to a webview that already exists, so they are safe to
        // run directly.
        "reload" => log(browser::reload(app.clone())),
        "back" => log(browser::go_back(app.clone())),
        "forward" => log(browser::go_forward(app.clone())),
        "bookmark" => log(browser::toggle_bookmark_active(app.clone()).map(|_| ())),

        // The chrome decides which view to show and toggles the panel itself.
        "history" | "settings" => {
            let _ = app.emit_to(browser::CHROME_LABEL, OPEN_PANEL_EVENT, action.to_string());
        }

        other => eprintln!("[shortcuts] unhandled action: {other}"),
    }
}

fn log(result: Result<(), String>) {
    if let Err(e) = result {
        eprintln!("[shortcuts] {e}");
    }
}

/// Spawns a webview-touching action off the main thread. See the module docs.
fn spawn<F, Fut>(app: &AppHandle, action: F)
where
    F: FnOnce(AppHandle) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send,
{
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = action(handle).await {
            eprintln!("[shortcuts] {e}");
        }
    });
}
