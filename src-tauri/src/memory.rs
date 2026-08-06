//! Making tabs you are not looking at cost less.
//!
//! # What a tab actually costs
//!
//! Measured on 2026-08-05, attributing WebView2 processes by parent PID:
//!
//! ```text
//!   1 tab    7 processes   522 MB    browser, gpu, crashpad, 2 renderer, 2 utility
//!   +1 tab   8 processes   586 MB    +1 renderer
//!   +1 tab   9 processes   640 MB    +1 renderer
//!   +1 tab  10 processes   740 MB    +1 renderer
//! ```
//!
//! The browser, GPU, crashpad and utility processes are shared, so a tab is one
//! renderer and roughly 55 to 100 MB depending on the page. That is the same
//! shape Chrome has, and it is the number worth attacking: ten background tabs
//! is most of a gigabyte held by pages nobody is reading.
//!
//! # TrySuspend is worth much less than it looks
//!
//! This module was written expecting `ICoreWebView2_3::TrySuspend` to be the
//! answer. It is not, and the measurement is the point of this note.
//!
//! With four tabs open and three of them suspended, total resident memory went
//! from 1099 MB to 1038 MB. About 5%. Listing the processes afterwards showed
//! why: eight renderers still resident, holding 57 to 137 MB each. **Suspending
//! freezes execution and keeps the heap.**
//!
//! It is kept because freezing a background renderer is a real saving in CPU and
//! battery, it costs nothing, and it loses no state. But do not come back here
//! expecting to reclaim memory by suspending harder.
//!
//! `ICoreWebView2_19::SetMemoryUsageTargetLevel(LOW)` is applied alongside it as
//! a hint to trim, and moves the number no further.
//!
//! # Where the memory actually came from
//!
//! Not loading the tab at all. Session restore used to build a webview and load
//! a page for every saved tab, so twenty restored tabs meant twenty renderers
//! before the window was usable. browser.rs parks them instead: on the same
//! six-tab session that is 8 processes and 501 MB rather than 29 and 1982 MB,
//! with the window usable in 1.3s.
//!
//! That trick is deliberately **not** extended to tabs that have already loaded.
//! Destroying a live webview reclaims the same renderer and also destroys scroll
//! position and anything typed into a form, and a tab you glanced away from for
//! a minute is not worth that. A restored tab has nothing to lose, which is
//! exactly why it is safe there and nowhere else.
//!
//! # Not immediately, and never the active tab
//!
//! Suspending the moment a tab loses focus would make Ctrl+Tab expensive: flick
//! through five tabs and you would suspend and resume four of them for nothing.
//! The delay below is long enough that switching about is free and short enough
//! that a tab left behind stops costing.
//!
//! `TrySuspend` also refuses while a webview is visible, which is why this can
//! only ever apply to background tabs: browser.rs hides inactive ones, so the
//! precondition is met by the layout rather than by anything here.

use std::time::Duration;

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_19, ICoreWebView2_3, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
};
use webview2_com::TrySuspendCompletedHandler;
use windows_core::Interface;

/// How long a tab sits in the background before it is suspended.
///
/// Long enough that cycling tabs with Ctrl+Tab never suspends anything. Since
/// the saving is CPU rather than memory (see above), there is nothing to be won
/// by making this aggressive, and plenty to lose: a suspended tab costs a moment
/// coming back.
const IDLE_BEFORE_SUSPEND: Duration = Duration::from_secs(45);

/// Suspends a tab that has been in the background long enough.
///
/// Spawned rather than run inline, and it re-checks before acting: the tab may
/// have been activated again, or closed, in the time it waited. Acting on stale
/// state here would suspend the tab the user is currently reading.
pub fn suspend_later(app: &AppHandle, tab_id: u32) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE_BEFORE_SUSPEND).await;

        // Still in the background, and still open?
        if crate::browser::active_tab_id(&app) == Some(tab_id) {
            return;
        }
        let Some(label) = crate::browser::tab_label(&app, tab_id) else {
            return; // closed while waiting
        };
        let Some(webview) = app.get_webview(&label) else {
            return;
        };

        let _ = webview.with_webview(move |platform| {
            let _ = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;

                // The hint first. If the suspend below is refused, this at least
                // asks the renderer to give back what it can.
                if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                    let _ = core19
                        .SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
                }

                let core3: ICoreWebView2_3 = core.cast()?;
                core3.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(
                    move |hr, succeeded| {
                        // Reported only on a real error. A plain `false` is
                        // ordinary: the runtime refuses while a download is
                        // running, media is playing, or the webview is visible.
                        if hr.is_err() {
                            eprintln!("[memory] suspend failed: {hr:?} ({succeeded:?})");
                        }
                        Ok(())
                    },
                )))?;
                Ok::<_, windows_core::Error>(())
            })();
        });
    });
}

/// Brings a tab back, if it was suspended.
///
/// Called on activation. `Resume` on a webview that was never suspended is a
/// no-op, so this does not need to know which state it was in - and asking
/// first would be a second round trip to get the same answer.
pub fn resume(app: &AppHandle, tab_id: u32) {
    let Some(label) = crate::browser::tab_label(app, tab_id) else {
        return;
    };
    let Some(webview) = app.get_webview(&label) else {
        return;
    };

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;
            if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                let _ =
                    core19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
            }
            let core3: ICoreWebView2_3 = core.cast()?;
            core3.Resume()?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}
