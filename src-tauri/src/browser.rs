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
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
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

/// Extra chrome height while the find bar is showing.
///
/// The bar grows the chrome and shrinks the page rather than floating over it.
/// An overlay would need a third webview, and covering the page is the one
/// thing a find bar must not do: the match it just scrolled to could be
/// underneath it.
const FIND_BAR_HEIGHT: f64 = 36.0;

/// Extra chrome height while the bookmarks bar is showing.
///
/// Shorter than the find bar, which has to hold a text field at a comfortable
/// size. This holds a row of small labels, and every pixel of it is one the page
/// does not get for as long as the bar is on.
const BOOKMARKS_BAR_HEIGHT: f64 = 32.0;

pub const WINDOW_LABEL: &str = "main";
pub const CHROME_LABEL: &str = "chrome";

/// Nudges the chrome to re-read the downloads list. Carries no payload: the
/// panel asks for the list itself, so the event only has to say "something
/// changed" rather than duplicate the state.
pub const DOWNLOADS_EVENT: &str = "brume://downloads";

/// Same idea for bookmarks, and it has two listeners rather than one: the panel
/// and the bar both render the list, so both have to hear that it moved.
///
/// The list is not folded into `BrowserState` instead. That is published on
/// every navigation, title change and load-progress tick, and putting every
/// bookmark in it would re-serialise the whole list dozens of times per page for
/// a thing that changes when the user presses Ctrl+D.
pub const BOOKMARKS_EVENT: &str = "brume://bookmarks";

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

/// Where one tab currently is.
///
/// This used to be a `Vec<String>` with an index, because nothing exposed
/// whether a webview could go back, so Brume kept its own stack and went back by
/// re-navigating. That refetched the page and lost scroll position on every
/// press. history.rs reaches WebView2's real history instead, so the stack is
/// gone and what is left is the current URL plus two flags mirrored from the
/// runtime.
struct NavState {
    url: String,
    loading: bool,
    /// Mirrored from CanGoBack/CanGoForward whenever HistoryChanged fires.
    /// Never computed here: the runtime is the only thing that knows.
    can_back: bool,
    can_forward: bool,
    /// Page zoom, mirrored from ZoomFactorChanged. 1.0 is 100%.
    ///
    /// Per tab, because WebView2 keeps zoom per webview: zooming one tab does
    /// not touch another, and the indicator has to follow whichever is in front.
    zoom: f64,
}

impl Default for NavState {
    /// Hand-written rather than derived, because `zoom` must start at 1.0.
    /// Deriving gives 0.0, and a new tab would report itself at 0% until the
    /// first ZoomFactorChanged.
    fn default() -> Self {
        Self {
            url: String::new(),
            loading: false,
            can_back: false,
            can_forward: false,
            zoom: 1.0,
        }
    }
}

impl NavState {
    fn current(&self) -> Option<&String> {
        if self.url.is_empty() {
            None
        } else {
            Some(&self.url)
        }
    }

    /// Records arriving somewhere. No stack to corrupt any more, so unlike the
    /// old `push` this has nothing to guard against.
    fn set_url(&mut self, url: String) {
        self.url = url;
    }
}

struct Tab {
    id: u32,
    /// Label of this tab's content webview. Derived from the id and never reused.
    label: String,
    title: String,
    nav: NavState,
    /// A private tab keeps nothing: no history entry, no session entry, and no
    /// place in the reopen-closed-tab list.
    ///
    /// The webview is built with `incognito`, so cookies and storage are the
    /// runtime's problem and go when it closes. What is tracked here is the part
    /// Brume itself would otherwise write to disk.
    private: bool,
    /// Pinned tabs sit at the front of the strip, render without a title, and
    /// refuse to close. Refusing is the point: a pinned tab is one you have said
    /// you want kept, so Ctrl+W landing on it would defeat the pinning.
    pinned: bool,
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
            .and_then(|u| {
                u.host_str()
                    .map(|h| h.trim_start_matches("www.").to_string())
            })
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

/// How many closed tabs can be reopened.
///
/// Bounded because this holds URLs of pages the user has closed - which is
/// exactly the list someone clearing their history would expect to be gone. Ten
/// covers the accidental Ctrl+W without keeping a shadow history for the session.
const CLOSED_TAB_LIMIT: usize = 10;

pub struct Browser {
    tabs: Mutex<Tabs>,
    /// URLs of recently closed tabs, most recent last, for Ctrl+Shift+T.
    ///
    /// Only the URL is kept, not the tab's whole history. Reopening restores
    /// where you were, not the trail you took to get there - which is what the
    /// shortcut is actually for, and it avoids persisting a per-tab history
    /// stack for tabs that no longer exist.
    closed: Mutex<Vec<String>>,
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
    /// Whether the find bar is showing, which makes the chrome taller.
    find_open: AtomicBool,
}

/// How much vertical space the chrome occupies right now.
///
/// Not a constant any more: the find bar and the bookmarks bar each add their
/// own height, and both can be open at once. Everywhere that positions a content
/// webview reads this, so the toolbar and the space reserved beneath it cannot
/// drift apart.
///
/// Summed rather than branched, so a third row later is one more term and not a
/// rewrite of the arithmetic.
fn chrome_extent(app: &AppHandle) -> f64 {
    let find_open = app.state::<Browser>().find_open.load(Ordering::Relaxed);
    // Read from settings rather than mirrored into `Browser`. One source of
    // truth: a copy here would be the thing that eventually disagrees with the
    // file after a failed write.
    let bookmarks_bar = app
        .state::<crate::settings::SettingsState>()
        .show_bookmarks_bar();

    extent_for(find_open, bookmarks_bar)
}

/// The arithmetic on its own, so it can be tested.
///
/// Split out from `chrome_extent` because that needs an `AppHandle` and a
/// running app, while this is the part that has to agree with the CSS. The
/// numbers it returns are asserted in the tests at the bottom of this file.
fn extent_for(find_open: bool, bookmarks_bar: bool) -> f64 {
    CHROME_HEIGHT
        + if find_open { FIND_BAR_HEIGHT } else { 0.0 }
        + if bookmarks_bar {
            BOOKMARKS_BAR_HEIGHT
        } else {
            0.0
        }
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            tabs: Mutex::new(Tabs::default()),
            closed: Mutex::new(Vec::new()),
            panel_open: AtomicBool::new(false),
            find_open: AtomicBool::new(false),
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
    pub private: bool,
    pub pinned: bool,
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
    /// Whether the bookmarks bar is showing.
    ///
    /// Published rather than left for the chrome to read from settings, so the
    /// row appearing and the page being resized to make room come off the same
    /// update instead of two that can land in either order.
    pub bookmarks_bar: bool,
    pub panel_open: bool,
    /// Zoom of the active tab. 1.0 is 100%; the chrome hides the control at
    /// that value rather than showing "100%" permanently.
    pub zoom: f64,
}

fn snapshot(tabs: &Tabs, bookmarked: bool, bookmarks_bar: bool, panel_open: bool) -> BrowserState {
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
                private: t.private,
                pinned: t.pinned,
            })
            .collect(),
        url: active
            .and_then(|t| t.nav.current().cloned())
            .unwrap_or_default(),
        can_go_back: active.is_some_and(|t| t.nav.can_back),
        can_go_forward: active.is_some_and(|t| t.nav.can_forward),
        loading: active.is_some_and(|t| t.nav.loading),
        bookmarked,
        bookmarks_bar,
        panel_open,
        zoom: active.map(|t| t.nav.zoom).unwrap_or(1.0),
    }
}

