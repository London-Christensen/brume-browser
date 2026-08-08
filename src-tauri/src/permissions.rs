//! Site permissions: camera, microphone, location and the rest.
//!
//! # What this replaces
//!
//! Nothing, which was the problem. Brume never handled `PermissionRequested`, so
//! every decision was WebView2's default: its own prompt, in its own styling,
//! with no record Brume could show you and no way to change your mind later. For
//! a browser that leads with privacy, having no opinion about the camera is a
//! strange place to have no opinion.
//!
//! # The runtime does the remembering
//!
//! `ICoreWebView2Profile4::SetPermissionState` persists a decision against an
//! origin, and once set the runtime stops raising `PermissionRequested` for it
//! entirely. So Brume stores nothing of its own here: no permissions.json, no
//! cache to fall out of step with what the engine actually enforces, and
//! `GetNonDefaultPermissionSettings` reads back exactly what is in force.
//!
//! That is worth more than it sounds. A list Brume kept itself could say
//! "blocked" while the engine allowed it, and the user would have no way to tell
//! which one was lying.
//!
//! # The deferral never leaves the main thread
//!
//! A prompt has to be answered by a human, so the event is deferred rather than
//! answered inline. The awkward part is that `ICoreWebView2Deferral` and the
//! event args are COM objects and are **not `Send`**: they cannot be parked in
//! the usual `Mutex<HashMap<..>>` next to the rest of Brume's state.
//!
//! The common workaround is to assert `unsafe impl Send` and promise to behave.
//! This does not. Pending requests live in a `thread_local!` on the main thread,
//! keyed by a plain integer, and `answer` hops back with
//! `run_on_main_thread` carrying only that integer and a bool. Nothing that
//! crosses a thread boundary is a COM pointer, so there is no promise to break.
//!
//! This is the same trap `downloads.rs` ran into from the other side, where
//! holding `ICoreWebView2DownloadOperation` to cancel it later was deferred for
//! exactly this reason. The pattern here is the answer to that one too.
//!
//! # This is the fifth interop module
//!
//! find.rs, history.rs, profile.rs and downloads.rs are the others.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Deferral, ICoreWebView2PermissionRequestedEventArgs, COREWEBVIEW2_PERMISSION_KIND,
    COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
    COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ, COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
    COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS,
    COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT, COREWEBVIEW2_PERMISSION_STATE,
    COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
    COREWEBVIEW2_PERMISSION_STATE_DENY,
};
use webview2_com::{take_pwstr, PermissionRequestedEventHandler};
use windows_core::Interface;

/// Tells the chrome a site is asking for something.
pub const PERMISSION_EVENT: &str = "brume://permission";

/// One outstanding request, waiting on a human.
///
/// Never leaves the thread it was created on. See the module docs.
struct Pending {
    args: ICoreWebView2PermissionRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
    origin: String,
    kind: COREWEBVIEW2_PERMISSION_KIND,
}

thread_local! {
    /// Requests the user has not answered yet, keyed by the id handed to the
    /// chrome. `RefCell` rather than a lock, because there is exactly one thread
    /// that ever touches this and a lock would imply otherwise.
    static PENDING: RefCell<HashMap<u64, Pending>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What the chrome is told when a site asks for something.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub id: u64,
    /// The asking origin, already trimmed to scheme and host for display.
    pub origin: String,
    /// A stable slug: "camera", "microphone", and so on.
    pub kind: &'static str,
    /// Whether the page asked because someone clicked something.
    ///
    /// Passed through rather than acted on. A request nobody initiated is more
    /// likely to be a site chancing its arm, and the prompt says so.
    pub user_initiated: bool,
}

/// The runtime's enum as a slug the chrome and the settings list can both use.
///
/// Deliberately exhaustive over the kinds this build knows, with a fallback: a
/// runtime newer than this one can raise a kind that did not exist when this was
/// written, and "something" is a better prompt than a crash.
fn kind_slug(kind: COREWEBVIEW2_PERMISSION_KIND) -> &'static str {
    match kind {
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => "microphone",
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => "camera",
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => "location",
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => "notifications",
        COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => "sensors",
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => "clipboard",
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS => "downloads",
        COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS => "fonts",
        COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT => "windows",
        COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY => "autoplay",
        _ => "other",
    }
}

/// Scheme and host only.
///
/// A permission belongs to an origin, not to a page: granting the camera to one
/// article on a site grants it to the site. Showing the full URL in the prompt
/// would suggest otherwise.
fn origin_of(url: &str) -> String {
    let origin = crate::siterules::origin_of(url);
    if origin.is_empty() {
        url.to_string()
    } else {
        origin
    }
}

