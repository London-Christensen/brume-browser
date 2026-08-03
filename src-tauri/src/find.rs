//! Find in page.
//!
//! # Why this file has unsafe in it
//!
//! WebView2 has had a real find API since runtime 1.0.2792, but neither Tauri
//! nor wry surfaces it. The only route is `Webview::with_webview`, which hands
//! back the raw `ICoreWebView2Controller`, and from there it is COM.
//!
//! The alternative was injecting JavaScript to walk the DOM and wrap matches in
//! spans. That breaks on cross-origin frames, fights with the page's own
//! styling, cannot search a PDF, and mutates a document Brume has no business
//! mutating. The native API does none of that and highlights matches the way
//! Edge does, because it *is* what Edge does.
//!
//! Every COM call is confined to this module, so a breaking change upstream has
//! one file to be repaired in. Same containment rule browser.rs follows for the
//! unstable multiwebview API.
//!
//! # Threading
//!
//! The commands here are deliberately **synchronous**, which is the opposite of
//! the rule browser.rs follows, and for a reason worth writing down.
//!
//! `with_webview` posts through `send_user_message`, which checks whether it is
//! already on the main thread. On the main thread it runs the closure *inline*
//! and returns; off it, it posts to the event loop and returns immediately
//! without waiting. A sync Tauri command already runs on the main thread, so the
//! closure has finished by the time `with_webview` returns and its result can
//! simply be read back.
//!
//! Making these async would invert that: the closure would be queued, the
//! command would return before it ran, and every result would have to be waited
//! on across a channel. Nothing here creates a webview, so none of the
//! `add_child` deadlock reasoning applies.

use std::sync::mpsc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment15, ICoreWebView2Find, ICoreWebView2_2, ICoreWebView2_28,
};
use webview2_com::FindStartCompletedHandler;
use windows_core::Interface;

/// Carries the match count to the find bar.
///
/// An event rather than a return value, because the count is not known when the
/// command returns. `Start` is asynchronous: it kicks off a document search and
/// completes later. Reading MatchCount straight after it reports 0 every time,
/// which was the first version of this file and it looked exactly like a broken
/// search.
pub const FIND_STATE_EVENT: &str = "brume://find-state";

/// What the find bar shows: "3 of 17".
///
/// Both are -1 from WebView2 until a search has actually run, so they are
/// normalised to zero here rather than leaking a sentinel into the UI.
#[derive(Serialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FindState {
    pub matches: i32,
    /// 1-based position of the highlighted match, or 0 when there is none.
    pub active: i32,
}