/// Builds a snapshot, consulting the bookmark store for the active URL.
///
/// Split out because the bookmark lookup needs its own lock, and taking it while
/// holding the tabs lock is exactly the pattern that turns into a deadlock the
/// first time something calls in the other order. The settings read below is
/// there for the same reason and is deliberately done before the tabs lock too.
fn current_state(app: &AppHandle) -> BrowserState {
    let active_url = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        tabs.active_tab()
            .and_then(|t| t.nav.current().cloned())
            .unwrap_or_default()
    };

    let bookmarked = app
        .state::<crate::store::Store>()
        .is_bookmarked(&active_url);
    let bookmarks_bar = app
        .state::<crate::settings::SettingsState>()
        .show_bookmarks_bar();

    let browser = app.state::<Browser>();
    let panel_open = browser.panel_open.load(Ordering::Relaxed);
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
    snapshot(&tabs, bookmarked, bookmarks_bar, panel_open)
}

/// Closes the panel, because something is about to show a page.
///
/// While the panel is open the chrome takes the whole window and **every content
/// webview is hidden behind it**, so anything meaning "show me a page" has to
/// close it first. Without this a new tab opens, loads, and is never seen: the
/// strip grows a tab, the address bar updates, and the window carries on showing
/// Settings. That reads as the browser being broken, and it was.
///
/// Returns whether it was open, so a caller can skip a relayout it does not need.
/// `swap` rather than a load and a store, so two of these racing cannot both
/// decide they were the one that closed it.
fn dismiss_panel(app: &AppHandle) -> bool {
    app.state::<Browser>()
        .panel_open
        .swap(false, Ordering::Relaxed)
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
        // The tab's *raw* title, not its display title.
        //
        // TabView carries display_title(), which falls back to the host and then
        // to "New tab" so the tab strip never shows a blank. Reading that here
        // meant the empty-title branch below could never be taken, and a freshly
        // launched Brume announced itself as "New tab - Brume" in the taskbar.
        // The window wants the opposite fallback: no page title yet means the
        // application name on its own.
        let raw_title = {
            let browser = app.state::<Browser>();
            let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
            tabs.active_tab()
                .map(|t| t.title.clone())
                .unwrap_or_default()
        };

        let title = match raw_title.trim() {
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

    let panel_open = app.state::<Browser>().panel_open.load(Ordering::Relaxed);

    // How tall the chrome is when it is only chrome. Grows with the find bar.
    let extent = chrome_extent(app);

    // With the panel open the chrome takes the whole window and every content
    // webview is hidden. That is what keeps history, bookmarks and settings
    // inside the one webview that holds capabilities, instead of needing a
    // privileged tab alongside arbitrary websites.
    let chrome_height = if panel_open { size.height } else { extent };

    // A window can be dragged smaller than the chrome mid-resize; clamping stops
    // the content webview being handed a negative height.
    let content_height = (size.height - extent).max(0.0);

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
                view.set_position(LogicalPosition::new(0.0, extent))?;
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
fn spawn_tab_webview(
    app: &AppHandle,
    id: u32,
    label: &str,
    url: &str,
    private: bool,
) -> tauri::Result<()> {
    // An error, not a silent Ok. Returning Ok here registered a tab that had no
    // webview behind it and reported success, which is the worst of both: the
    // tab strip grew a row that rendered nothing and nothing anywhere said why.
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return Err(tauri::Error::WindowNotFound);
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);
    // Matches whatever the chrome currently occupies, so a tab opened while the
    // find bar is up is not born 36px too tall and overlapping it.
    let extent = chrome_extent(app);
    let content_height = (size.height - extent).max(0.0);

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
    let newwin_handle = app.clone();
    let download_handle = app.clone();
    // A link opened from a private tab stays private. Inheriting is the only
    // sane rule: the page that asked for it is already in a private context.
    let opener_private = private;

    window.add_child(
        WebviewBuilder::new(label, WebviewUrl::External(parsed))
            // Incognito hands cookies, storage and cache to a throwaway
            // partition the runtime discards with the webview. Brume still has
            // to keep its own records out, which is what Tab::private is for.
            .incognito(private)
            // Ctrl+scroll and Ctrl+plus/minus, which WebView2 gates behind its
            // IsZoomControlEnabled setting.
            //
            // wry and tauri-runtime both default this to `false`, so leaving it
            // unset does not mean "platform default" - it means zoom is switched
            // off outright, on a browser. Enabling it is the whole fix: WebView2
            // handles both the wheel and the keyboard itself, so there is no
            // accelerator for Brume to register and nothing to keep in step.
            .zoom_hotkeys_enabled(true)
            // A page asking for a new window gets a Brume tab instead.
            //
            // This is not an enhancement, it repairs a hole: wry registers a
            // NewWindowRequested handler unconditionally, and when no callback
            // is supplied its else-branch calls SetHandled(true) and completes
            // the deferral - which *cancels* the request. Without this,
            // target="_blank" and window.open() silently did nothing at all.
            .on_new_window(move |url, _features| {
                let handle = newwin_handle.clone();
                let target = url.to_string();
                // Spawned, never called inline. This handler runs on the main
                // thread, and open_tab_inner reaches add_child, which dispatches
                // to the main thread and then blocks waiting on it - the same
                // deadlock the async commands below exist to avoid.
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = open_tab_inner(&handle, Some(target), opener_private) {
                        eprintln!("[browser] new-window request failed: {e}");
                    }
                });
                // Deny, not Allow: allowing it hands the page a bare OS window
                // with no chrome, no tab strip and no address bar - a popup
                // Brume could neither show the URL of nor close.
                NewWindowResponse::Deny
            })
            // Downloads, so Brume has a record of them.
            //
            // WebView2 handles the transfer and its own default dialog either
            // way; without this Brume simply never heard about it, so there was
            // nothing to list. The destination is left alone deliberately: the
            // runtime already puts files where Windows says downloads go, and
            // overriding that to somewhere Brume invented would be worse.
            //
            // The runtime reports started and finished and nothing between, so
            // there is no byte count to show a progress bar with. Reaching one
            // means going through ICoreWebView2DownloadOperation directly.
            .on_download(move |_webview, event| {
                match event {
                    tauri::webview::DownloadEvent::Requested { url, destination } => {
                        let name = destination
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        download_handle
                            .state::<crate::store::Store>()
                            .begin_download(url.as_str(), &name);
                    }
                    tauri::webview::DownloadEvent::Finished { url, path, success } => {
                        download_handle
                            .state::<crate::store::Store>()
                            .finish_download(url.as_str(), path.as_deref(), success);
                    }
                    // The enum is non_exhaustive, so a future variant compiles
                    // rather than breaking the build.
                    _ => {}
                }
                let _ = download_handle.emit_to(CHROME_LABEL, DOWNLOADS_EVENT, ());
                // Always allow. Brume is recording, not gatekeeping.
                true
            })
            .on_navigation(move |url| {
                // Brume's own UI is served from tauri.localhost, and the asset
                // protocol from asset.localhost. Neither is somewhere a website
                // has any business navigating to.
                //
                // The content webview holds no capabilities, so a page that got
                // there could not invoke a command today. It could still render
                // Brume's chrome inside a tab, and the whole reason that
                // capability scoping is safe is that nothing else is relying on
                // it alone. Refusing here is a second lock on the same door.
                //
                // Matched exactly rather than by suffix: `ends_with` would also
                // accept `nottauri.localhost`.
                if url
                    .host_str()
                    .is_some_and(|h| h == "tauri.localhost" || h == "asset.localhost")
                {
                    eprintln!("[browser] refused navigation to Brume's own origin: {url}");
                    return false;
                }

                // Fires for every navigation, including ones Brume did not
                // start: link clicks, form submissions, redirects, JavaScript.
                // Recording here rather than only in `navigate` is what keeps
                // the address bar honest.
                {
                    let browser = nav_handle.state::<Browser>();
                    let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
                    if let Some(tab) = tabs.tab_mut(id) {
                        tab.nav.set_url(url.to_string());
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
                            // A private tab records nothing. This is the single
                            // place a visit would reach history from.
                            if finished && !tab.private {
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

                    // The tab has settled somewhere new, so the saved session is
                    // stale. Written here rather than only at quit, so an
                    // unexpected exit still has something to restore.
                    save_session(&load_handle);
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
        LogicalPosition::new(0.0, extent),
        LogicalSize::new(size.width, content_height),
    )?;

    Ok(())
}

/// Default window size, used on first run and whenever a saved one is unusable.
const DEFAULT_WIDTH: f64 = 1200.0;
const DEFAULT_HEIGHT: f64 = 800.0;

/// Whether a saved position still lands on a display that exists.
///
/// Restoring coordinates blindly is how a window ends up invisible: unplug the
/// second monitor and last session's position is somewhere the desktop no longer
/// covers, leaving Brume running with no way to reach it. Only the top-left
/// corner is checked, which is enough to guarantee the title bar is grabbable.
fn position_is_on_screen(window: &tauri::Window, x: f64, y: f64) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };

    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let pos = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        x >= pos.x && y >= pos.y && x < pos.x + size.width && y < pos.y + size.height
    })
}

