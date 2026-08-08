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

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::browser;

/// Sent to the chrome for actions it owns, such as the address bar caret.
pub const FOCUS_ADDRESS_EVENT: &str = "brume://focus-address";
pub const OPEN_PANEL_EVENT: &str = "brume://open-panel";
pub const OPEN_FIND_EVENT: &str = "brume://open-find";
pub const OPEN_TAB_SEARCH_EVENT: &str = "brume://open-tab-search";

/// Accelerator string paired with the action it triggers.
///
/// Plain strings so adding a binding is a one-line change. The plugin parses
/// them; an unparseable one is reported at registration rather than silently
/// doing nothing.
const BINDINGS: &[(&str, &str)] = &[
    ("CmdOrCtrl+T", "new_tab"),
    // Ctrl+N is a new window and Ctrl+Shift+N a private one, which is what
    // every browser binds them to. Ctrl+Shift+N used to be a private *tab*
    // here, because there were no windows to open; that moves to Ctrl+Shift+P
    // rather than being dropped.
    ("CmdOrCtrl+N", "new_window"),
    ("CmdOrCtrl+Shift+N", "private_window"),
    ("CmdOrCtrl+Shift+P", "private_tab"),
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
    // Named "sidebar", not "tab_sidebar". Anything starting `tab_` is claimed
    // by the numbered-tab arm, which would try to parse "sidebar" as a digit
    // and silently do nothing. That is the same trap "search_tabs" documents,
    // and it was walked into again while writing this line.
    //
    // Two of them, for the reason the DevTools block below records: Ctrl+Shift+E
    // is the natural one and was already owned by another application on the
    // machine this was written on. Ctrl+Shift+Y was measured free.
    ("CmdOrCtrl+Shift+E", "sidebar"),
    ("CmdOrCtrl+Shift+Y", "sidebar"),
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
    // Named "search_tabs", not "tab_search": anything starting with `tab_` is
    // claimed by the numbered-tab arm below, which would try to parse "search"
    // as a digit and silently do nothing. The test that counts those actions is
    // what caught it.
    ("CmdOrCtrl+Shift+A", "search_tabs"),
    ("CmdOrCtrl+P", "print"),
    ("F11", "fullscreen"),
    // Three, and the third is not redundancy for its own sake.
    //
    // F12 and Ctrl+Shift+I are what people reach for and what the Chromium
    // menus advertise, so they stay. But a global shortcut can simply be
    // unavailable: another application registers it first and the OS never
    // delivers it here. On the machine this was written on, something owns a
    // whole block of Ctrl+Shift letters - C, E, I, J, L and U were all taken -
    // which took out both conventional DevTools keys at once.
    //
    // Ctrl+Shift+K is the fallback. Firefox uses it for the web console, so it
    // is not invented, and it was measured free rather than assumed.
    //
    // Deliberately not a Ctrl+Alt combination, though several were free:
    // Ctrl+Alt is AltGr on many European layouts, so holding one globally can
    // stop a user typing a character they need.
    ("F12", "devtools"),
    ("CmdOrCtrl+Shift+I", "devtools"),
    ("CmdOrCtrl+Shift+K", "devtools"),
    ("CmdOrCtrl+U", "view_source"),
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

/// Whether the shortcuts are currently registered.
///
/// Guards against registering twice, which happened on every launch: `build`
/// creates the window already focused, so its `Focused(true)` handler arms the
/// shortcuts, and then `main` arms them again on the next line because the
/// comment there says that event has "been and gone". Both are true, and one of
/// them is redundant.
///
/// The cost was not just noise. The second pass re-registered 34 shortcuts that
/// were already held, and reported the three the machine had taken a second
/// time, so a launch printed every conflict twice.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Registers or releases the shortcuts as the window gains and loses focus.
///
/// Idempotent. With more than one window, focus moving between them fires
/// `Focused(false)` for the one leaving and `Focused(true)` for the one
/// arriving, in an order that is not guaranteed, so this has to be safe to call
/// with the same value twice.
pub fn set_active(app: &AppHandle, active: bool) {
    if ARMED.swap(active, Ordering::Relaxed) == active {
        return;
    }
    let manager = app.global_shortcut();

    if !active {
        // Released the moment focus leaves, so Brume is not holding Ctrl+T
        // hostage while the user is in another application.
        let _ = manager.unregister_all();
        return;
    }

    // Registered first, reported afterwards. What to say about a failure depends
    // on whether the same action got one of its other accelerators, and that is
    // not known until the whole pass is done. Reporting inline claimed devtools
    // had no shortcut at all while Ctrl+Shift+K was about to register fine.
    let mut failed: Vec<(&'static str, &'static str)> = Vec::new();
    let mut worked: Vec<(&'static str, &'static str)> = Vec::new();

    for (accel, action) in BINDINGS {
        match manager.register(*accel) {
            Ok(()) => worked.push((accel, action)),
            Err(e) => {
                if !is_taken(&e) {
                    // Something other than a conflict. Reported as-is, since
                    // there is no useful reframing for an unknown failure.
                    eprintln!("[shortcuts] could not register {accel} for {action}: {e}");
                    continue;
                }
                failed.push((accel, action));
            }
        }
    }

    for (accel, action) in failed {
        let alternative = worked
            .iter()
            .find(|(_, a)| a == &action)
            .map(|(alt, _)| *alt);
        report_taken(accel, action, alternative);
    }
}

/// Whether a failure is another application owning the combination.
///
/// `global-hotkey` maps Win32's `ERROR_HOTKEY_ALREADY_REGISTERED` onto its
/// `AlreadyRegistered` variant, whose Display is "HotKey already registered".
/// Printed raw that reads as Brume registering the same key twice, which would
/// be a bug here. It is not: the code is 1409 from `RegisterHotKey`, and it
/// means another application on the machine owns that combination system-wide.
/// Nothing here can reclaim it, and the key never reaches Brume at all.
///
/// Matched on the message rather than the variant, which is not ideal and is not
/// avoidable: the plugin flattens `global_hotkey::Error` into
/// `GlobalHotkey(String)` before Brume sees it, so reaching the typed variant
/// would mean depending on `global_hotkey` directly for one match. The variant
/// half is typed; only the reason is text. If a future version rewords it, the
/// worst case is the blunter message, not a wrong one.
fn is_taken(e: &tauri_plugin_global_shortcut::Error) -> bool {
    matches!(
        e,
        tauri_plugin_global_shortcut::Error::GlobalHotkey(m)
            if m.contains("already registered")
    )
}

/// Says that a combination is spoken for, and what still works instead.
///
/// Measured on 2026-08-07: F12, Ctrl+Shift+I and Ctrl+Shift+E all came back this
/// way on the development machine, along with Ctrl+Shift+C, J, L and U when
/// probed. Something there owns a block of Ctrl+Shift letters. That is why the
/// actions those keys drive carry a second accelerator.
fn report_taken(accel: &'static str, action: &str, alternative: Option<&'static str>) {
    // Once per run, per accelerator.
    //
    // The shortcuts are released and re-registered on every focus change, so a
    // conflict is rediscovered every time the user clicks away and back. Even a
    // single launch would report each one twice, because the window loses and
    // regains focus while its webviews attach. Printing on every rediscovery
    // turns one fact about the machine into a stream that reads as something
    // going wrong repeatedly.
    static REPORTED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    if !REPORTED
        .lock()
        .expect("reported shortcuts mutex poisoned")
        .insert(accel)
    {
        return;
    }

    match alternative {
        Some(alt) => eprintln!(
            "[shortcuts] {accel} is owned by another application, so {action} is on {alt} here."
        ),
        None => eprintln!(
            "[shortcuts] {accel} is owned by another application, so {action} has no keyboard \
             shortcut on this machine. Everything else still works."
        ),
    }
}

/// Runs the action behind a binding.
///
/// Every action here resolves the focused window before doing anything. A global
/// shortcut arrives with no webview attached, so without that it would act on
/// whichever window happened to be first in the map rather than the one in front.
pub fn handle(app: &AppHandle, action: &str) {
    // Nothing to act on. Possible in the moment between the last window closing
    // and the process exiting, when the accelerators are briefly still armed.
    let Some(win) = browser::focused_window(app) else {
        return;
    };

    match action {
        "new_tab" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::open_tab(app, w, None, None).await
        }),

        "new_window" => spawn(
            app,
            |app| async move { browser::open_window(app, None).await },
        ),

        "private_window" => spawn(app, |app| async move {
            browser::open_window(app, Some(true)).await
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
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::open_tab(app, w, None, Some(true)).await
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
            browser::emit_to_focused_chrome(app, FOCUS_ADDRESS_EVENT, ());
        }

        // The chrome opens the bar and focuses the field; opening it resizes a
        // webview, so the chrome calls set_find_bar rather than doing it here.
        "find" => {
            browser::emit_to_focused_chrome(app, OPEN_FIND_EVENT, ());
        }

        // Also the chrome's: it already has the tab list from brume://state, so
        // searching it needs nothing from here beyond the keystroke.
        "search_tabs" => {
            browser::emit_to_focused_chrome(app, OPEN_TAB_SEARCH_EVENT, ());
        }

        // Spawned, not called inline. These used to be sync commands that only
        // talked to an existing webview, but each one now closes the panel first
        // so the result is actually visible, and that re-lays-out webviews.
        "reload" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::reload(app, w).await
        }),
        "back" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::go_back(app, w).await
        }),
        "forward" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::go_forward(app, w).await
        }),
        // Alt+Home, not Ctrl+Home. Ctrl+Home is "scroll to top of document" in
        // every browser and belongs to the page, not to us.
        "home" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::go_home(app, w).await
        }),
        "print" => log(browser::print_page(app.clone(), win)),
        "devtools" => log(browser::open_devtools(app.clone(), win)),
        "view_source" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::view_source(app, w).await
        }),

        // Resize webviews, so they go through spawn like the tab commands.
        "fullscreen" => spawn(app, |app| async move {
            let Some(w) = browser::focused_window(&app) else {
                return Ok(());
            };
            browser::toggle_fullscreen(app, w).await
        }),
        "bookmarks_bar" => spawn(app, |app| async move {
            browser::toggle_bookmarks_bar(app).await
        }),
        "sidebar" => spawn(
            app,
            |app| async move { browser::toggle_tab_sidebar(app).await },
        ),
        "bookmark" => log(browser::toggle_bookmark_active(app.clone(), win).map(|_| ())),

        // The chrome decides which view to show and toggles the panel itself.
        "history" | "settings" | "downloads" => {
            browser::emit_to_focused_chrome(app, OPEN_PANEL_EVENT, action.to_string());
        }
        "clear_data" => {
            browser::emit_to_focused_chrome(app, OPEN_PANEL_EVENT, "settings".to_string());
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
    fn actions_that_lose_their_key_to_other_software_keep_a_spare() {
        // Not redundancy to be tidied away. A global shortcut can simply be
        // unavailable: another application registers it first and the OS never
        // delivers it here. Both conventional DevTools keys went that way on the
        // machine this was written on, along with Ctrl+Shift+E for the sidebar,
        // so each of those actions carries a second accelerator that was
        // measured free. Removing one would leave the action unreachable by
        // keyboard on any machine with the same software installed.
        for action in ["devtools", "sidebar", "reload"] {
            let count = BINDINGS.iter().filter(|(_, a)| *a == action).count();
            assert!(
                count >= 2,
                "{action} should keep more than one accelerator, has {count}"
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