/// Subscribes to permission requests for one tab.
///
/// Called once per content webview, next to `history::watch`. The token is
/// dropped for the same reason: the subscription lives as long as the webview.
pub fn watch(app: &AppHandle, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();
    // Kept so the prompt reaches the window this tab is actually in. A request
    // from a background window raising its prompt over the foreground one would
    // be asking about a page the user cannot see.
    let tab_label = label.to_string();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;

            let mut token = 0i64;
            core.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };

                    let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                    args.PermissionKind(&mut kind)?;

                    let mut raw = windows_core::PWSTR::null();
                    args.Uri(&mut raw)?;
                    let uri = if raw.is_null() {
                        String::new()
                    } else {
                        take_pwstr(raw)
                    };

                    let mut user_initiated = windows_core::BOOL::from(false);
                    let _ = args.IsUserInitiated(&mut user_initiated);

                    // Deferred, because the answer comes from a person. Without
                    // this the handler would have to decide now, and deciding
                    // now can only mean guessing.
                    let deferral = args.GetDeferral()?;

                    // Denied up front, then overwritten when the answer arrives.
                    // If anything below fails, or the window is closed with the
                    // prompt still up, the request resolves as denied rather
                    // than as whatever the runtime felt like.
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;

                    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                    let origin = origin_of(&uri);

                    PENDING.with(|p| {
                        p.borrow_mut().insert(
                            id,
                            Pending {
                                args: args.clone(),
                                deferral,
                                origin: origin.clone(),
                                kind,
                            },
                        )
                    });

                    crate::browser::emit_to_tab_chrome(
                        &handle,
                        &tab_label,
                        PERMISSION_EVENT,
                        PermissionRequest {
                            id,
                            origin,
                            kind: kind_slug(kind),
                            user_initiated: user_initiated.as_bool(),
                        },
                    );
                    Ok(())
                })),
                &mut token,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}

/// Answers an outstanding request.
///
/// `allow` decides this request. `remember` decides whether it sticks.
///
/// # `remember` works backwards, and that is the runtime's doing
///
/// Setting `State` to ALLOW or DENY on the event args **already persists it**.
/// Measured, not assumed: answering with `remember: false` and then reading
/// `GetNonDefaultPermissionSettings` came back with the decision sitting there
/// in the profile. The documentation only advertises `SetPermissionState` as
/// "a persistent version of the State property", which reads as though the
/// event-args version is not persistent. It is.
///
/// So there is nothing to do to remember an answer, and forgetting one takes an
/// extra call: after completing the deferral, the setting is put back to
/// DEFAULT. That is what makes a dismissed prompt mean "no, this time" rather
/// than "no, forever".
///
/// DEFAULT is deliberately not used to answer the request itself. It means "do
/// whatever the browser would have done", and what WebView2 would have done is
/// show its own prompt, which is the thing this module exists to replace.
///
/// Async, and then straight back onto the main thread: the command itself must
/// not run there, and the COM objects cannot leave it. `run_on_main_thread`
/// queues rather than blocks, so this is not the deadlock `add_child` warns
/// about.
#[tauri::command]
pub async fn answer_permission(
    app: AppHandle,
    id: u64,
    allow: bool,
    remember: bool,
) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let Some(pending) = PENDING.with(|p| p.borrow_mut().remove(&id)) else {
            // Already answered, or the webview went away with it. Nothing to do
            // and nothing worth reporting: the request is gone either way.
            return;
        };

        let state = if allow {
            COREWEBVIEW2_PERMISSION_STATE_ALLOW
        } else {
            COREWEBVIEW2_PERMISSION_STATE_DENY
        };

        unsafe {
            let _ = pending.args.SetState(state);
            // Completed last. Until this runs the page is still blocked waiting,
            // so an early return anywhere above leaves it hanging.
            let _ = pending.deferral.Complete();
        }

        // Not `if remember { persist(..) }`. The line above has already
        // persisted it; see the note on this function. Forgetting is the part
        // that takes work, and it happens after the page has its answer so the
        // answer is never at risk of being undone with it.
        if !remember {
            persist(
                &handle,
                &pending.origin,
                pending.kind,
                COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
            );
        }
    })
    .map_err(|e| format!("Could not answer the request: {e}"))
}

/// One decision already in force, for the Settings list.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSetting {
    pub origin: String,
    pub kind: &'static str,
    /// "allow" or "block". Anything at the default is not in this list at all.
    pub state: &'static str,
}