/// Persists the window's current geometry.
///
/// Skipped while maximised or minimised for the size, because those report the
/// filled or zeroed rectangle rather than the size to return to - saving them
/// would make un-maximising restore to full screen forever.
fn save_geometry(app: &AppHandle) {
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        return;
    }

    let maximized = window.is_maximized().unwrap_or(false);
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let Ok(pos) = window.outer_position() else {
        return;
    };

    let size: LogicalSize<f64> = size.to_logical(scale);
    let pos: LogicalPosition<f64> = pos.to_logical(scale);

    let settings = app.state::<crate::settings::SettingsState>();
    let previous = settings.window();

    // While maximised, keep the previously saved size and record only the flag,
    // so the un-maximised size survives being maximised at quit.
    let (width, height, x, y) = match (maximized, previous) {
        (true, Some(prev)) => (prev.width, prev.height, prev.x, prev.y),
        (true, None) => (DEFAULT_WIDTH, DEFAULT_HEIGHT, pos.x, pos.y),
        (false, _) => (size.width, size.height, pos.x, pos.y),
    };

    let _ = settings.set_window(crate::settings::WindowGeometry {
        x,
        y,
        width,
        height,
        maximized,
    });
}

/// Records the open tabs so the next launch can rebuild them.
///
/// Called whenever the set of tabs changes, not only at quit. Saving on close
/// alone meant a crash or a kill lost the session, which is the one case
/// restoring it is actually for. The write is small and atomic, and it happens
/// far less often than the history append that already runs on every page load.
///
/// Only URLs. Restoring each tab's own back history would mean persisting a
/// stack per tab, and since history.rs handed traversal to WebView2 there is no
/// such stack to persist - the runtime's history dies with the webview.
/// Reopening where you were is the part worth keeping.
fn save_session(app: &AppHandle) {
    let (saved, active) = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        // Private tabs are left out: a session file naming them would outlive
        // the browsing they were supposed to keep off disk.
        let saved: Vec<crate::settings::SessionTab> = tabs
            .items
            .iter()
            .filter(|t| !t.private)
            .filter_map(|t| {
                t.nav.current().and_then(|u| {
                    // A tab still on about:blank has nowhere to be restored to.
                    (!u.starts_with("about:")).then(|| crate::settings::SessionTab {
                        url: u.clone(),
                        pinned: t.pinned,
                    })
                })
            })
            .collect();

        // Position within the filtered list, not the tab id.
        let active = tabs
            .items
            .iter()
            .filter(|t| t.nav.current().is_some_and(|u| !u.starts_with("about:")))
            .position(|t| t.id == tabs.active)
            .unwrap_or(0);

        (saved, active)
    };

    let _ = app
        .state::<crate::settings::SettingsState>()
        .set_session(saved, active);
}

