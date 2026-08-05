//! Raising the chrome above the page.
//!
//! # The z-order is not what you would assume
//!
//! Brume's window holds sibling child webviews, and their Win32 z-order does not
//! follow creation order in the direction that would be convenient. Measured on
//! 2026-08-05 by walking `GetTopWindow` and `GW_HWNDNEXT` over the window's
//! children, with the page deliberately left visible under a full-window chrome:
//!
//! ```text
//!   0. WRY_WEBVIEW  1920x981    <- the content webview, TOPMOST
//!   1. WRY_WEBVIEW  1920x1057   <- the chrome, underneath
//! ```
//!
//! So the chrome cannot cover a page by being made bigger. That is why the panel
//! hides every content webview instead of simply drawing over them: hiding is
//! the only thing that works, not a stylistic choice.
//!
//! # Why an overlay needs something better than hiding
//!
//! The panel can get away with it because opening History is a deliberate move
//! away from the page. An address bar dropdown is not: making the page vanish on
//! every keystroke would be far worse than the problem it solves. Growing the
//! chrome the way the find bar does is no better, because that reflows the page
//! down and back on every character typed.
//!
//! So the chrome is raised above the page for as long as an overlay is open and
//! dropped back afterwards. The page is never moved or hidden; the overlay simply
//! covers the top of it, which is what a dropdown is supposed to do.
//!
//! # Contained on purpose
//!
//! This is the only place in the browser that reaches Win32 window management
//! rather than WebView2, so the `unsafe` and the one `SetWindowPos` call live
//! here and nowhere else.

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_BOTTOM, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

/// Puts the chrome above or below the content webviews.
///
/// Best effort throughout. Failing to reorder leaves a dropdown drawn behind the
/// page, which is a visual bug; failing loudly here would take the address bar
/// down with it, which is worse.
///
/// `SWP_NOACTIVATE` matters: without it, raising the chrome steals focus from
/// whatever had it, and the thing that has focus while this runs is the address
/// bar the user is typing into.
pub fn set_chrome_on_top(app: &AppHandle, on_top: bool) {
    let Some(chrome) = app.get_webview(crate::browser::CHROME_LABEL) else {
        return;
    };

    let _ = chrome.with_webview(move |platform| {
        let _ = (|| unsafe {
            // The HWND hosting this webview. wry reaches for it the same way,
            // which is the only evidence available that it is the intended route.
            let mut hwnd = HWND::default();
            platform.controller().ParentWindow(&mut hwnd)?;
            if hwnd.is_invalid() {
                return Ok(());
            }

            // HWND_BOTTOM rather than remembering the previous position: the
            // window has exactly two kinds of child, and the resting order is
            // chrome underneath. Restoring "wherever it was" would need state
            // that could go stale the moment a tab is opened.
            let after = if on_top { HWND_TOP } else { HWND_BOTTOM };
            SetWindowPos(
                hwnd,
                Some(after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}
