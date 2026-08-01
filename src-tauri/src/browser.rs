//! Window, tabs, and webview management.
//!
//! Brume's window holds the chrome webview plus **one content webview per tab**:
//!
//! ```text
//!   +--------------------------------------------------+
//!   |  tab strip                                       |  <- our HTML
//!   |  toolbar                                         |
//!   +--------------------------------------------------+
//!   |                                                  |
//!   |  content webview for the ACTIVE tab              |  <- the web
//!   |  (inactive tabs are hidden, not destroyed)       |
//!   +--------------------------------------------------+
//! ```
//!
//! One webview per tab rather than one reused webview: switching tabs must not
//! reload the page, lose scroll position, or discard form input. Inactive tabs
//! are hidden and keep running, which is what every real browser does and what
//! makes a background tab still finish loading.
//!
//! Pages in an `<iframe>` were never an option - `X-Frame-Options` and
//! `frame-ancestors` let any site refuse to be embedded, and most large ones do.
//!
//! This is the only module that touches Tauri's `unstable` multiwebview API, so
//! that a breaking change upstream has exactly one place to be repaired.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WindowEvent,
};

/// Chrome geometry, in *logical* pixels.
///
/// Logical rather than physical: on a 150% display the strip must still be this
/// many CSS pixels tall, or the toolbar and the space reserved for it disagree.
/// These must match the heights in `src/index.html`.
const TAB_STRIP_HEIGHT: f64 = 36.0;
const TOOLBAR_HEIGHT: f64 = 40.0;
const CHROME_HEIGHT: f64 = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT;

pub const WINDOW_LABEL: &str = "main";
pub const CHROME_LABEL: &str = "chrome";

/// Fallback landing page.
///
/// Only reached if settings cannot be read at all - normally a new tab uses
/// `SettingsState::resolved_homepage`, which is the user's homepage if they set
/// one and the active engine's own landing page otherwise.
const FALLBACK_HOME: &str = "https://duckduckgo.com/";

/// Where a new tab should open.
fn home_url(app: &AppHandle) -> String {
    let resolved = app
        .state::<crate::settings::SettingsState>()
        .resolved_homepage(app);
    if resolved.is_empty() {
        FALLBACK_HOME.to_string()
    } else {
        resolved
    }
}

/// Session history for one tab.
///
/// Brume owns this rather than deferring to the webview's own history, because
/// nothing exposes whether a webview *can* go back - neither the DOM nor Tauri -
/// and a back button that is always enabled is worse than no back button.
///
/// The cost is that going back re-navigates instead of restoring from the
/// back-forward cache, so scroll position is lost and the page is refetched.
/// The way out is WebView2's own CanGoBack via `webview2-com`.
#[derive(Default)]
struct NavState {
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
    ///
    /// Ignores a URL identical to the current entry, which is what lets `back`
    /// and `forward` - which re-navigate to a known entry - walk the history
    /// without corrupting it.
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

struct Tab {
    id: u32,
    /// Label of this tab's content webview. Derived from the id and never reused.
    label: String,
    title: String,
    nav: NavState,
}

impl Tab {
    fn display_title(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        // Before the first title arrives, the host reads better than a raw URL
        // and much better than an empty tab.
        // tauri re-exports the url crate, so this needs no extra dependency.
        self.nav
            .current()
            .and_then(|u| tauri::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
            .unwrap_or_else(|| "New tab".to_string())
    }
}

#[derive(Default)]
struct Tabs {
    items: Vec<Tab>,
    /// Id, not index. An index would silently point at the wrong tab the moment
    /// one before it is closed.
    active: u32,
    next_id: u32,
}

impl Tabs {
    fn active_tab(&self) -> Option<&Tab> {
        self.items.iter().find(|t| t.id == self.active)
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let active = self.active;
        self.items.iter_mut().find(|t| t.id == active)
    }

    fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.items.iter_mut().find(|t| t.id == id)
    }
}

pub struct Browser {
    tabs: Mutex<Tabs>,
    /// Whether the chrome is expanded over the whole window to show history,
    /// bookmarks or settings.
    ///
    /// The panel lives inside the chrome webview rather than in a tab or a
    /// second window. A tab would need IPC permissions, which would mean putting
    /// a privileged page in the same list as arbitrary websites; a separate
    /// window is heavier and loses the browser's context. Expanding the chrome
    /// keeps every privileged surface in the one webview that already has
    /// capabilities.
    panel_open: AtomicBool,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            tabs: Mutex::new(Tabs::default()),
            panel_open: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// State published to the chrome
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TabView {
    pub id: u32,
    pub title: String,
    pub url: String,
    pub active: bool,
    pub loading: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub tabs: Vec<TabView>,
    /// Address bar contents: the active tab's URL.
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
    /// Whether the active tab's URL is bookmarked, so the star reflects reality
    /// rather than the frontend having to track it separately and drift.
    pub bookmarked: bool,
    pub panel_open: bool,
}

fn snapshot(tabs: &Tabs, bookmarked: bool, panel_open: bool) -> BrowserState {
    let active = tabs.active_tab();

    BrowserState {
        tabs: tabs
            .items
            .iter()
            .map(|t| TabView {
                id: t.id,
                title: t.display_title(),
                url: t.nav.current().cloned().unwrap_or_default(),
                active: t.id == tabs.active,
                loading: t.nav.loading,
            })
            .collect(),
        url: active
            .and_then(|t| t.nav.current().cloned())
            .unwrap_or_default(),
        can_go_back: active.is_some_and(|t| t.nav.can_go_back()),
        can_go_forward: active.is_some_and(|t| t.nav.can_go_forward()),
        loading: active.is_some_and(|t| t.nav.loading),
        bookmarked,
        panel_open,
    }
}

/// Builds a snapshot, consulting the bookmark store for the active URL.
///
/// Split out because the bookmark lookup needs its own lock, and taking it while
/// holding the tabs lock is exactly the pattern that turns into a deadlock the
/// first time something calls in the other order.
fn current_state(app: &AppHandle) -> BrowserState {
    let active_url = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        tabs.active_tab()
            .and_then(|t| t.nav.current().cloned())
            .unwrap_or_default()
    };

    let bookmarked = app.state::<crate::store::Store>().is_bookmarked(&active_url);

    let browser = app.state::<Browser>();
    let panel_open = browser.panel_open.load(Ordering::Relaxed);
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
    snapshot(&tabs, bookmarked, panel_open)
}

/// Pushes current state to the chrome.
///
/// The chrome never asks; it is told. A one-way feed means the tab strip and the
/// buttons cannot disagree with reality after a link click, a redirect, or a
/// background tab finishing its load.
fn publish(app: &AppHandle) {
    let state = current_state(app);

    // The window title tracks the active tab, the way every browser's does.
    // Driven from the same place as the rest of the state so the two cannot
    // disagree - a title left showing a page you have navigated away from is a
    // small thing that reads as broken.
    if let Some(window) = app.get_window(WINDOW_LABEL) {
        let active = state.tabs.iter().find(|t| t.active);
        let title = match active.map(|t| t.title.as_str()).unwrap_or("") {
            "" => "Brume".to_string(),
            page => format!("{page} — Brume"),
        };
        let _ = window.set_title(&title);
    }

    let _ = app.emit_to(CHROME_LABEL, "brume://state", state);
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Positions the chrome and the active tab, and hides the rest.
///
/// Child webviews take no part in any layout system - they are rectangles the
/// host places by hand, so this runs on every resize and scale-factor change.
fn relayout(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return Ok(());
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);

    let panel_open = app
        .state::<Browser>()
        .panel_open
        .load(Ordering::Relaxed);

    // With the panel open the chrome takes the whole window and every content
    // webview is hidden. That is what keeps history, bookmarks and settings
    // inside the one webview that holds capabilities, instead of needing a
    // privileged tab alongside arbitrary websites.
    let chrome_height = if panel_open { size.height } else { CHROME_HEIGHT };

    // A window can be dragged smaller than the chrome mid-resize; clamping stops
    // the content webview being handed a negative height.
    let content_height = (size.height - CHROME_HEIGHT).max(0.0);

    if let Some(chrome) = app.get_webview(CHROME_LABEL) {
        chrome.set_position(LogicalPosition::new(0.0, 0.0))?;
        chrome.set_size(LogicalSize::new(size.width, chrome_height))?;
    }

    let (active_label, inactive_labels) = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        (
            tabs.active_tab().map(|t| t.label.clone()),
            tabs.items
                .iter()
                .filter(|t| t.id != tabs.active)
                .map(|t| t.label.clone())
                .collect::<Vec<_>>(),
        )
    };