/// Builds the window, the chrome, and the first tab.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let saved = app.state::<crate::settings::SettingsState>().window();

    // Size is taken from the saved geometry, but position is applied *after* the
    // window exists - checking whether coordinates are on a real display needs
    // the monitor list, and that comes off a window.
    let (width, height) = match saved {
        Some(g) if g.width >= 480.0 && g.height >= 360.0 => (g.width, g.height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };

    let window = tauri::window::WindowBuilder::new(app, WINDOW_LABEL)
        .title("Brume")
        .inner_size(width, height)
        .min_inner_size(480.0, 360.0)
        .center()
        .theme(Some(tauri::Theme::Dark))
        // Painted before the webviews attach, so startup does not flash white.
        .background_color(tauri::window::Color(0x10, 0x14, 0x18, 0xff))
        .build()?;

    // Position and maximised state, now that there is a window to ask about
    // monitors. An off-screen saved position is discarded and the centred
    // position from the builder stands.
    if let Some(g) = saved {
        if position_is_on_screen(&window, g.x, g.y) {
            let _ = window.set_position(LogicalPosition::new(g.x, g.y));
        }

        // Then shrink it if it no longer fits.
        //
        // The size was applied by the builder before any monitor was known. A
        // window saved on a large display and reopened on a smaller one would
        // otherwise come back bigger than the screen it is on, with its edges
        // and resize handles somewhere unreachable. Clamped against whichever
        // monitor it actually landed on.
        if let Ok(Some(monitor)) = window.current_monitor() {
            let scale = monitor.scale_factor();
            let available: LogicalSize<f64> = monitor.size().to_logical(scale);
            let fitted_w = width.min(available.width);
            let fitted_h = height.min(available.height);
            if fitted_w < width || fitted_h < height {
                let _ = window.set_size(LogicalSize::new(fitted_w, fitted_h));
                let _ = window.center();
            }
        }

        if g.maximized {
            let _ = window.maximize();
        }
    }

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);

    window.add_child(
        WebviewBuilder::new(CHROME_LABEL, WebviewUrl::App("index.html".into()))
            // The chrome must never leave its own origin.
            //
            // This is the one webview that holds capabilities, so a website
            // loaded here would inherit every command Brume exposes. Nothing in
            // the chrome navigates anywhere today - there is not a single
            // anchor in index.html - but that is a property of the current
            // markup, not of the design, and one "report a bug" link in the
            // settings panel would quietly undo it.
            //
            // Phrased as "refuse external web addresses" rather than "allow a
            // known list", so an internal scheme this does not anticipate still
            // loads. Getting an allowlist wrong here means a blank chrome and a
            // browser that does not start.
            .on_navigation(|url| {
                let is_web = matches!(url.scheme(), "http" | "https");
                let internal = url
                    .host_str()
                    .is_some_and(|h| h == "localhost" || h.ends_with(".localhost"));
                if is_web && !internal {
                    eprintln!("[browser] refused to navigate the chrome to {url}");
                    return false;
                }
                true
            }),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, CHROME_HEIGHT),
    )?;

    // Rebuild last session's tabs, or open the homepage if there is none.
    //
    // Restored in order, then the previously active one is brought to the front.
    // A tab that fails to build is skipped rather than aborting the launch: one
    // bad saved URL should not leave the browser with no window worth looking at.
    let (session, session_active) = app.state::<crate::settings::SettingsState>().session();

    if session.is_empty() {
        open_tab_inner(app, None, false)?;
    } else {
        for saved in &session {
            match open_tab_inner(app, Some(saved.url.clone()), false) {
                // Restored tabs land at the end, so the pinned ones need putting
                // back at the front. Set here rather than passed into
                // open_tab_inner, which would have to know about ordering.
                Ok(()) => {
                    if saved.pinned {
                        let browser = app.state::<Browser>();
                        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
                        let active = tabs.active;
                        if let Some(tab) = tabs.tab_mut(active) {
                            tab.pinned = true;
                        }
                    }
                }
                Err(e) => eprintln!("[browser] could not restore {}: {e}", saved.url),
            }
        }
        // Sorted once, after the lot are in. Stable, so tabs keep the order they
        // were saved in within each group.
        {
            let browser = app.state::<Browser>();
            let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
            tabs.items.sort_by_key(|t| !t.pinned);
        }
        // Every restore left its tab active, so the last one is in front.
        if let Some(id) = tab_id_at(app, session_active) {
            let browser = app.state::<Browser>();
            let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
            tabs.active = id;
        }
        // Nothing restored, so there is no tab at all. Fall back rather than
        // leave an empty window.
        if tab_id_at(app, 0).is_none() {
            open_tab_inner(app, None, false)?;
        }
        relayout(app)?;
        publish(app);
    }

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
        // Saved on close rather than on every Moved and Resized event.
        //
        // Those fire continuously while a window is dragged, and each one would
        // be a serialise-and-rewrite of settings.json - hundreds of writes to
        // record one final position. The cost is that a hard kill loses the
        // last move, which is a fair trade for not thrashing the disk.
        WindowEvent::CloseRequested { .. } => {
            save_geometry(&event_handle);
            save_session(&event_handle);
        }
        _ => {}
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The active tab's content webview, for modules that need the raw thing.
///
/// find.rs reaches through this to the underlying ICoreWebView2. Exposed rather
/// than duplicated so there is still one place that knows how a tab maps to a
/// webview label.
pub fn active_content_webview(app: &AppHandle) -> Result<tauri::webview::Webview, String> {
    active_webview(app)
}

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
fn open_tab_inner(app: &AppHandle, url: Option<String>, private: bool) -> tauri::Result<()> {
    let target = url.unwrap_or_else(|| home_url(app));

    // Before the layout below, not after: a tab opened from the panel has to end
    // up visible. The relayout at the end of this function is what applies it.
    dismiss_panel(app);

    let (id, label, previous_active) = {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        let id = tabs.next_id;
        tabs.next_id += 1;

        // Labels are never reused, even after a tab is closed. Reusing one risks
        // a stale handler from the old webview writing into the new tab.
        let label = format!("tab-{id}");

        // Kept so the tab can be taken back out if the webview fails to build.
        let previous_active = tabs.active;

        tabs.items.push(Tab {
            id,
            label: label.clone(),
            title: String::new(),
            nav: NavState::default(),
            private,
            pinned: false,
        });
        tabs.active = id;

        (id, label, previous_active)
    };

    // Roll back if the webview cannot be created.
    //
    // The tab has to be registered first, because the webview's own handlers
    // fire during construction and look it up by id. But leaving it there on
    // failure produced a tab with nothing behind it that was also the *active*
    // tab, so every subsequent navigate, reload and back resolved to a webview
    // that did not exist and failed. The chrome raises a dialog per failed
    // command, so the browser became unusable until that tab was closed.
    let spawned = spawn_tab_webview(app, id, &label, &target, private);

    // Subscribe to the runtime's history events once the webview exists. Only
    // on success: there is nothing to watch otherwise.
    if spawned.is_ok() {
        crate::history::watch(app, id, &label);
        crate::history::watch_zoom(app, id, &label);
        // Progress only. The start and finish records still come from
        // `on_download` above; this adds the byte count in between.
        crate::downloads::watch(app, &label);
    }

    if let Err(e) = spawned {
        {
            let browser = app.state::<Browser>();
            let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
            tabs.items.retain(|t| t.id != id);
            // Only restore the old active tab if it is still open.
            if tabs.items.iter().any(|t| t.id == previous_active) {
                tabs.active = previous_active;
            }
        }
        let _ = relayout(app);
        publish(app);
        return Err(e);
    }

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
pub async fn open_tab(
    app: AppHandle,
    url: Option<String>,
    private: Option<bool>,
) -> Result<(), String> {
    open_tab_inner(&app, url, private.unwrap_or(false)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_inner(&app, id)
}

fn close_tab_inner(app: &AppHandle, id: u32) -> Result<(), String> {
    let (label, closed_last, reopen_url) = {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        let Some(pos) = tabs.items.iter().position(|t| t.id == id) else {
            return Ok(());
        };

        // A pinned tab refuses to close. Ctrl+W is easy to hit, and pinning is
        // the user saying this one should survive it. Unpin first.
        if tabs.items[pos].pinned {
            return Ok(());
        }

        let label = tabs.items[pos].label.clone();
        let was_active = tabs.active == id;
        // Captured before the removal: afterwards the tab is gone and there is
        // nothing left to read the URL from.
        // A private tab is not offered back by Ctrl+Shift+T. Reopening it
        // would put its URL in a list that survives the tab.
        let reopen_url = if tabs.items[pos].private {
            None
        } else {
            tabs.items[pos].nav.current().cloned()
        };
        tabs.items.remove(pos);

        if tabs.items.is_empty() {
            (label, true, reopen_url)
        } else {
            if was_active {
                // Activate the neighbour to the right, or the new last tab -
                // which is what closing a tab in any mainstream browser does.
                let next = tabs.items.get(pos).or_else(|| tabs.items.last());
                tabs.active = next.map(|t| t.id).unwrap_or(0);
            }
            (label, false, reopen_url)
        }
    };

    // Recorded after the tabs lock is released, because this takes a different
    // lock and doing it above would set a lock order the rest of the file does
    // not follow.
    if let Some(url) = reopen_url {
        if !url.is_empty() && !url.starts_with("about:") {
            let browser = app.state::<Browser>();
            let mut closed = browser.closed.lock().expect("closed mutex poisoned");
            closed.push(url);
            // Trim from the front: the oldest entry is the one worth losing.
            if closed.len() > CLOSED_TAB_LIMIT {
                let excess = closed.len() - CLOSED_TAB_LIMIT;
                closed.drain(..excess);
            }
        }
    }

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

    relayout(app).map_err(|e| e.to_string())?;
    save_session(app);
    publish(app);
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
    // Clicking a tab is asking to see it, so the panel gets out of the way.
    dismiss_panel(&app);
    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

/// Navigates the active tab to whatever the address bar contained.
///
/// The search engine comes from settings rather than from the caller: the chrome
/// should not have to know, or be able to disagree about, which engine is current.
/// Async because it can close the panel, and closing the panel re-lays-out
/// webviews. Same rule as `set_panel`.
#[tauri::command]
pub async fn navigate(app: AppHandle, input: String) -> Result<(), String> {
    let target = {
        let settings = app.state::<crate::settings::SettingsState>();
        let engine_id = settings.get().search_engine;
        crate::search::resolve(&input, &engine_id, settings.is_dark(&app))
    };
    if target.is_empty() {
        return Ok(());
    }

    let url = target
        .parse()
        .map_err(|_| format!("That does not look like an address: {target}"))?;

    // After the parse, so a typo in the address bar does not close the panel on
    // its way to reporting itself.
    show_page(&app)?;

    active_webview(&app)?
        .navigate(url)
        .map_err(|e| e.to_string())
}

/// Closes the panel and applies the layout, for a command about to show a page.
///
/// Every caller is `async` for this reason: `relayout` moves and resizes
/// webviews, which is the thing sync commands must not do from the main thread.
fn show_page(app: &AppHandle) -> Result<(), String> {
    if dismiss_panel(app) {
        relayout(app).map_err(|e| e.to_string())?;
        publish(app);
    }
    Ok(())
}

/// Opens the system print dialog for the active tab.
///
/// WebView2's own dialog, not one of Brume's. Printing is a platform surface
/// with page setup, printer selection and a preview, and reimplementing it to
/// match the chrome would be a large amount of work for something people expect
/// to look exactly like it does everywhere else.
#[tauri::command]
pub fn print_page(app: AppHandle) -> Result<(), String> {
    active_webview(&app)?.print().map_err(|e| e.to_string())
}

/// Toggles fullscreen.
///
/// The chrome goes with it. Tauri's fullscreen covers the whole window, and the
/// content webview is positioned relative to that window, so `relayout` puts the
/// page over the full screen once the toolbar is out of the way.
#[tauri::command]
pub async fn toggle_fullscreen(app: AppHandle) -> Result<(), String> {
    let Some(window) = app.get_window(WINDOW_LABEL) else {
        return Ok(());
    };
    let now = window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(!now).map_err(|e| e.to_string())?;
    relayout(&app).map_err(|e| e.to_string())?;
    Ok(())
}

/// Navigates the active tab to the homepage.
///
/// Resolved at press time rather than cached, so changing the homepage or the
/// search engine in Settings takes effect without a restart. That matters
/// because an empty homepage means "follow the search engine", so the
/// destination can change without the homepage setting itself changing.
#[tauri::command]
pub async fn go_home(app: AppHandle) -> Result<(), String> {
    let target = home_url(&app);
    let url = target
        .parse()
        .map_err(|_| format!("Homepage is not a valid address: {target}"))?;

    show_page(&app)?;
    active_webview(&app)?
        .navigate(url)
        .map_err(|e| e.to_string())
}

/// Walks the active tab's history by `delta` entries.
fn traverse(app: &AppHandle, forward: bool) -> Result<(), String> {
    // The runtime decides whether there is anywhere to go. Brume no longer
    // tracks an index it could disagree about.
    let webview = active_webview(app)?;
    crate::history::go(&webview, forward)?;
    // No publish here. Moving fires the webview's own navigation and history
    // events, and those publish with values the runtime has actually settled on.
    Ok(())
}

/// Records what the runtime says about a tab's history, and republishes.
///
/// Called from history.rs when WebView2 reports a change. Separate from the
/// navigation handlers on purpose: whether a page can be gone back from is not
/// knowable at the moment navigation starts.
/// Records the zoom level the runtime reports, and republishes.
///
/// Compared with a small epsilon rather than `==`: the factor is a float that
/// WebView2 steps in fractions, and an exact match would republish on every
/// event even when nothing visible changed.
pub fn update_zoom(app: &AppHandle, tab_id: u32, zoom: f64) {
    {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if (tab.nav.zoom - zoom).abs() < 0.001 {
            return;
        }
        tab.nav.zoom = zoom;
    }
    publish(app);
}

/// Sets the active tab's zoom. `1.0` is 100%.
///
/// WebView2 handles Ctrl+scroll and Ctrl+plus/minus itself, so this is not how
/// zooming normally happens. It exists so the toolbar control can reset, and so
/// the level can be driven from a test, which is the only way to prove the
/// ZoomFactorChanged watcher actually fires.
#[tauri::command]
pub fn set_zoom(app: AppHandle, factor: f64) -> Result<(), String> {
    let Some(tab_id) = active_tab_id(&app) else {
        return Ok(());
    };
    let webview = active_webview(&app)?;
    let actual = crate::history::set_zoom(&webview, factor)?;

    // Updated from the value read back, not left to the event. A programmatic
    // SetZoomFactor does not raise ZoomFactorChanged, so waiting for it would
    // leave the indicator showing the old level forever.
    update_zoom(&app, tab_id, actual);
    Ok(())
}

/// Puts the active tab back to 100%.
#[tauri::command]
pub fn reset_zoom(app: AppHandle) -> Result<(), String> {
    set_zoom(app, 1.0)
}

pub fn update_traverse(app: &AppHandle, tab_id: u32, can_back: bool, can_forward: bool) {
    {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if tab.nav.can_back == can_back && tab.nav.can_forward == can_forward {
            return; // nothing the chrome would render differently
        }
        tab.nav.can_back = can_back;
        tab.nav.can_forward = can_forward;
    }
    publish(app);
}

#[tauri::command]
pub async fn go_back(app: AppHandle) -> Result<(), String> {
    show_page(&app)?;
    traverse(&app, false)
}

#[tauri::command]
pub async fn go_forward(app: AppHandle) -> Result<(), String> {
    show_page(&app)?;
    traverse(&app, true)
}

/// Reloads the active tab.
///
/// Uses the runtime's own reload rather than evaluating `location.reload()`.
/// Injected script only works on a document that hosts script in the first
/// place: a PDF in the built-in viewer, a bare image, or one of WebView2's own
/// error pages has no `location` worth calling, so the old approach failed
/// silently on exactly the pages a reload button is most wanted on.
#[tauri::command]
pub async fn reload(app: AppHandle) -> Result<(), String> {
    show_page(&app)?;
    active_webview(&app)?.reload().map_err(|e| e.to_string())
}

/// Stops the active tab loading.
///
/// Still `window.stop()`, because neither Tauri nor wry surfaces WebView2's
/// `Stop()` - checked, there is no `Webview::stop`. Reaching it means going
/// through `with_webview` to the ICoreWebView2 directly, which is worth doing
/// alongside the other interop work rather than on its own.
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

/// Pins or unpins a tab, moving it to keep pinned tabs at the front.
///
/// Reordered rather than merely flagged, because a pinned tab in the middle of
/// the strip is a pinned tab you still have to hunt for. Pinning moves it to the
/// end of the pinned run; unpinning moves it to the front of the unpinned one,
/// so it lands where it would sit if it had never been pinned.
#[tauri::command]
pub async fn set_tab_pinned(app: AppHandle, id: u32, pinned: bool) -> Result<(), String> {
    {
        let browser = app.state::<Browser>();
        let mut tabs = browser.tabs.lock().expect("tabs mutex poisoned");

        let Some(pos) = tabs.items.iter().position(|t| t.id == id) else {
            return Ok(());
        };
        if tabs.items[pos].pinned == pinned {
            return Ok(());
        }

        let mut tab = tabs.items.remove(pos);
        tab.pinned = pinned;
        // Both cases land at the same index: the boundary between the pinned run
        // and the rest.
        let boundary = tabs.items.iter().filter(|t| t.pinned).count();
        tabs.items.insert(boundary, tab);
    }
    save_session(&app);
    publish(&app);
    Ok(())
}

/// Opens a copy of a tab beside it.
///
/// The URL only. Duplicating a tab's back history would mean copying the
/// runtime's own history, which is not something WebView2 exposes.
#[tauri::command]
pub async fn duplicate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    let source = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        tabs.items
            .iter()
            .find(|t| t.id == id)
            .map(|t| (t.nav.current().cloned(), t.private))
    };

    let Some((Some(url), private)) = source else {
        return Ok(());
    };
    open_tab_inner(&app, Some(url), private).map_err(|e| e.to_string())
}

/// Closes every tab except `id`. Pinned tabs are kept.
#[tauri::command]
pub async fn close_other_tabs(app: AppHandle, id: u32) -> Result<(), String> {
    let doomed: Vec<u32> = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        tabs.items
            .iter()
            .filter(|t| t.id != id && !t.pinned)
            .map(|t| t.id)
            .collect()
    };

    for tab in doomed {
        close_tab_inner(&app, tab)?;
    }
    Ok(())
}

