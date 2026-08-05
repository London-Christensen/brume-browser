//! Which tab is making noise, and muting it.
//!
//! # Why this is its own module
//!
//! `ICoreWebView2_8` carries both halves: `IsDocumentPlayingAudio` says a tab is
//! producing sound, and `IsMuted`/`SetIsMuted` silence it without pausing it.
//! Neither is surfaced by Tauri or wry, so this goes to COM like the other
//! interop modules.
//!
//! It could have gone in history.rs, which already watches zoom despite zoom not
//! being history. That drift is the argument against, not for: three unrelated
//! things behind one module name is how a file stops being findable.
//!
//! # Watched, never polled
//!
//! Both properties raise change events, so Brume mirrors them into `Tab` the
//! same way `can_back` and `zoom` are mirrored. Nothing here computes whether a
//! tab is audible; asking the runtime on a timer would be both slower and
//! wrong between ticks.
//!
//! # `IsDocumentPlayingAudio` has never been seen to report true
//!
//! Muting works and is verified: `IsMutedChanged` fires, the value reaches the
//! tab strip, and the indicator switches. The other half does not.
//!
//! Tried on 2026-08-05 against WebView2 150.0.4078.105, with the tab both muted
//! and unmuted: a Web Audio oscillator, an `<audio>` element on a `data:` WAV,
//! and an `<audio>` element on a remote `.ogg`, at volumes 0.02, 0.1 and 0.15.
//! `audible` stayed false throughout. The read itself is fine - the same
//! `publish` call reports `muted` correctly from the same interface - so either
//! the change event never fires or the runtime does not consider any of that
//! "playing audio".
//!
//! The code is left in rather than removed. It is correct against the documented
//! API, costs nothing while the event stays quiet, and starts working the day
//! the runtime reports it. What it must not do is be the *only* way to mute a
//! tab, because an indicator that never appears is a control that does not
//! exist. The tab context menu is the real entry point.
//!
//! # Muting is per tab, and it is not pausing
//!
//! A muted tab carries on playing. That is deliberate and matches every other
//! browser: muting a video you walked away from should not lose your place in
//! it, and a page that pauses itself when muted would be doing something the
//! user did not ask for.

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
use webview2_com::{IsDocumentPlayingAudioChangedEventHandler, IsMutedChangedEventHandler};
use windows_core::Interface;

/// Subscribes to a tab's audio state.
///
/// Called once when a tab's content webview is created, next to
/// `history::watch`. The tokens are dropped for the same reason: the
/// subscription lives as long as the webview, and the webview goes with its tab.
///
/// A runtime too old for `ICoreWebView2_8` fails the cast and the tab simply
/// never reports audio, which is the right outcome: no indicator, everything
/// else unaffected.
pub fn watch(app: &AppHandle, tab_id: u32, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let audible_handle = app.clone();
    let muted_handle = app.clone();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;
            let core8: ICoreWebView2_8 = core.cast()?;

            let mut audio_token = 0i64;
            core8.add_IsDocumentPlayingAudioChanged(
                &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(
                    move |sender, _args| {
                        if let Some(core) = sender.as_ref() {
                            if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
                                publish(&audible_handle, tab_id, &core8);
                            }
                        }
                        Ok(())
                    },
                )),
                &mut audio_token,
            )?;

            let mut muted_token = 0i64;
            core8.add_IsMutedChanged(
                &IsMutedChangedEventHandler::create(Box::new(move |sender, _args| {
                    if let Some(core) = sender.as_ref() {
                        if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
                            publish(&muted_handle, tab_id, &core8);
                        }
                    }
                    Ok(())
                })),
                &mut muted_token,
            )?;

            Ok::<_, windows_core::Error>(())
        })();
    });
}

/// Reads both properties and hands them to browser.rs.
///
/// Both together rather than one per event, because the strip renders them as
/// one thing: a muted tab that is playing looks different from a muted tab that
/// is silent, and an update carrying half the answer would flicker between them.
fn publish(app: &AppHandle, tab_id: u32, core8: &ICoreWebView2_8) {
    unsafe {
        let mut audible = windows_core::BOOL::from(false);
        let mut muted = windows_core::BOOL::from(false);
        if core8.IsDocumentPlayingAudio(&mut audible).is_err() {
            return;
        }
        let _ = core8.IsMuted(&mut muted);
        crate::browser::update_audio(app, tab_id, audible.as_bool(), muted.as_bool());
    }
}

/// Mutes or unmutes one tab.
///
/// Synchronous like the other interop commands: `with_webview` runs the closure
/// inline when already on the main thread, and there is no completion handler to
/// wait on. Nothing is published from here - `IsMutedChanged` fires and the
/// watcher above reports whatever the runtime actually settled on.
#[tauri::command]
pub fn set_tab_muted(app: AppHandle, id: u32, muted: bool) -> Result<(), String> {
    let label = crate::browser::tab_label(&app, id)
        .ok_or_else(|| format!("No tab with id {id}"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| "That tab has no webview.".to_string())?;

    webview
        .with_webview(move |platform| {
            let _ = (|| unsafe {
                let core8 = platform.controller().CoreWebView2()?.cast::<ICoreWebView2_8>()?;
                core8.SetIsMuted(muted)?;
                Ok::<_, windows_core::Error>(())
            })();
        })
        .map_err(|e| format!("Could not reach the tab: {e}"))
}