/// Runs `f` against the active tab's `ICoreWebView2Find`.
///
/// The whole COM dance lives here so the commands below read as intent. The
/// channel is how a value gets back out of a `FnOnce` that must return `()`;
/// see the threading note above for why it has always already been sent by the
/// time it is read.
fn with_find<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&ICoreWebView2Find) -> windows_core::Result<T> + Send + 'static,
{
    let webview = crate::browser::active_content_webview(app)?;
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform| {
            // Every step is fallible and none of it is unwrapped. A cast that
            // fails means an older runtime, which should read as "find is
            // unavailable" rather than take the browser down.
            let result = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;
                let find = core.cast::<ICoreWebView2_28>()?.Find()?;
                f(&find)
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Could not reach the page: {e}"))?;

    // recv_timeout rather than try_recv, so this works from either side.
    //
    // On the main thread the closure has already run and the value is waiting.
    // Off it, `with_webview` posted to the event loop and this blocks a worker
    // thread until the main thread gets to it, which is safe because the main
    // thread is not waiting on us.
    match rx.recv_timeout(std::time::Duration::from_millis(500)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(describe(e)),
        Err(_) => Err("The page did not respond to the find request.".into()),
    }
}

/// Turns a COM failure into something worth showing a person.
///
/// The one case worth naming is an old runtime: `ICoreWebView2_28` arrived in
/// 1.0.2792, and a machine below that gets E_NOINTERFACE from the cast, which
/// on its own reads as gibberish.
fn describe(e: windows_core::Error) -> String {
    if e.code() == windows_core::HRESULT(0x8000_4002u32 as i32) {
        return "This version of the WebView2 runtime has no find support. Update Edge WebView2.".into();
    }
    format!("Find failed: {e}")
}

fn read_state(find: &ICoreWebView2Find) -> windows_core::Result<FindState> {
    unsafe {
        let mut matches = 0i32;
        find.MatchCount(&mut matches)?;
        let mut active = 0i32;
        find.ActiveMatchIndex(&mut active)?;
        // WebView2 reports -1 for "no search running" on both.
        Ok(FindState {
            matches: matches.max(0),
            active: active.max(0),
        })
    }
}

/// Pushes the current count to the find bar.
fn emit(app: &AppHandle, state: FindState) {
    let _ = app.emit_to(crate::browser::CHROME_LABEL, FIND_STATE_EVENT, state);
}

/// Starts a search. The count arrives later on `FIND_STATE_EVENT`.
#[tauri::command]
pub fn find_start(app: AppHandle, query: String, case_sensitive: bool) -> Result<(), String> {
    if query.is_empty() {
        return find_stop(app);
    }

    // Start does not go through `with_find`, because building the options
    // object needs the environment and the Find interface does not expose it.
    let webview = crate::browser::active_content_webview(&app)?;
    let (tx, rx) = mpsc::channel();
    let emitter = app.clone();

    webview
        .with_webview(move |platform| {
            let result = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;

                let env = core.cast::<ICoreWebView2_2>()?.Environment()?;
                let options = env.cast::<ICoreWebView2Environment15>()?.CreateFindOptions()?;
                options.SetFindTerm(&windows_core::HSTRING::from(query.as_str()))?;
                options.SetIsCaseSensitive(case_sensitive)?;
                options.SetShouldHighlightAllMatches(true)?;
                // Brume draws its own find bar, so WebView2's must stay shut.
                // Without this both appear and the two disagree about state.
                options.SetSuppressDefaultFindDialog(true)?;

                let find = core.cast::<ICoreWebView2_28>()?.Find()?;

                // The count is read in the completion handler, not here.
                // `Start` returns as soon as the search is queued, so reading
                // MatchCount on this line gives 0 every time.
                let counted = find.clone();
                find.Start(
                    &options,
                    &FindStartCompletedHandler::create(Box::new(move |_hr| {
                        if let Ok(state) = read_state(&counted) {
                            emit(&emitter, state);
                        }
                        Ok(())
                    })),
                )?;
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Could not reach the page: {e}"))?;

    match rx.try_recv() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(describe(e)),
        Err(_) => Err("The page did not respond to the find request.".into()),
    }
}

/// How long to let a move settle before reading the index back.
///
/// `FindNext` and `FindPrevious` are asynchronous like `Start`, so reading
/// ActiveMatchIndex on the following line returns the index *before* the move.
/// The bar sat one press behind: Next on "1 of 2" still read "1 of 2".
///
/// The obvious fix, ActiveMatchIndexChanged, was tried and does not help. That
/// event fires before the property is updated, so a read inside the handler
/// returns the stale value too. Measured, not assumed. What does work is
/// letting it settle and then reading, which is what this is.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(60);

/// Steps to the next or previous match.
fn step(app: &AppHandle, forward: bool) -> Result<(), String> {
    with_find(app, move |find| unsafe {
        if forward {
            find.FindNext()
        } else {
            find.FindPrevious()
        }
    })?;

    // Read back off the main thread, so the wait does not stall the UI. A bare
    // thread rather than the async runtime, because the only thing needed here
    // is a sleep and pulling in a timer would mean a new dependency for it.
    //
    // This is also why `with_find` waits on a channel rather than assuming the
    // closure has already run: from this thread it genuinely has not.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        if let Ok(state) = with_find(&app, read_state) {
            emit(&app, state);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn find_next(app: AppHandle) -> Result<(), String> {
    step(&app, true)
}

#[tauri::command]
pub fn find_previous(app: AppHandle) -> Result<(), String> {
    step(&app, false)
}

/// Ends the search and clears the highlights.
#[tauri::command]
pub fn find_stop(app: AppHandle) -> Result<(), String> {
    with_find(&app, |find| unsafe { find.Stop() })?;
    emit(&app, FindState::default());
    Ok(())
}
