//! Window and webview management.
//!
//! Brume's window contains **two sibling webviews**, not one:
//!
//! ```text
//!   +--------------------------------------------------+
//!   |  chrome    toolbar, address bar, later the tabs   |  <- our HTML
//!   +--------------------------------------------------+
//!   |                                                  |
//!   |  content   whatever the user is actually looking  |  <- the web
//!   |            at                                    |
//!   +--------------------------------------------------+
//! ```
//!
//! The obvious alternative - one webview showing our chrome, with pages in an
//! `<iframe>` - does not work. `X-Frame-Options` and `frame-ancestors` let any
//! site refuse to be embedded, and most large ones do. A browser that cannot
//! open Google is not a browser.
//!
//! This is the only module that touches Tauri's `unstable` multiwebview API, so
//! that a breaking change upstream has exactly one place to be repaired.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WindowEvent,
};

/// Height of the chrome strip in logical pixels.
///
/// Logical, not physical: on a 150% display the strip must still be 48 CSS
/// pixels tall, or the toolbar and the space reserved for it disagree.
const CHROME_HEIGHT: f64 = 48.0;

pub const WINDOW_LABEL: &str = "main";
pub const CHROME_LABEL: &str = "chrome";
pub const CONTENT_LABEL: &str = "content";

/// Where a new window starts. Becomes a setting in step 8.
///
/// Carries the same promo-suppression parameters as the search template in
/// `search.rs`, so the landing page is as free of house advertising as the
/// results page is.
const HOME_URL: &str = "https://duckduckgo.com/?kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1&kae=d";

/// Session history for the content webview.
///
/// Brume owns this rather than deferring to the webview's own history, because
/// there is no way to ask a webview whether it *can* go back - neither the DOM
/// nor Tauri exposes it - and a back button that is always enabled is worse than
/// no back button.
///
/// The cost is that going back re-navigates instead of restoring from the
/// back-forward cache, so scroll position is lost and the page is refetched.
/// That is a real regression against a mainstream browser and the obvious thing
/// to improve later, most likely by reaching into WebView2's own CanGoBack
/// through webview2-com.
#[derive(Default)]
pub struct NavState {
    entries: Vec<String>,
    /// Index into `entries` of the page currently displayed.
    index: usize,
    loading: bool,
}

impl NavState {
    fn current(&self) -> Option<&String> {
        self.entries.get(self.index)
    }

    fn can_go_back(&self) -> bool {
        self.index > 0
    }

    fn can_go_forward(&self) -> bool {
        !self.entries.is_empty() && self.index + 1 < self.entries.len()
    }

    /// Records arriving somewhere new, discarding any forward history.
    fn push(&mut self, url: String) {
        if self.current().is_some_and(|c| *c == url) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.index + 1);
        }
        self.entries.push(url);
        self.index = self.entries.len() - 1;
    }
}

pub struct Browser {
    pub nav: Mutex<NavState>,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            nav: Mutex::new(NavState::default()),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NavStatus {
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
}

/// Pushes the current navigation state to the chrome.
///
/// The chrome never asks; it is told. Keeping it a one-way feed means the
/// buttons cannot disagree with reality after a link click or a redirect, which
/// they would if the frontend tracked any of this itself.
fn publish(app: &AppHandle) {
    let state = app.state::<Browser>();
    let nav = state.nav.lock().expect("nav mutex poisoned");

    let status = NavStatus {
        url: nav.current().cloned().unwrap_or_default(),
        can_go_back: nav.can_go_back(),
        can_go_forward: nav.can_go_forward(),
        loading: nav.loading,
    };

    let _ = app.emit_to(CHROME_LABEL, "brume://nav", status);
}

/// Lays the two webviews out for the window's current size.
///
/// Called on every resize. Both webviews are positioned explicitly because
/// child webviews do not participate in any layout system - they are rectangles
/// the host places by hand.
fn relayout(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return Ok(());
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);

    // A window can be dragged to a size smaller than the chrome during a resize;
    // clamping keeps the content webview from being given a negative height.
    let content_height = (size.height - CHROME_HEIGHT).max(0.0);

    if let Some(chrome) = app.get_webview(CHROME_LABEL) {
        chrome.set_position(LogicalPosition::new(0.0, 0.0))?;
        chrome.set_size(LogicalSize::new(size.width, CHROME_HEIGHT))?;
    }

    if let Some(content) = app.get_webview(CONTENT_LABEL) {
        content.set_position(LogicalPosition::new(0.0, CHROME_HEIGHT))?;
        content.set_size(LogicalSize::new(size.width, content_height))?;
    }

    Ok(())
}

