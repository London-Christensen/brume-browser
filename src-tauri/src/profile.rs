//! Clearing cookies, cache and site data.
//!
//! History and bookmarks are Brume's own files and store.rs deals with them.
//! Everything a *site* leaves behind lives in the WebView2 profile instead:
//! cookies, local storage, IndexedDB, the HTTP cache. None of it is reachable
//! through Tauri, so this is the third module that goes to COM directly, after
//! find.rs and history.rs.
//!
//! For a browser whose search is already stripped of ads and trackers, having no
//! way to clear a cookie from inside the application was the conspicuous gap.

use std::sync::mpsc;

use tauri::AppHandle;
use webview2_com::ClearBrowsingDataCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13};
use windows_core::Interface;

/// Whether the clear finished, or why it could not start.
type ClearResult = Result<bool, windows_core::Error>;

/// Clears every kind of browsing data the runtime holds for this profile.
///
/// `ClearBrowsingDataAll` rather than a time range: a "clear everything" control
/// that quietly kept last week's cookies would be worse than not offering one.
///
/// The call is asynchronous and reports through a completion handler, so this
/// waits for it. A control that returns before the work is done invites a second
/// press, and the honest thing is to finish before saying so.
///
/// **This command must stay `async`**, unlike the ones in find.rs and history.rs.
/// Those only read and return; this one waits for a completion handler, and that
/// handler runs on the main thread. As a sync command it ran on the main thread
/// itself and then blocked it waiting for a callback that needed it, so the
/// clear did happen but the command always reported a timeout, with the UI
/// frozen for the full 30 seconds first. Async puts the wait on a worker and
/// leaves the main thread free to deliver the callback.
///
/// Note this does not touch history or bookmarks. Those are Brume's own records
/// and are cleared from their own panel, which keeps "what the sites left" and
/// "what I chose to keep" as separate decisions.
#[tauri::command]
pub async fn clear_site_data(app: AppHandle) -> Result<(), String> {
    // Any tab will do: they all share one profile unless they are private, and
    // a private tab's partition is discarded with its webview anyway.
    let webview = crate::browser::active_content_webview(&app)?;
    // Annotated because the closure sends from two places with different error
    // origins, and inference cannot pick between the several From<Error> impls
    // that tao, wry and webview2_com all provide.
    let (tx, rx) = mpsc::channel::<ClearResult>();

    webview
        .with_webview(move |platform| {
            let started = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;
                let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
                let profile2 = profile.cast::<ICoreWebView2Profile2>()?;

                let done = tx.clone();
                profile2.ClearBrowsingDataAll(&ClearBrowsingDataCompletedHandler::create(
                    Box::new(move |hr| {
                        let _ = done.send(Ok(hr.is_ok()));
                        Ok(())
                    }),
                ))?;
                Ok(())
            })();

            // Only report a start failure; success is reported by the handler.
            if let Err(e) = started {
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| format!("Could not reach the profile: {e}"))?;

    // Longer than the other interop waits. This clears an entire profile from
    // disk, and on a well-used one that is not instant.
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err("The runtime reported the clear did not complete.".into()),
        Ok(Err(e)) => Err(format!("Could not clear site data: {e}")),
        Err(_) => Err("Clearing site data timed out.".into()),
    }
}
