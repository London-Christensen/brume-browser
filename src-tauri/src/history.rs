//! Back and forward, through WebView2's own session history.
//!
//! # Why Brume stopped keeping its own stack
//!
//! browser.rs used to hold a `Vec<String>` per tab and go back by *navigating*
//! to the previous URL. That worked, and it was the only option at the time:
//! nothing in Tauri or wry exposed whether a webview could go back, and a back
//! button that is always enabled is worse than no back button.
//!
//! The cost was paid on every single press. Re-navigating refetches the page,
//! loses scroll position, discards form state and re-runs whatever the page does
//! on load. `GoBack` does none of that, because it walks the back-forward cache
//! the way the browser engine intends.
//!
//! `ICoreWebView2` has had `CanGoBack`, `GoBack` and `HistoryChanged` since the
//! very first release. They were simply out of reach until find.rs established
//! that `with_webview` is a workable route to the raw interface.
//!
//! # This is the second interop module
//!
//! find.rs is the other. Both exist for the same reason, that Tauri surfaces
//! only part of WebView2, and both keep their `unsafe` confined so a breaking
//! change upstream has a small number of places to be repaired in.

use std::sync::mpsc;

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use webview2_com::{HistoryChangedEventHandler, ZoomFactorChangedEventHandler};

/// Runs `f` against a webview's `ICoreWebView2`.
///
/// Same shape and the same threading reasoning as find.rs: `with_webview` runs
/// the closure inline when already on the main thread, and the channel is how a
/// value gets back out of a closure that has to return `()`.
fn with_core<T, F>(webview: &tauri::webview::Webview, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&ICoreWebView2) -> windows_core::Result<T> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform| {
            let result = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;
                f(&core)
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Could not reach the page: {e}"))?;

    match rx.recv_timeout(std::time::Duration::from_millis(500)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(format!("History call failed: {e}")),
        Err(_) => Err("The page did not respond.".into()),
    }
}

/// Steps the webview back or forward through its own history.
///
/// Not a navigation. The engine restores the previous entry from the
/// back-forward cache, which is the whole point of the exercise.
pub fn go(webview: &tauri::webview::Webview, forward: bool) -> Result<(), String> {
    with_core(webview, move |core| unsafe {
        if forward {
            core.GoForward()
        } else {
            core.GoBack()
        }
    })
}

/// Subscribes to zoom changes for one tab.
///
/// Zoom itself is WebView2's: `zoom_hotkeys_enabled` lets it handle Ctrl+scroll
/// and Ctrl+plus/minus internally, so Brume never sets the level and only needs
/// to know what it became. Without this the toolbar could not say you were at
/// 150%, which is the whole complaint the indicator answers.
///
/// Watched on the **controller**, not the webview: zoom is a property of the
/// host's presentation of the page rather than of the document.
pub fn watch_zoom(app: &AppHandle, tab_id: u32, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();

    let _ = webview.with_webview(move |platform| {
        let controller = platform.controller();
        let _ = (|| unsafe {
            let mut token = 0i64;
            controller.add_ZoomFactorChanged(
                &ZoomFactorChangedEventHandler::create(Box::new(move |sender, _args| {
                    if let Some(controller) = sender.as_ref() {
                        let mut factor = 1.0f64;
                        if controller.ZoomFactor(&mut factor).is_ok() {
                            crate::browser::update_zoom(&handle, tab_id, factor);
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}

/// Sets the zoom level.
///
/// Goes through the controller rather than `Webview::set_zoom` so the change
/// comes from the same object the watcher reads, and ZoomFactorChanged fires
/// exactly as it would for a Ctrl+scroll. Nothing is published from here: the
/// event does that, with whatever value the runtime actually settled on.
///
/// Clamped to the range Chromium itself offers. WebView2 accepts absurd factors
/// without complaint, and a page at 5000% is a browser someone has to force-quit.
/// Returns the factor the runtime actually settled on, which is not always the
/// one asked for.
///
/// Read back rather than assumed, because **ZoomFactorChanged does not fire for
/// a programmatic SetZoomFactor** - measured, not guessed: setting 1.5 resized
/// the page correctly while the watcher stayed silent and the indicator kept
/// reading 100%. The event still covers user zoom, which is the path that
/// matters most, but this one cannot rely on it.
pub fn set_zoom(webview: &tauri::webview::Webview, factor: f64) -> Result<f64, String> {
    let clamped = factor.clamp(0.25, 5.0);
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform| {
            let controller = platform.controller();
            let result = unsafe {
                controller.SetZoomFactor(clamped).and_then(|()| {
                    let mut actual = 1.0f64;
                    controller.ZoomFactor(&mut actual)?;
                    Ok(actual)
                })
            };
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Could not reach the page: {e}"))?;

    match rx.recv_timeout(std::time::Duration::from_millis(500)) {
        Ok(Ok(actual)) => Ok(actual),
        Ok(Err(e)) => Err(format!("Could not set zoom: {e}")),
        Err(_) => Err("The page did not respond.".into()),
    }
}

/// Subscribes to history changes for one tab.
///
/// Called once when a tab's webview is created. The alternative was reading
/// CanGoBack inside `publish`, but publish runs from inside WebView2's own
/// navigation events, and calling back into the same object from one of its
/// handlers is exactly the reentrancy its docs warn about. Letting the runtime
/// say when history changed avoids the question entirely.
///
/// The token is dropped rather than kept: the subscription lives as long as the
/// webview, and the webview is closed when its tab is.
pub fn watch(app: &AppHandle, tab_id: u32, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;

            let mut token = 0i64;
            core.add_HistoryChanged(
                &HistoryChangedEventHandler::create(Box::new(move |sender, _args| {
                    if let Some(core) = sender.as_ref() {
                        let mut back = false.into();
                        let _ = core.CanGoBack(&mut back);
                        let mut forward = false.into();
                        let _ = core.CanGoForward(&mut forward);
                        crate::browser::update_traverse(
                            &handle,
                            tab_id,
                            back.as_bool(),
                            forward.as_bool(),
                        );
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}
