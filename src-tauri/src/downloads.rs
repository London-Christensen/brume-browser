//! Download progress, through WebView2's own download operation.
//!
//! # Why this is a fourth interop module
//!
//! Tauri's `on_download` reports exactly two things: a download was requested,
//! and a download finished. There is nothing in between, so the downloads panel
//! could say "Downloading..." and never anything more useful than that. The
//! comment in browser.rs saying a byte count means going to
//! `ICoreWebView2DownloadOperation` directly is what this module is.
//!
//! `BytesReceivedChanged` on that operation fires as the count moves, and the
//! operation carries the total as well. Reaching either needs the raw interface,
//! the same route find.rs, history.rs and profile.rs already take, and the
//! `unsafe` is kept in here for the same reason.
//!
//! # It runs alongside wry's handler rather than replacing it
//!
//! `add_DownloadStarting` takes any number of subscribers, and wry has already
//! registered one: that is what `on_download` is built on. Brume adds a second.
//! The existing start-and-finish recording in browser.rs therefore keeps working
//! untouched, and this module only has to supply the part that was missing.
//!
//! Nothing specifies which of the two handlers runs first, so the first progress
//! tick can arrive before the store has a row to put it in.
//! `update_download_progress` ignores a miss rather than inventing a row; the
//! event fires many more times and the next one lands.
//!
//! # Downloads are watched per tab, not per window
//!
//! The event belongs to the webview that started the download, so every content
//! webview needs its own subscription. Progress is keyed by URL, which is what
//! `finish_download` already matches on, so this introduces no second notion of
//! which download is which.

use std::cell::RefCell;
use std::collections::HashMap;

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2DownloadOperation, ICoreWebView2_4, COREWEBVIEW2_DOWNLOAD_STATE,
    COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS,
};
use webview2_com::{
    take_pwstr, BytesReceivedChangedEventHandler, DownloadStartingEventHandler,
    StateChangedEventHandler,
};
use windows_core::Interface;

thread_local! {
    /// Downloads still running, so one can be cancelled later.
    ///
    /// `ICoreWebView2DownloadOperation` is not `Send`, which is why cancelling
    /// was deferred in 0.4.0. permissions.rs settled the pattern: the operation
    /// never leaves the thread that created it, the key is a plain `String`, and
    /// the command hops back with `run_on_main_thread` carrying only the key.
    /// No `unsafe impl Send`, so there is no promise to break.
    ///
    /// Keyed by URL, the same thing `finish_download` matches on. Two downloads
    /// of one URL at once are ambiguous here exactly as they are there, and the
    /// answer is the same: it is the rare case, and cancelling the first of them
    /// is not a wrong answer.
    static RUNNING: RefCell<HashMap<String, ICoreWebView2DownloadOperation>> =
        RefCell::new(HashMap::new());
}

