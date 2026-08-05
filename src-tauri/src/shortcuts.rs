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
pub const OPEN_FIND_EVENT: &str = "brume://open-find";

/// Accelerator string paired with the action it triggers.
///
/// Plain strings so adding a binding is a one-line change. The plugin parses
/// them; an unparseable one is reported at registration rather than silently
/// doing nothing.
const BINDINGS: &[(&str, &str)] = &[
    ("CmdOrCtrl+T", "new_tab"),
    ("CmdOrCtrl+W", "close_tab"),
    ("CmdOrCtrl+Shift+T", "reopen_tab"),
    ("CmdOrCtrl+Tab", "next_tab"),
    ("CmdOrCtrl+Shift+Tab", "prev_tab"),
    // Ctrl+1..8 select by position; Ctrl+9 is "last tab" rather than the ninth,
    // which is the convention every mainstream browser follows.
    ("CmdOrCtrl+1", "tab_1"),
    ("CmdOrCtrl+2", "tab_2"),
    ("CmdOrCtrl+3", "tab_3"),
    ("CmdOrCtrl+4", "tab_4"),
    ("CmdOrCtrl+5", "tab_5"),
    ("CmdOrCtrl+6", "tab_6"),
    ("CmdOrCtrl+7", "tab_7"),
    ("CmdOrCtrl+8", "tab_8"),
    ("CmdOrCtrl+9", "last_tab"),
    ("CmdOrCtrl+L", "focus_address"),
    ("CmdOrCtrl+R", "reload"),
    ("F5", "reload"),
    ("Alt+Left", "back"),
    ("Alt+Right", "forward"),
    ("Alt+Home", "home"),
    ("CmdOrCtrl+D", "bookmark"),
    ("CmdOrCtrl+Shift+B", "bookmarks_bar"),
    ("CmdOrCtrl+H", "history"),
    ("CmdOrCtrl+J", "downloads"),
    ("CmdOrCtrl+Comma", "settings"),
    // Opens Settings, where the control actually lives. Brume has no separate
    // clear-data dialog and inventing one to satisfy a keystroke would be a
    // second place for the same switch to drift out of step.
    ("CmdOrCtrl+Shift+Delete", "clear_data"),
    // No Escape binding here on purpose. These are *global* shortcuts while the
    // window has focus, so registering Escape would take it away from every
    // page: no dismissing a site's own dialog, no leaving its fullscreen. The
    // find bar closes on Escape from its own keydown handler in the chrome,
    // which is where the focus already is while typing in it.
    ("CmdOrCtrl+F", "find"),
    ("CmdOrCtrl+P", "print"),
    ("F11", "fullscreen"),
    ("CmdOrCtrl+Shift+N", "private_tab"),
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
        "new_tab" => spawn(app, |app| async move {
            browser::open_tab(app, None, None).await
        }),

        "close_tab" => {
            if let Some(tab) = browser::active_tab_id(app) {
                spawn(
                    app,
                    move |app| async move { browser::close_tab(app, tab).await },
                );
            }
        }

        "reopen_tab" => spawn(
            app,
            |app| async move { browser::reopen_closed_tab(app).await },
        ),

        "private_tab" => spawn(app, |app| async move {
            browser::open_tab(app, None, Some(true)).await
        }),

        "next_tab" | "prev_tab" => {
            let forward = action == "next_tab";
            if let Some(tab) = browser::neighbour_tab_id(app, forward) {
                spawn(app, move |app| async move {
                    browser::activate_tab(app, tab).await
                });
            }
        }

        // Ctrl+1..8. The digit is parsed back out of the action name rather than
        // carried alongside it, so adding a binding stays a one-line change to
        // BINDINGS - which is the property that table exists to have.
        _ if action.starts_with("tab_") => {
            let Some(n) = action[4..].parse::<usize>().ok().filter(|n| *n >= 1) else {
                eprintln!("[shortcuts] unparseable tab index in {action}");
                return;
            };
            if let Some(tab) = browser::tab_id_at(app, n - 1) {
                spawn(app, move |app| async move {
                    browser::activate_tab(app, tab).await
                });
            }
        }

        "last_tab" => {
            if let Some(tab) = browser::last_tab_id(app) {
                spawn(app, move |app| async move {
                    browser::activate_tab(app, tab).await
                });
            }
        }

        // Owned by the chrome: it holds the address bar.
        "focus_address" => {
            let _ = app.emit_to(browser::CHROME_LABEL, FOCUS_ADDRESS_EVENT, ());
        }

        // The chrome opens the bar and focuses the field; opening it resizes a
        // webview, so the chrome calls set_find_bar rather than doing it here.
        "find" => {
            let _ = app.emit_to(browser::CHROME_LABEL, OPEN_FIND_EVENT, ());
        }

        // Spawned, not called inline. These used to be sync commands that only
        // talked to an existing webview, but each one now closes the panel first
        // so the result is actually visible, and that re-lays-out webviews.
        "reload" => spawn(app, |app| async move { browser::reload(app).await }),
        "back" => spawn(app, |app| async move { browser::go_back(app).await }),
        "forward" => spawn(app, |app| async move { browser::go_forward(app).await }),
        // Alt+Home, not Ctrl+Home. Ctrl+Home is "scroll to top of document" in
        // every browser and belongs to the page, not to us.
        "home" => spawn(app, |app| async move { browser::go_home(app).await }),
        "print" => log(browser::print_page(app.clone())),

        // Resize webviews, so they go through spawn like the tab commands.
        "fullscreen" => spawn(
            app,
            |app| async move { browser::toggle_fullscreen(app).await },
        ),
        "bookmarks_bar" => spawn(app, |app| async move {
            browser::toggle_bookmarks_bar(app).await
        }),
        "bookmark" => log(browser::toggle_bookmark_active(app.clone()).map(|_| ())),

        // The chrome decides which view to show and toggles the panel itself.
        "history" | "settings" | "downloads" => {
            let _ = app.emit_to(browser::CHROME_LABEL, OPEN_PANEL_EVENT, action.to_string());
        }
        "clear_data" => {
            let _ = app.emit_to(browser::CHROME_LABEL, OPEN_PANEL_EVENT, "settings".to_string());
        }

        other => eprintln!("[shortcuts] unhandled action: {other}"),
    }
}