/// Every decision the profile is currently enforcing.
///
/// Read from the runtime rather than from anything Brume stored, which is the
/// point: this list cannot claim a site is blocked while the engine allows it.
///
/// Async and channel-based for the same reason `clear_site_data` is: the answer
/// arrives in a completion handler that needs the main thread, so a sync command
/// waiting on it would block the thread it is waiting for.
#[tauri::command]
pub async fn list_permissions(app: AppHandle) -> Result<Vec<PermissionSetting>, String> {
    use std::sync::mpsc;
    use webview2_com::GetNonDefaultPermissionSettingsCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile4, ICoreWebView2_13};

    let webview = crate::browser::active_content_webview(&app)?;
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform| {
            let result = (|| unsafe {
                let core = platform.controller().CoreWebView2()?;
                let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
                let profile4 = profile.cast::<ICoreWebView2Profile4>()?;

                let sender = tx.clone();
                profile4.GetNonDefaultPermissionSettings(
                    &GetNonDefaultPermissionSettingsCompletedHandler::create(Box::new(
                        move |hr, view| {
                            let mut out = Vec::new();
                            if hr.is_ok() {
                                if let Some(view) = view {
                                    let mut count = 0u32;
                                    let _ = view.Count(&mut count);
                                    for i in 0..count {
                                        if let Ok(item) = view.GetValueAtIndex(i) {
                                            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                                            let _ = item.PermissionKind(&mut kind);
                                            let mut state =
                                                COREWEBVIEW2_PERMISSION_STATE::default();
                                            let _ = item.PermissionState(&mut state);

                                            let mut raw = windows_core::PWSTR::null();
                                            let origin = if item.PermissionOrigin(&mut raw).is_ok()
                                                && !raw.is_null()
                                            {
                                                take_pwstr(raw)
                                            } else {
                                                String::new()
                                            };

                                            out.push(PermissionSetting {
                                                origin,
                                                kind: kind_slug(kind),
                                                state: if state
                                                    == COREWEBVIEW2_PERMISSION_STATE_ALLOW
                                                {
                                                    "allow"
                                                } else {
                                                    "block"
                                                },
                                            });
                                        }
                                    }
                                }
                            }
                            let _ = sender.send(out);
                            Ok(())
                        },
                    )),
                )?;
                Ok::<_, windows_core::Error>(())
            })();

            if result.is_err() {
                // The handler will never fire, so unblock the wait below rather
                // than letting it time out with nothing to show for it.
                let _ = tx.send(Vec::new());
            }
        })
        .map_err(|e| format!("Could not reach the profile: {e}"))?;

    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "The profile did not answer.".to_string())
}

/// Changes or clears one decision, from the Settings list.
///
/// "default" hands the origin back to being asked about next time, which is the
/// only way to undo an answer short of clearing everything.
#[tauri::command]
pub async fn set_permission(
    app: AppHandle,
    origin: String,
    kind: String,
    state: String,
) -> Result<(), String> {
    let kind = slug_kind(&kind).ok_or_else(|| format!("Unknown permission: {kind}"))?;
    let state = match state.as_str() {
        "allow" => COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        "block" => COREWEBVIEW2_PERMISSION_STATE_DENY,
        "default" => COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
        other => return Err(format!("Unknown state: {other}")),
    };

    let handle = app.clone();
    app.run_on_main_thread(move || persist(&handle, &origin, kind, state))
        .map_err(|e| format!("Could not change the permission: {e}"))
}

/// The inverse of `kind_slug`, for slugs arriving back from the chrome.
///
/// Returns `None` rather than guessing: a slug this build does not know is a
/// chrome and a backend that disagree, and silently writing the wrong permission
/// would be far worse than refusing.
fn slug_kind(slug: &str) -> Option<COREWEBVIEW2_PERMISSION_KIND> {
    Some(match slug {
        "microphone" => COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        "camera" => COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        "location" => COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
        "notifications" => COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
        "sensors" => COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS,
        "clipboard" => COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ,
        "downloads" => COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
        "fonts" => COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS,
        "windows" => COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT,
        "autoplay" => COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY,
        _ => return None,
    })
}

/// Writes a decision into the profile, where the runtime enforces it.
///
/// Must already be on the main thread. Best effort: failing to persist means
/// being asked again next time, which is a great deal better than failing to
/// answer the request that is currently blocking a page.
fn persist(
    app: &AppHandle,
    origin: &str,
    kind: COREWEBVIEW2_PERMISSION_KIND,
    state: COREWEBVIEW2_PERMISSION_STATE,
) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile4, ICoreWebView2_13};
    use webview2_com::SetPermissionStateCompletedHandler;

    let Ok(webview) = crate::browser::active_content_webview(app) else {
        return;
    };
    let origin = origin.to_string();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;
            let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
            let profile4 = profile.cast::<ICoreWebView2Profile4>()?;

            profile4.SetPermissionState(
                kind,
                &windows_core::HSTRING::from(origin.as_str()),
                state,
                &SetPermissionStateCompletedHandler::create(Box::new(move |hr| {
                    if hr.is_err() {
                        eprintln!("[permissions] could not persist: {hr:?}");
                    }
                    Ok(())
                })),
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}