/// Builds the window and its two webviews.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let window = tauri::window::WindowBuilder::new(app, WINDOW_LABEL)
        .title("Brume")
        .inner_size(1200.0, 800.0)
        .min_inner_size(480.0, 360.0)
        .center()
        .theme(Some(tauri::Theme::Dark))
        // Painted before the webviews attach, so startup does not flash white.
        .background_color(tauri::window::Color(0x10, 0x14, 0x18, 0xff))
        .build()?;

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);
    let content_height = (size.height - CHROME_HEIGHT).max(0.0);

    window.add_child(
        WebviewBuilder::new(CHROME_LABEL, WebviewUrl::App("index.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, CHROME_HEIGHT),
    )?;

    let nav_handle = app.clone();
    let load_handle = app.clone();

    window.add_child(
        WebviewBuilder::new(CONTENT_LABEL, WebviewUrl::External(HOME_URL.parse().unwrap()))
            .on_navigation(move |url| {
                // Fires for every navigation, including ones Brume did not
                // start: link clicks, form submissions, redirects, and
                // JavaScript. Recording here rather than only in `navigate` is
                // what keeps the address bar honest.
                //
                // `push` ignores a URL identical to the current entry, which is
                // what makes back() and forward() - which re-navigate to a known
                // entry - not corrupt the very history they are walking.
                let state = nav_handle.state::<Browser>();
                {
                    let mut nav = state.nav.lock().expect("nav mutex poisoned");
                    nav.push(url.to_string());
                    nav.loading = true;
                }
                publish(&nav_handle);
                true
            })
            .on_page_load(move |_webview, payload| {
                {
                    let state = load_handle.state::<Browser>();
                    let mut nav = state.nav.lock().expect("nav mutex poisoned");
                    nav.loading = matches!(payload.event(), PageLoadEvent::Started);
                }
                publish(&load_handle);
            }),
        LogicalPosition::new(0.0, CHROME_HEIGHT),
        LogicalSize::new(size.width, content_height),
    )?;

    let resize_handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            let _ = relayout(&resize_handle);
        }
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn content_webview(app: &AppHandle) -> Result<tauri::webview::Webview, String> {
    app.get_webview(CONTENT_LABEL)
        .ok_or_else(|| "The content view is not available.".to_string())
}

/// Navigates to whatever the address bar contained.
///
/// The search engine comes from settings rather than from the caller: the
/// chrome should not have to know, or be able to disagree about, which engine is
/// current.
#[tauri::command]
pub fn navigate(app: AppHandle, input: String) -> Result<(), String> {
    let engine_id = app.state::<crate::settings::SettingsState>().get().search_engine;
    let target = crate::search::resolve(&input, &engine_id);
    if target.is_empty() {
        return Ok(());
    }

    let url = target
        .parse()
        .map_err(|_| format!("That does not look like an address: {target}"))?;

    content_webview(&app)?.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn go_back(app: AppHandle) -> Result<(), String> {
    let target = {
        let state = app.state::<Browser>();
        let mut nav = state.nav.lock().expect("nav mutex poisoned");
        if !nav.can_go_back() {
            return Ok(());
        }
        nav.index -= 1;
        nav.current().cloned()
    };

    if let Some(url) = target {
        let parsed = url.parse().map_err(|_| "Invalid history entry".to_string())?;
        content_webview(&app)?.navigate(parsed).map_err(|e| e.to_string())?;
    }
    publish(&app);
    Ok(())
}

#[tauri::command]
pub fn go_forward(app: AppHandle) -> Result<(), String> {
    let target = {
        let state = app.state::<Browser>();
        let mut nav = state.nav.lock().expect("nav mutex poisoned");
        if !nav.can_go_forward() {
            return Ok(());
        }
        nav.index += 1;
        nav.current().cloned()
    };

    if let Some(url) = target {
        let parsed = url.parse().map_err(|_| "Invalid history entry".to_string())?;
        content_webview(&app)?.navigate(parsed).map_err(|e| e.to_string())?;
    }
    publish(&app);
    Ok(())
}

#[tauri::command]
pub fn reload(app: AppHandle) -> Result<(), String> {
    content_webview(&app)?
        .eval("location.reload()")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_loading(app: AppHandle) -> Result<(), String> {
    content_webview(&app)?
        .eval("window.stop()")
        .map_err(|e| e.to_string())?;

    {
        let state = app.state::<Browser>();
        let mut nav = state.nav.lock().expect("nav mutex poisoned");
        nav.loading = false;
    }
    publish(&app);
    Ok(())
}

/// Lets the chrome ask for the current state once, on load.
///
/// Everything after that arrives by event.
#[tauri::command]
pub fn nav_status(app: AppHandle) -> NavStatus {
    let state = app.state::<Browser>();
    let nav = state.nav.lock().expect("nav mutex poisoned");
    NavStatus {
        url: nav.current().cloned().unwrap_or_default(),
        can_go_back: nav.can_go_back(),
        can_go_forward: nav.can_go_forward(),
        loading: nav.loading,
    }
}