    // Hidden rather than resized to nothing: a hidden webview keeps running, so
    // a background tab still finishes loading and keeps its scroll position.
    for label in inactive_labels {
        if let Some(view) = app.get_webview(&label) {
            let _ = view.hide();
        }
    }

    if let Some(label) = active_label {
        if let Some(view) = app.get_webview(&label) {
            if panel_open {
                let _ = view.hide();
            } else {
                view.set_position(LogicalPosition::new(0.0, CHROME_HEIGHT))?;
                view.set_size(LogicalSize::new(size.width, content_height))?;
                let _ = view.show();
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Creates a content webview for one tab and registers its event handlers.
fn spawn_tab_webview(app: &AppHandle, id: u32, label: &str, url: &str) -> tauri::Result<()> {
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return Ok(());
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);
    let content_height = (size.height - CHROME_HEIGHT).max(0.0);

    let parsed = url.parse().unwrap_or_else(|_| {
        // A homepage the user typed by hand can be unparseable; falling back
        // beats refusing to open a tab.
        FALLBACK_HOME
            .parse()
            .expect("FALLBACK_HOME is a compile-time constant and must be valid")
    });

    let nav_handle = app.clone();
    let load_handle = app.clone();
    let title_handle = app.clone();

    window.add_child(
        WebviewBuilder::new(label, WebviewUrl::External(parsed))
            .on_navigation(move |url| {
                // Fires for every navigation, including ones Brume did not
                // start: link clicks, form submissions, redirects, JavaScript.
                // Recording here rather than only in `navigate` is what keeps
                // the address bar honest.
                {
                    let browser = nav_handle.state::<Browser>();
                    let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
                    if let Some(tab) = tabs.tab_mut(id) {
                        tab.nav.push(url.to_string());
                        tab.nav.loading = true;
                        // The old page's title does not describe the new one.
                        tab.title.clear();
                    }
                }
                publish(&nav_handle);
                true
            })
            .on_page_load(move |_webview, payload| {
                let finished = matches!(payload.event(), PageLoadEvent::Finished);

                // What to record is decided while holding the tabs lock, but the
                // store is written after releasing it. Taking the store's lock
                // while holding this one would establish a lock order that the
                // bookmark path takes in reverse.
                let visit = {
                    let browser = load_handle.state::<Browser>();
                    let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
                    match tabs.tab_mut(id) {
                        Some(tab) => {
                            tab.nav.loading = !finished;
                            if finished {
                                tab.nav
                                    .current()
                                    .cloned()
                                    .map(|url| (url, tab.display_title()))
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                };

                // Recorded on load *finished* rather than on navigation start,
                // so the entry carries a real title instead of an empty one.
                if let Some((url, title)) = visit {
                    load_handle
                        .state::<crate::store::Store>()
                        .record_visit(&url, &title);
                }

                publish(&load_handle);
            })
            .on_document_title_changed(move |_webview, title| {
                // Titles arrive from the runtime, not from the page over IPC.
                //
                // That distinction matters: the content webview is deliberately
                // outside every capability, so a website cannot call a Brume
                // command. Having pages report their own titles would have meant
                // opening an IPC channel to the entire internet for a cosmetic
                // feature.
                {
                    let browser = title_handle.state::<Browser>();
                    let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
                    if let Some(tab) = tabs.tab_mut(id) {
                        tab.title = title;
                    }
                }
                publish(&title_handle);
            }),
        LogicalPosition::new(0.0, CHROME_HEIGHT),
        LogicalSize::new(size.width, content_height),
    )?;

    Ok(())
}

/// Builds the window, the chrome, and the first tab.
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

    window.add_child(
        WebviewBuilder::new(CHROME_LABEL, WebviewUrl::App("index.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, CHROME_HEIGHT),
    )?;

    open_tab_inner(app, None)?;

    let event_handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let _ = relayout(&event_handle);
        }
        // Shortcuts are registered globally, so they are armed only while Brume
        // is in front - otherwise Brume would be holding Ctrl+T hostage for
        // every other application on the machine.
        WindowEvent::Focused(focused) => {
            crate::shortcuts::set_active(&event_handle, *focused);
        }
        _ => {}
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn active_webview(app: &AppHandle) -> Result<tauri::webview::Webview, String> {
    let label = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        tabs.active_tab().map(|t| t.label.clone())
    };

    label
        .and_then(|l| app.get_webview(&l))
        .ok_or_else(|| "No active tab.".to_string())
}

/// The real implementation, in Tauri's error type.
///
/// Split from the command so that `build` - which returns `tauri::Result` - can
/// open the first tab through the same path, instead of the command's
/// stringly-typed error being forced through a conversion at the call site.
fn open_tab_inner(app: &AppHandle, url: Option<String>) -> tauri::Result<()> {
    let target = url.unwrap_or_else(|| home_url(app));

    let (id, label) = {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        let id = tabs.next_id;
        tabs.next_id += 1;

        // Labels are never reused, even after a tab is closed. Reusing one risks
        // a stale handler from the old webview writing into the new tab.
        let label = format!("tab-{id}");

        tabs.items.push(Tab {
            id,
            label: label.clone(),
            title: String::new(),
            nav: NavState::default(),
        });
        tabs.active = id;

        (id, label)
    };

    spawn_tab_webview(app, id, &label, &target)?;
    relayout(app)?;
    publish(app);
    Ok(())
}

// These three commands are `async` for a reason that is invisible until it bites.
//
// Tauri runs a *synchronous* command on the main thread. `Window::add_child`
// dispatches the build onto the main thread and then blocks the caller waiting
// for the result:
//
//     self.run_on_main_thread(move || { ...build...; tx.send(res) })?;
//     rx.recv().unwrap()
//
// Called from the main thread, that queues work onto the very thread it then
// blocks - the queued closure can never run, and the app deadlocks with the new
// webview stranded at about:blank. Nothing errors; the whole UI simply stops
// responding, which is a miserable thing to debug.
//
// Declaring the command `async` puts it on the async runtime instead, so
// add_child dispatches to a main thread that is still free to run it. The same
// applies to any future command that creates, closes or reparents a webview.

#[tauri::command]
pub async fn open_tab(app: AppHandle, url: Option<String>) -> Result<(), String> {
    open_tab_inner(&app, url).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_inner(&app, id)
}

fn close_tab_inner(app: &AppHandle, id: u32) -> Result<(), String> {
    let (label, closed_last, new_active) = {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        let Some(pos) = tabs.items.iter().position(|t| t.id == id) else {
            return Ok(());
        };

        let label = tabs.items[pos].label.clone();
        let was_active = tabs.active == id;
        tabs.items.remove(pos);

        if tabs.items.is_empty() {
            (label, true, None)
        } else {
            if was_active {
                // Activate the neighbour to the right, or the new last tab -
                // which is what closing a tab in any mainstream browser does.
                let next = tabs.items.get(pos).or_else(|| tabs.items.last());
                tabs.active = next.map(|t| t.id).unwrap_or(0);
            }
            let active = tabs.active;
            (label, false, Some(active))
        }
    };

    if let Some(view) = app.get_webview(&label) {
        let _ = view.close();
    }

    if closed_last {
        // Closing the final tab closes the browser, as it does everywhere else.
        if let Some(window) = app.get_window(WINDOW_LABEL) {
            let _ = window.close();
        }
        return Ok(());
    }

    let _ = new_active;
    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        if tabs.items.iter().any(|t| t.id == id) {
            tabs.active = id;
        }
    }
    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

/// Navigates the active tab to whatever the address bar contained.
///
/// The search engine comes from settings rather than from the caller: the chrome
/// should not have to know, or be able to disagree about, which engine is current.
#[tauri::command]
pub fn navigate(app: AppHandle, input: String) -> Result<(), String> {
    let settings = app.state::<crate::settings::SettingsState>();
    let engine_id = settings.get().search_engine;
    let target = crate::search::resolve(&input, &engine_id, settings.is_dark(&app));
    if target.is_empty() {
        return Ok(());
    }

    let url = target
        .parse()
        .map_err(|_| format!("That does not look like an address: {target}"))?;

    active_webview(&app)?
        .navigate(url)
        .map_err(|e| e.to_string())
}

/// Walks the active tab's history by `delta` entries.
fn traverse(app: &AppHandle, forward: bool) -> Result<(), String> {
    let target = {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.active_tab_mut() else {
            return Ok(());
        };

        if forward {
            if !tab.nav.can_go_forward() {
                return Ok(());
            }
            tab.nav.index += 1;
        } else {
            if !tab.nav.can_go_back() {
                return Ok(());
            }
            tab.nav.index -= 1;
        }
        tab.nav.current().cloned()
    };

    if let Some(url) = target {
        let parsed = url
            .parse()
            .map_err(|_| "Invalid history entry".to_string())?;
        active_webview(app)?
            .navigate(parsed)
            .map_err(|e| e.to_string())?;
    }
    publish(app);
    Ok(())
}

#[tauri::command]
pub fn go_back(app: AppHandle) -> Result<(), String> {
    traverse(&app, false)
}

#[tauri::command]
pub fn go_forward(app: AppHandle) -> Result<(), String> {
    traverse(&app, true)
}

#[tauri::command]
pub fn reload(app: AppHandle) -> Result<(), String> {
    active_webview(&app)?
        .eval("location.reload()")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_loading(app: AppHandle) -> Result<(), String> {
    active_webview(&app)?
        .eval("window.stop()")
        .map_err(|e| e.to_string())?;

    {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        if let Some(tab) = tabs.active_tab_mut() {
            tab.nav.loading = false;
        }
    }
    publish(&app);
    Ok(())
}

/// Id of the active tab, for callers that act on "whatever is in front".
pub fn active_tab_id(app: &AppHandle) -> Option<u32> {
    let browser = app.state::<Browser>();
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
    tabs.active_tab().map(|t| t.id)
}

/// Moves to the next or previous tab, wrapping at the ends.
///
/// Wrapping rather than stopping: Ctrl+Tab on the last tab going nowhere feels
/// broken, and every browser cycles.
pub fn neighbour_tab_id(app: &AppHandle, forward: bool) -> Option<u32> {
    let browser = app.state::<Browser>();
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");

    let count = tabs.items.len();
    if count == 0 {
        return None;
    }
    let current = tabs.items.iter().position(|t| t.id == tabs.active)?;
    let next = if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    };
    tabs.items.get(next).map(|t| t.id)
}

/// Lets the chrome ask for the current state once, on load.
///
/// Everything after that arrives by event.
#[tauri::command]
pub fn browser_state(app: AppHandle) -> BrowserState {
    current_state(&app)
}

/// Expands the chrome over the whole window, or restores it.
///
/// Async for the same reason the tab commands are: it re-lays-out webviews.
#[tauri::command]
pub async fn set_panel(app: AppHandle, open: bool) -> Result<(), String> {
    app.state::<Browser>()
        .panel_open
        .store(open, Ordering::Relaxed);
    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

/// Bookmarks or un-bookmarks the active tab, and republishes so the star updates.
#[tauri::command]
pub fn toggle_bookmark_active(app: AppHandle) -> Result<bool, String> {
    let (url, title) = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        match tabs.active_tab() {
            Some(t) => (
                t.nav.current().cloned().unwrap_or_default(),
                t.display_title(),
            ),
            None => return Ok(false),
        }
    };

    let bookmarked = app
        .state::<crate::store::Store>()
        .toggle_bookmark(&url, &title)?;
    publish(&app);
    Ok(bookmarked)
}