/// Closes every tab to the right of `id`. Pinned tabs are kept.
///
/// "Right" is position in the strip, so this reads the order rather than
/// comparing ids: ids ascend by creation, and pinning reorders.
#[tauri::command]
pub async fn close_tabs_to_right(app: AppHandle, id: u32) -> Result<(), String> {
    let doomed: Vec<u32> = {
        let browser = app.state::<Browser>();
        let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
        match tabs.items.iter().position(|t| t.id == id) {
            Some(pos) => tabs.items[pos + 1..]
                .iter()
                .filter(|t| !t.pinned)
                .map(|t| t.id)
                .collect(),
            None => Vec::new(),
        }
    };

    for tab in doomed {
        close_tab_inner(&app, tab)?;
    }
    Ok(())
}

/// Id of the tab at a zero-based position, for the Ctrl+1..8 bindings.
///
/// Returns `None` when there is no tab there, so Ctrl+5 with three tabs open
/// does nothing rather than jumping somewhere arbitrary.
pub fn tab_id_at(app: &AppHandle, index: usize) -> Option<u32> {
    let browser = app.state::<Browser>();
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
    tabs.items.get(index).map(|t| t.id)
}

/// Id of the last tab, for Ctrl+9.
///
/// Ctrl+9 means "last tab", not "ninth tab", in every mainstream browser - it is
/// the one number that is positional rather than an index.
pub fn last_tab_id(app: &AppHandle) -> Option<u32> {
    let browser = app.state::<Browser>();
    let tabs = browser.tabs.lock().expect("tabs mutex poisoned");
    tabs.items.last().map(|t| t.id)
}