/// Subscribes to downloads for one tab.
///
/// Called once when a tab's content webview is created, next to `history::watch`
/// and for the same reasons. The token is dropped rather than kept: the
/// subscription lives as long as the webview, and the webview is closed with its
/// tab.
pub fn watch(app: &AppHandle, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;

            // DownloadStarting is on ICoreWebView2_4, not on the base interface.
            // A runtime old enough to lack it fails this cast, and that is the
            // right outcome: no progress bar, and everything else including the
            // download itself carries on working.
            let core: ICoreWebView2_4 = core.cast()?;

            let mut token = 0i64;
            core.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
                    if let Some(args) = args {
                        // Not cancelled, not redirected, not given a different
                        // path. Brume is recording, not gatekeeping, and the
                        // handler that decides is wry's.
                        if let Ok(operation) = args.DownloadOperation() {
                            watch_operation(&handle, &operation);
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

/// Subscribes to the byte count of one download.
///
/// Separate from `watch` because the lifetimes differ: the DownloadStarting
/// subscription belongs to the webview, this one belongs to a single transfer.
/// Its token is dropped too, since the operation is released when the download
/// ends and the subscription goes with it.
fn watch_operation(app: &AppHandle, operation: &ICoreWebView2DownloadOperation) {
    // Read once, here, rather than inside the handler. The URI does not change
    // for the life of a download, and this way the per-tick work is two integer
    // reads instead of a string allocation and a COM free.
    let Some(url) = uri_of(operation) else {
        return;
    };
    let handle = app.clone();

    // Held so `cancel_download` has something to call Cancel on. Removed again
    // when the runtime says the transfer left IN_PROGRESS, so the map holds
    // exactly the downloads a cancel button is showing for.
    RUNNING.with(|r| r.borrow_mut().insert(url.clone(), operation.clone()));

    let done_url = url.clone();
    let _ = unsafe {
        let mut state_token = 0i64;
        operation.add_StateChanged(
            &StateChangedEventHandler::create(Box::new(move |sender, _args| {
                if let Some(op) = sender.as_ref() {
                    let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
                    if op.State(&mut state).is_ok()
                        && state != COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                    {
                        RUNNING.with(|r| r.borrow_mut().remove(&done_url));
                    }
                }
                Ok(())
            })),
            &mut state_token,
        )
    };

    let _ = unsafe {
        let mut token = 0i64;
        operation.add_BytesReceivedChanged(
            &BytesReceivedChangedEventHandler::create(Box::new(move |sender, _args| {
                if let Some(operation) = sender.as_ref() {
                    let mut received = 0i64;
                    if operation.BytesReceived(&mut received).is_err() {
                        return Ok(());
                    }

                    // Read every tick rather than once at the start. A server
                    // that sent no Content-Length reports 0 here, and some
                    // report it only once the response is properly under way.
                    let mut total = 0i64;
                    let _ = operation.TotalBytesToReceive(&mut total);

                    if handle
                        .state::<crate::store::Store>()
                        .update_download_progress(&url, received, total)
                    {
                        // Every window, because the downloads list is app-wide
                        // and a panel open in another one is just as stale.
                        crate::browser::notify_downloads_everywhere(&handle);
                    }
                }
                Ok(())
            })),
            &mut token,
        )
    };
}

/// Stops a running download.
///
/// Async, then straight onto the main thread, for the reason in the `RUNNING`
/// docs: the operation cannot leave the thread that made it, so only the URL
/// crosses over.
///
/// Nothing is written to the store here. Cancelling makes the runtime finish the
/// transfer as interrupted, which fires wry's own `Finished` with
/// `success: false`, and the existing bookkeeping in browser.rs moves the row
/// to the finished list. One path for a download ending, however it ended.
#[tauri::command]
pub async fn cancel_download(app: AppHandle, url: String) -> Result<(), String> {
    app.run_on_main_thread(move || {
        // Removed as well as cancelled: the state change that would normally
        // clear it is not guaranteed to arrive if the webview is going away.
        let operation = RUNNING.with(|r| r.borrow_mut().remove(&url));
        if let Some(operation) = operation {
            unsafe {
                if let Err(e) = operation.Cancel() {
                    eprintln!("[downloads] could not cancel {url}: {e}");
                }
            }
        }
    })
    .map_err(|e| format!("Could not cancel the download: {e}"))
}

/// Starts a failed or cancelled download again, from the beginning.
///
/// Not a resume, and deliberately so: picking up a partial transfer needs the
/// half-written file tracked and range support detected, with a fallback to a
/// full retry anyway when the server has none. That is its own feature.
///
/// Re-issued by navigating the active tab, which is precisely what following the
/// original link did. The consequence is worth knowing: a URL that has since
/// started answering with a page rather than a file will navigate that tab, just
/// as it would have the first time.
#[tauri::command]
pub async fn retry_download(
    app: AppHandle,
    window: tauri::Window,
    url: String,
) -> Result<(), String> {
    crate::browser::navigate(app, window, url).await
}

/// The download's source URL, as an owned `String`.
///
/// `Uri` hands back a string the runtime allocated and the caller owns.
/// `take_pwstr` reads it and frees it, which is why this is worth its own
/// function rather than being inlined at the one call site: getting the free
/// wrong leaks on every download.
fn uri_of(operation: &ICoreWebView2DownloadOperation) -> Option<String> {
    unsafe {
        let mut raw = windows_core::PWSTR::null();
        operation.Uri(&mut raw).ok()?;
        if raw.is_null() {
            return None;
        }
        let url = take_pwstr(raw);
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    }
}