fn log(result: Result<(), String>) {
    if let Err(e) = result {
        eprintln!("[shortcuts] {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_accelerator_parses() {
        // An unparseable accelerator is only reported at registration, which is
        // a runtime eprintln nobody reads. Failing the build is louder.
        for (accel, action) in BINDINGS {
            assert!(
                accel.parse::<Shortcut>().is_ok(),
                "{accel} (for {action}) is not a valid accelerator"
            );
        }
    }

    #[test]
    fn no_accelerator_is_bound_twice() {
        // action_for returns the *first* match, so a duplicate accelerator means
        // the later binding silently never fires - and the two would have to be
        // read side by side to notice.
        let mut seen = HashSet::new();
        for (accel, _) in BINDINGS {
            let parsed = accel
                .parse::<Shortcut>()
                .expect("checked by the test above");
            assert!(seen.insert(parsed), "{accel} is bound more than once");
        }
    }

    #[test]
    fn every_accelerator_resolves_to_its_own_action() {
        for (accel, action) in BINDINGS {
            let parsed = accel.parse::<Shortcut>().expect("checked above");
            assert_eq!(
                action_for(&parsed),
                Some(*action),
                "{accel} did not resolve back to {action}"
            );
        }
    }

    #[test]
    fn numbered_tab_actions_carry_a_usable_index() {
        // `handle` parses the digit back out of the action name, so a typo like
        // "tab_one" would compile, register, and then do nothing at all.
        let numbered: Vec<&str> = BINDINGS
            .iter()
            .map(|(_, action)| *action)
            .filter(|a| a.starts_with("tab_"))
            .collect();

        assert_eq!(numbered.len(), 8, "expected Ctrl+1..8");
        for action in numbered {
            let n = action[4..]
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{action} has no parseable index"));
            assert!((1..=8).contains(&n), "{action} is out of range");
        }
    }

    #[test]
    fn an_unbound_shortcut_resolves_to_nothing() {
        let unbound = "CmdOrCtrl+Shift+F12"
            .parse::<Shortcut>()
            .expect("valid accelerator");
        assert_eq!(action_for(&unbound), None);
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