/// Reopens the most recently closed tab. Does nothing when none were closed.
///
/// Async for the same reason the tab commands are: it creates a webview.
pub async fn reopen_closed_tab(app: AppHandle) -> Result<(), String> {
    let url = {
        let browser = app.state::<Browser>();
        let mut closed = browser.closed.lock().expect("closed mutex poisoned");
        closed.pop()
    };

    match url {
        Some(url) => open_tab_inner(&app, Some(url), false).map_err(|e| e.to_string()),
        None => Ok(()),
    }
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

/// Shows or hides the find bar, resizing the page to make room.
///
/// Async for the same reason `set_panel` is: it re-lays-out webviews.
///
/// Closing it also stops the search, so the highlights go with the bar. Leaving
/// a page full of highlighted matches after the bar is gone would be a state
/// with no visible way to clear it.
#[tauri::command]
pub async fn set_find_bar(app: AppHandle, open: bool) -> Result<(), String> {
    app.state::<Browser>()
        .find_open
        .store(open, Ordering::Relaxed);

    if !open {
        // Best effort: a tab with no page, or an older runtime with no find
        // support, should still be able to close the bar.
        let _ = crate::find::find_stop(app.clone());
    }

    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

/// Shows or hides the bookmarks bar, resizing the page to make room.
///
/// Async for the same reason `set_find_bar` is: it re-lays-out webviews.
///
/// The preference goes to settings rather than to an in-memory flag, so the bar
/// is still there next launch. That also makes settings the one place holding
/// it, which is what `chrome_extent` reads.
#[tauri::command]
pub async fn set_bookmarks_bar(app: AppHandle, show: bool) -> Result<(), String> {
    app.state::<crate::settings::SettingsState>()
        .set_show_bookmarks_bar(show)?;

    relayout(&app).map_err(|e| e.to_string())?;
    publish(&app);
    Ok(())
}

/// Flips the bookmarks bar. Ctrl+Shift+B, and nothing else needs a toggle.
#[tauri::command]
pub async fn toggle_bookmarks_bar(app: AppHandle) -> Result<(), String> {
    let showing = app
        .state::<crate::settings::SettingsState>()
        .show_bookmarks_bar();
    set_bookmarks_bar(app, !showing).await
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

    // The star comes back through publish, but the bar renders the whole list
    // and would otherwise not hear that it just gained or lost a row.
    let _ = app.emit_to(CHROME_LABEL, BOOKMARKS_EVENT, ());
    publish(&app);
    Ok(bookmarked)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The traversal tests that used to live here are gone with the stack they
    // exercised. Back and forward are WebView2's now, and verifying them needs a
    // running webview rather than a unit test - done through tools/cdp.ps1.
    // What is left here is the part Brume still decides for itself.

    #[test]
    fn a_fresh_tab_has_no_url_and_cannot_move() {
        let nav = NavState::default();
        assert_eq!(nav.current(), None);
        // Both mirror the runtime and start false, so a new tab's back button is
        // disabled until WebView2 says otherwise.
        assert!(!nav.can_back);
        assert!(!nav.can_forward);
    }

    #[test]
    fn an_empty_url_is_not_a_url() {
        let mut nav = NavState::default();
        nav.set_url(String::new());
        assert_eq!(
            nav.current(),
            None,
            "empty should read as nowhere, not as \"\""
        );

        nav.set_url("https://a.test/".into());
        assert_eq!(nav.current().map(String::as_str), Some("https://a.test/"));
    }

    #[test]
    fn display_title_falls_back_to_the_host_then_to_new_tab() {
        let mut tab = Tab {
            id: 0,
            label: "tab-0".into(),
            title: String::new(),
            nav: NavState::default(),
            private: false,
            pinned: false,
        };
        assert_eq!(tab.display_title(), "New tab");

        tab.nav.set_url("https://www.example.com/deep/path".into());
        // www. is stripped: it is noise in a tab that is only ~140px wide.
        assert_eq!(tab.display_title(), "example.com");

        tab.title = "  ".into();
        assert_eq!(
            tab.display_title(),
            "example.com",
            "blank title is not a title"
        );

        tab.title = "Real Page Title".into();
        assert_eq!(tab.display_title(), "Real Page Title");
    }

    /// The four heights the chrome can be, spelled out.
    ///
    /// These are a contract with src/index.html, where the same numbers are CSS.
    /// Nothing can check the two agree at build time, so the values are written
    /// out here rather than recomputed from the constants: a test that says
    /// `CHROME_HEIGHT + FIND_BAR_HEIGHT` would pass just as happily after
    /// someone changed one of them and forgot the stylesheet.
    #[test]
    fn the_chrome_grows_by_each_open_bar() {
        assert_eq!(extent_for(false, false), 76.0, "tab strip plus toolbar");
        assert_eq!(extent_for(true, false), 112.0, "plus the find bar");
        assert_eq!(extent_for(false, true), 108.0, "plus the bookmarks bar");
        // Both at once is the case a branch instead of a sum would get wrong.
        assert_eq!(extent_for(true, true), 144.0, "plus both");
    }
}
