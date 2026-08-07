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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

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

/// How far a new window is offset from the last one, in logical pixels.
///
/// Enough that the title bar and one edge of the window underneath stay visible.
/// A window opening exactly on top of another is indistinguishable from nothing
/// having happened.
const NEW_WINDOW_OFFSET: f64 = 32.0;

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

/// Path of Brume's own new tab page, under the chrome's origin.
///
/// The only path a content webview is allowed to load from that origin; see the
/// navigation guard.
const NEW_TAB_PATH: &str = "/newtab.html";

/// Brume's new tab page, with the theme along for the ride.
///
/// The page holds no capabilities and cannot ask what the theme is, so it is
/// told. Without it a new tab would always paint dark and would look wrong for
/// the one setting it exists to match.
fn new_tab_url(app: &AppHandle) -> String {
    let theme = app.state::<crate::settings::SettingsState>().get().theme;
    format!("http://tauri.localhost{NEW_TAB_PATH}?theme={theme}")
}

/// Where a new tab should open.
///
/// An explicit homepage wins. Otherwise Brume's own page, which is **not** what
/// this used to do: the old default sent every new tab to the search engine's
/// landing page, so parking ten tabs told a search engine ten times that you
/// were there, before you had typed anything. See BUILD_NOTES.
fn home_url(app: &AppHandle) -> String {
    let resolved = app
        .state::<crate::settings::SettingsState>()
        .resolved_homepage(app);
    if resolved.is_empty() {
        new_tab_url(app)
    } else {
        resolved
    }
}

/// Whether a URL is Brume's new tab page.
///
/// Used to keep it out of history and out of the address bar. Compared on host
/// and path so the theme query string does not matter.
pub fn is_new_tab(url: &str) -> bool {
    tauri::Url::parse(url)
        .is_ok_and(|u| u.host_str() == Some("tauri.localhost") && u.path() == NEW_TAB_PATH)
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
    /// Whether the page is making noise, and whether it has been silenced.
    ///
    /// Both mirrored from the runtime by audio.rs, like `can_back` and `zoom`.
    /// Nothing here works either of them out.
    audible: bool,
    muted: bool,
    /// Whether this tab has a webview behind it yet.
    ///
    /// False only for a tab restored from the last session and not yet looked
    /// at. Restoring used to build a webview and load the page for every saved
    /// tab at launch, so twenty restored tabs meant twenty renderers and twenty
    /// page loads before the window was usable. A parked tab costs a row in the
    /// strip and nothing else until it is activated.
    ///
    /// Deliberately only used for restore, never on a timer. Throwing away a
    /// live webview to reclaim memory would also throw away scroll position and
    /// anything typed into a form, and a tab you glanced away from for a minute
    /// is not worth that. A tab that was never loaded has nothing to lose.
    loaded: bool,
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

/// Everything that belongs to one window.
///
/// All of this was global until 0.7.0, when it stopped being able to be. Tabs,
/// the panel, the find bar and the overlay are each a property of one window's
/// chrome, and a second window has its own of every one of them.
pub struct WindowState {
    /// This window's own label, so state can find its way back to its window.
    window: String,
    /// The label of this window's chrome webview.
    ///
    /// **Must keep the `chrome-` prefix.** `capabilities/default.json` grants
    /// Brume's commands to `chrome-*` and nothing else, and content webviews are
    /// `tab-*`. The two namespaces staying disjoint is the whole protection: a
    /// content webview that ever matched `chrome-*` would be handed the IPC
    /// bridge for every site on the internet.
    chrome: String,
    tabs: Mutex<Tabs>,
    /// URLs of recently closed tabs, most recent last, for Ctrl+Shift+T.
    ///
    /// Only the URL is kept, not the tab's whole history. Reopening restores
    /// where you were, not the trail you took to get there - which is what the
    /// shortcut is actually for, and it avoids persisting a per-tab history
    /// stack for tabs that no longer exist.
    ///
    /// Per window, so Ctrl+Shift+T in one window cannot resurrect a tab closed
    /// in another one that is still open next to it.
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
    /// How far down the chrome document an open overlay reaches, in whole
    /// logical pixels from its top. 0 means nothing is overlaying.
    ///
    /// The chrome sends the **bottom edge**, not a height, so that the bar
    /// heights stay in one place. A height would have to be measured against
    /// `chrome_extent`, which the chrome would then need its own copy of, and
    /// the find bar contract already shows where duplicated constants end up.
    ///
    /// Deliberately **not** part of `chrome_extent`. Every other bar takes space
    /// away from the page; an overlay covers the top of it instead, so the page
    /// neither moves nor resizes while it is open. See overlay.rs.
    ///
    /// An integer because there is no atomic f64 and a pixel is a pixel.
    overlay_bottom: AtomicU32,
    /// The tab shown beside the active one, when the window is split.
    ///
    /// `None` is the ordinary case and the one every version before 0.8.0 had.
    /// A second visible content webview is the assumption that survived even the
    /// multi-window refactor, so this is the first thing to break it.
    ///
    /// Held as a tab id rather than a label, for the reason `window_of_tab`
    /// exists: an id survives things a label does not have to.
    split: Mutex<Option<u32>>,
}

/// Gap between the two panes, in logical pixels.
///
/// Wide enough to read as two panes rather than one page with a seam down it.
/// The window background shows through, which is why nothing has to draw it.
const SPLIT_GAP: f64 = 2.0;

pub struct Browser {
    /// Per-window state, keyed by window label.
    ///
    /// `Arc` so a caller can take its window's state and drop this lock before
    /// doing anything with it. Holding the map while laying a window out would
    /// serialise every window against every other one, and worse, a command that
    /// opened a window while holding it would deadlock.
    windows: Mutex<HashMap<String, Arc<WindowState>>>,
    /// Tab ids, allocated across every window rather than per window.
    ///
    /// Globally unique because a webview label is `tab-{id}` and labels are
    /// unique app-wide. Per-window counters would hand two windows the same
    /// `tab-1`, and the second `add_child` would collide with the first.
    ///
    /// Note that a tab does **not** keep its id when it moves between windows:
    /// `move_tab_to_new_window` rebuilds it, so it arrives with a fresh one.
    /// Measured, 2026-08-07. WebView2 gives no way to reparent a webview, which
    /// is the same reason the move loses scroll position.
    next_tab_id: AtomicU32,
    /// Suffix for the next window label. `main` is 1 and never reused.
    next_window: AtomicU32,
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
fn chrome_extent(app: &AppHandle, state: &WindowState) -> f64 {
    let find_open = state.find_open.load(Ordering::Relaxed);
    // Read from settings rather than mirrored into `Browser`. One source of
    // truth: a copy here would be the thing that eventually disagrees with the
    // file after a failed write.
    //
    // Still app-wide rather than per window: the bookmarks bar is a preference,
    // and a bar that was showing in one window and not the next would read as a
    // bug rather than a feature.
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
            windows: Mutex::new(HashMap::new()),
            next_tab_id: AtomicU32::new(1),
            // `main` takes 1, so the next window is `win-2`. Never reused, for
            // the same reason tab labels are not: a stale handler writing into
            // a window that took a recycled label is not worth the tidiness.
            next_window: AtomicU32::new(2),
        }
    }
}

impl WindowState {
    fn new(window: &str) -> Self {
        Self {
            window: window.to_string(),
            chrome: chrome_label(window),
            tabs: Mutex::new(Tabs::default()),
            closed: Mutex::new(Vec::new()),
            panel_open: AtomicBool::new(false),
            find_open: AtomicBool::new(false),
            overlay_bottom: AtomicU32::new(0),
            split: Mutex::new(None),
        }
    }
}

impl Browser {
    /// This window's state, created on first use.
    fn window(&self, label: &str) -> Arc<WindowState> {
        self.windows
            .lock()
            .expect("windows mutex poisoned")
            .entry(label.to_string())
            .or_insert_with(|| Arc::new(WindowState::new(label)))
            .clone()
    }

    /// Forgets a window's state once it has closed.
    fn forget(&self, label: &str) {
        self.windows
            .lock()
            .expect("windows mutex poisoned")
            .remove(label);
    }

    /// Every open window's state.
    fn all(&self) -> Vec<Arc<WindowState>> {
        self.windows
            .lock()
            .expect("windows mutex poisoned")
            .values()
            .cloned()
            .collect()
    }

    fn next_tab_id(&self) -> u32 {
        self.next_tab_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// The chrome webview label for a window.
///
/// See `WindowState::chrome`: the `chrome-` prefix is what the capability file
/// matches on, so this is a security boundary rather than a naming convention.
pub fn chrome_label(window_label: &str) -> String {
    format!("chrome-{window_label}")
}

/// The state of the window a command came from.
///
/// Commands take `tauri::Window` and hand it here. Before 0.7.0 they read one
/// global `Browser`, which is why so many signatures changed at once.
pub fn state_for(app: &AppHandle, window_label: &str) -> Arc<WindowState> {
    app.state::<Browser>().window(window_label)
}

/// The window holding a given tab.
///
/// A scan rather than a `window` field on `Tab`. There are a handful of windows
/// and a few dozen tabs, so the cost is nothing, and it means moving a tab
/// between windows is only moving it between two lists: there is no second
/// record of where it lives that could be left pointing at the old one.
fn window_of_tab(app: &AppHandle, id: u32) -> Option<Arc<WindowState>> {
    app.state::<Browser>().all().into_iter().find(|w| {
        w.tabs
            .lock()
            .expect("tabs mutex poisoned")
            .items
            .iter()
            .any(|t| t.id == id)
    })
}

/// The window holding the tab with this webview label.
fn window_of_tab_label(app: &AppHandle, label: &str) -> Option<Arc<WindowState>> {
    app.state::<Browser>().all().into_iter().find(|w| {
        w.tabs
            .lock()
            .expect("tabs mutex poisoned")
            .items
            .iter()
            .any(|t| t.label == label)
    })
}

/// The window the user is actually in.
///
/// For callers with no window to hand: a global shortcut fires against whatever
/// is focused, and a tray or menu action has no originating webview at all.
/// Falls back to any window, so a keystroke arriving in the gap between one
/// window losing focus and the next gaining it still does something sensible
/// rather than nothing.
pub fn focused_state(app: &AppHandle) -> Option<Arc<WindowState>> {
    let browser = app.state::<Browser>();
    let all = browser.all();
    all.iter()
        .find(|w| {
            app.get_window(&w.window)
                .and_then(|win| win.is_focused().ok())
                .unwrap_or(false)
        })
        .cloned()
        .or_else(|| all.first().cloned())
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
    pub audible: bool,
    pub muted: bool,
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
    /// The tab shown beside the active one, or `None` when not split.
    ///
    /// Published so the strip can mark it. Without that, a split tab looks like
    /// an ordinary background tab while occupying half the window, which reads
    /// as the browser having lost track of what it is showing.
    pub split: Option<u32>,
}

fn snapshot(
    tabs: &Tabs,
    bookmarked: bool,
    bookmarks_bar: bool,
    panel_open: bool,
    split: Option<u32>,
) -> BrowserState {
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
                audible: t.audible,
                muted: t.muted,
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
        // Reported as absent once the tab is gone, so a split that outlived its
        // partner cannot leave the strip marking a tab that is not there.
        split: split.filter(|id| tabs.items.iter().any(|t| t.id == *id)),
    }
}

/// Builds a snapshot, consulting the bookmark store for the active URL.
///
/// Split out because the bookmark lookup needs its own lock, and taking it while
/// holding the tabs lock is exactly the pattern that turns into a deadlock the
/// first time something calls in the other order. The settings read below is
/// there for the same reason and is deliberately done before the tabs lock too.
fn current_state(app: &AppHandle, state: &WindowState) -> BrowserState {
    let active_url = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
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

    let panel_open = state.panel_open.load(Ordering::Relaxed);
    let split = *state.split.lock().expect("split mutex poisoned");
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    snapshot(&tabs, bookmarked, bookmarks_bar, panel_open, split)
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
fn dismiss_panel(state: &WindowState) -> bool {
    state.panel_open.swap(false, Ordering::Relaxed)
}

/// Pushes current state to the chrome.
///
/// The chrome never asks; it is told. A one-way feed means the tab strip and the
/// buttons cannot disagree with reality after a link click, a redirect, or a
/// background tab finishing its load.
fn publish(app: &AppHandle, win: &WindowState) {
    let state = current_state(app, win);

    // The window title tracks the active tab, the way every browser's does.
    // Driven from the same place as the rest of the state so the two cannot
    // disagree - a title left showing a page you have navigated away from is a
    // small thing that reads as broken.
    if let Some(window) = app.get_window(&win.window) {
        // The tab's *raw* title, not its display title.
        //
        // TabView carries display_title(), which falls back to the host and then
        // to "New tab" so the tab strip never shows a blank. Reading that here
        // meant the empty-title branch below could never be taken, and a freshly
        // launched Brume announced itself as "New tab - Brume" in the taskbar.
        // The window wants the opposite fallback: no page title yet means the
        // application name on its own.
        let raw_title = {
            let tabs = win.tabs.lock().expect("tabs mutex poisoned");
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

    let _ = app.emit_to(&win.chrome, "brume://state", state);
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Positions the chrome and the active tab, and hides the rest.
///
/// Child webviews take no part in any layout system - they are rectangles the
/// host places by hand, so this runs on every resize and scale-factor change.
fn relayout(app: &AppHandle, state: &WindowState) -> tauri::Result<()> {
    let Some(window) = app.get_window(&state.window) else {
        return Ok(());
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);

    let panel_open = state.panel_open.load(Ordering::Relaxed);

    // How tall the chrome is when it is only chrome. Grows with the find bar.
    let extent = chrome_extent(app, state);

    // An overlay grows the chrome without taking anything from the page.
    // overlay.rs raises the chrome above the content webview so the extra height
    // covers the top of the page instead of pushing it down.
    let overlay = state.overlay_bottom.load(Ordering::Relaxed) as f64;

    // With the panel open the chrome takes the whole window and every content
    // webview is hidden. That is what keeps history, bookmarks and settings
    // inside the one webview that holds capabilities, instead of needing a
    // privileged tab alongside arbitrary websites.
    let chrome_height = if panel_open {
        size.height
    } else {
        // Whichever is lower: the bars, or the bottom of an open overlay.
        extent.max(overlay).min(size.height)
    };

    // A window can be dragged smaller than the chrome mid-resize; clamping stops
    // the content webview being handed a negative height.
    let content_height = (size.height - extent).max(0.0);

    if let Some(chrome) = app.get_webview(&state.chrome) {
        chrome.set_position(LogicalPosition::new(0.0, 0.0))?;
        chrome.set_size(LogicalSize::new(size.width, chrome_height))?;
    }

    // The split partner, if there is one and it is still open. Resolved here so
    // a tab closed while split simply falls back to one pane rather than
    // leaving half the window empty.
    let split_id = *state.split.lock().expect("split mutex poisoned");

    let (active_label, split_label, inactive_labels) = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let split_label = split_id
            .filter(|id| *id != tabs.active)
            .and_then(|id| tabs.items.iter().find(|t| t.id == id))
            .map(|t| t.label.clone());
        (
            tabs.active_tab().map(|t| t.label.clone()),
            split_label.clone(),
            tabs.items
                .iter()
                .filter(|t| t.id != tabs.active)
                .filter(|t| Some(&t.label) != split_label.as_ref())
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

    // Two panes when split, one otherwise. Split down the middle rather than at
    // a draggable ratio: a divider is its own interaction, and half is the
    // useful case. The gap is left as window background, so nothing draws it.
    let (left_width, right_x, right_width) = match split_label {
        Some(_) => {
            let half = ((size.width - SPLIT_GAP) / 2.0).max(0.0);
            (half, half + SPLIT_GAP, half)
        }
        None => (size.width, 0.0, 0.0),
    };

    if let Some(label) = active_label {
        if let Some(view) = app.get_webview(&label) {
            if panel_open {
                let _ = view.hide();
            } else {
                view.set_position(LogicalPosition::new(0.0, extent))?;
                view.set_size(LogicalSize::new(left_width, content_height))?;
                let _ = view.show();
            }
        }
    }

    if let Some(label) = split_label {
        if let Some(view) = app.get_webview(&label) {
            if panel_open {
                let _ = view.hide();
            } else {
                view.set_position(LogicalPosition::new(right_x, extent))?;
                view.set_size(LogicalSize::new(right_width, content_height))?;
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
    state: &WindowState,
    id: u32,
    label: &str,
    url: &str,
    private: bool,
) -> tauri::Result<()> {
    // An error, not a silent Ok. Returning Ok here registered a tab that had no
    // webview behind it and reported success, which is the worst of both: the
    // tab strip grew a row that rendered nothing and nothing anywhere said why.
    let Some(window) = app.get_window(&state.window) else {
        return Err(tauri::Error::WindowNotFound);
    };

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);
    // Matches whatever the chrome currently occupies, so a tab opened while the
    // find bar is up is not born 36px too tall and overlapping it.
    let extent = chrome_extent(app, state);
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
                    // Into the window the opening tab is in, so a link opened
                    // from a second window does not land in the first.
                    let Some(state) = window_of_tab(&handle, id) else {
                        return;
                    };
                    if let Err(e) = open_tab_inner(&handle, &state, Some(target), opener_private) {
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
                // Every window: the downloads list is app-wide, so a panel open
                // in another window is just as stale.
                notify_downloads_everywhere(&download_handle);
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
                // One exception, kept as narrow as it can be: Brume's own new
                // tab page, which has to live somewhere a content webview can
                // actually load. Matched on the exact path, so nothing else
                // under that origin is reachable.
                //
                // What a page gains by navigating itself here is nothing. It
                // holds no capabilities either way, and the moment it navigates
                // it stops being able to render anything of its own. The address
                // bar keeps showing the real URL, so there is nothing to
                // impersonate.
                let own_origin = url
                    .host_str()
                    .is_some_and(|h| h == "tauri.localhost" || h == "asset.localhost");
                if own_origin && url.path() != NEW_TAB_PATH {
                    eprintln!("[browser] refused navigation to Brume's own origin: {url}");
                    return false;
                }

                // Fires for every navigation, including ones Brume did not
                // start: link clicks, form submissions, redirects, JavaScript.
                // Recording here rather than only in `navigate` is what keeps
                // the address bar honest.
                // Looked up rather than captured. The window is known when this
                // webview is built, but a tab can be moved to another one
                // afterwards, and a captured label would then update the window
                // the tab used to be in.
                if let Some(win) = window_of_tab(&nav_handle, id) {
                    {
                        let mut tabs = win.tabs.lock().expect("tabs mutex poisoned");
                        if let Some(tab) = tabs.tab_mut(id) {
                            tab.nav.set_url(url.to_string());
                            tab.nav.loading = true;
                            // The old page's title does not describe the new one.
                            tab.title.clear();
                        }
                    }
                    publish(&nav_handle, &win);
                }
                true
            })
            .on_page_load(move |_webview, payload| {
                let finished = matches!(payload.event(), PageLoadEvent::Finished);

                // What to record is decided while holding the tabs lock, but the
                // store is written after releasing it. Taking the store's lock
                // while holding this one would establish a lock order that the
                // bookmark path takes in reverse.
                let Some(win) = window_of_tab(&load_handle, id) else {
                    return;
                };
                let visit = {
                    let mut tabs = win.tabs.lock().expect("tabs mutex poisoned");
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
                    // Brume's own new tab page is not somewhere you went. It
                    // would otherwise be far and away the most visited entry in
                    // history, and it is the one page a back button should never
                    // need to return to.
                    if !is_new_tab(&url) {
                        load_handle
                            .state::<crate::store::Store>()
                            .record_visit(&url, &title);
                    }

                    // The tab has settled somewhere new, so the saved session is
                    // stale. Written here rather than only at quit, so an
                    // unexpected exit still has something to restore.
                    save_session(&load_handle);

                    // On finish rather than on navigation start: applying it
                    // earlier means the runtime resets the factor as the new
                    // document commits, and the zoom silently does not stick.
                    apply_site_zoom(&load_handle, id, &url);
                }

                publish(&load_handle, &win);
            })
            .on_document_title_changed(move |_webview, title| {
                // Titles arrive from the runtime, not from the page over IPC.
                //
                // That distinction matters: the content webview is deliberately
                // outside every capability, so a website cannot call a Brume
                // command. Having pages report their own titles would have meant
                // opening an IPC channel to the entire internet for a cosmetic
                // feature.
                if let Some(win) = window_of_tab(&title_handle, id) {
                    {
                        let mut tabs = win.tabs.lock().expect("tabs mutex poisoned");
                        if let Some(tab) = tabs.tab_mut(id) {
                            tab.title = title;
                        }
                    }
                    publish(&title_handle, &win);
                }
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
    // Ordered by window label so the restored order is stable rather than
    // whatever the map happened to iterate in. `main` sorts before `win-N`,
    // which is also the order they were opened in.
    let mut states = app.state::<Browser>().all();
    states.sort_by(|a, b| a.window.cmp(&b.window));

    let windows: Vec<crate::settings::SessionWindow> = states
        .iter()
        .map(|state| {
            let tabs = state.tabs.lock().expect("tabs mutex poisoned");

            // Private tabs are left out: a session file naming them would
            // outlive the browsing they were supposed to keep off disk.
            let saved: Vec<crate::settings::SessionTab> = tabs
                .items
                .iter()
                .filter(|t| !t.private)
                .filter_map(|t| {
                    t.nav.current().and_then(|u| {
                        // A tab still on about:blank has nowhere to restore to.
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

            crate::settings::SessionWindow {
                tabs: saved,
                active,
            }
        })
        // A window whose tabs were all private has nothing to restore, and an
        // empty window on next launch would be a puzzle rather than a session.
        .filter(|w| !w.tabs.is_empty())
        .collect();

    let _ = app
        .state::<crate::settings::SettingsState>()
        .set_session(windows);
}

/// Builds the first window and restores the session into it and any others.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let session = app.state::<crate::settings::SettingsState>().session();

    // The first window takes the saved geometry; any others are offset from it
    // by `build_window`. One saved geometry rather than one per window, because
    // a window is identified by its label and labels are not stable across runs:
    // remembering where "win-3" was tells you nothing about which window that is
    // the next time.
    let first = session.first().cloned().unwrap_or_default();
    build_window(app, WINDOW_LABEL, Some(&first), true)?;

    // Everything after the first. Restored in order, each with its own tabs.
    for saved in session.iter().skip(1) {
        let label = next_window_label(app);
        if let Err(e) = build_window(app, &label, Some(saved), false) {
            // One window failing is not a reason to lose the rest of the
            // session, or to leave the browser with nothing on screen.
            eprintln!("[browser] could not restore a window: {e}");
        }
    }

    Ok(())
}

/// The label for the next window. `main` is taken; the rest are `win-N`.
fn next_window_label(app: &AppHandle) -> String {
    let n = app
        .state::<Browser>()
        .next_window
        .fetch_add(1, Ordering::Relaxed);
    format!("win-{n}")
}

/// Builds one window, its chrome, and its tabs.
///
/// `restore` is the session for this window, or `None` for a plain new window
/// which gets a single homepage tab. `use_geometry` applies the saved size and
/// position, which only the first window does: a second window opening exactly
/// on top of the first is indistinguishable from nothing having happened.
fn build_window(
    app: &AppHandle,
    label: &str,
    restore: Option<&crate::settings::SessionWindow>,
    use_geometry: bool,
) -> tauri::Result<()> {
    let state = state_for(app, label);
    let saved = if use_geometry {
        app.state::<crate::settings::SettingsState>().window()
    } else {
        None
    };

    // Size is taken from the saved geometry, but position is applied *after* the
    // window exists - checking whether coordinates are on a real display needs
    // the monitor list, and that comes off a window.
    let (width, height) = match saved {
        Some(g) if g.width >= 480.0 && g.height >= 360.0 => (g.width, g.height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };

    let window = tauri::window::WindowBuilder::new(app, label)
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
    } else if !use_geometry {
        // Offset from wherever the last window is, so a new one is visibly a
        // new one rather than sitting exactly on top of the old.
        if let Ok(pos) = window.outer_position() {
            let scale = window.scale_factor().unwrap_or(1.0);
            let logical: LogicalPosition<f64> = pos.to_logical(scale);
            let _ = window.set_position(LogicalPosition::new(
                logical.x + NEW_WINDOW_OFFSET,
                logical.y + NEW_WINDOW_OFFSET,
            ));
        }
    }

    let scale = window.scale_factor()?;
    let size: LogicalSize<f64> = window.inner_size()?.to_logical(scale);

    window.add_child(
        WebviewBuilder::new(&state.chrome, WebviewUrl::App("index.html".into()))
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

    // Rebuild this window's tabs, or open the homepage if there are none.
    //
    // Restored in order, then the previously active one is brought to the front.
    // A tab that fails to build is skipped rather than aborting the launch: one
    // bad saved URL should not leave the browser with no window worth looking at.
    let empty = crate::settings::SessionWindow::default();
    let restore = restore.unwrap_or(&empty);
    let session = &restore.tabs;
    let session_active = restore.active;

    if session.is_empty() {
        open_tab_inner(app, &state, None, false)?;
    } else {
        // Parked, not opened. Every saved tab used to get a webview and a page
        // load right here, so a session of twenty tabs meant twenty renderers
        // and twenty network requests before the window was usable, for pages
        // nobody had asked to see yet. Now only the one that was in front is
        // built, below; the rest load when they are first clicked.
        for saved in session {
            park_tab(app, &state, saved.url.clone(), saved.pinned);
        }
        // Sorted once, after the lot are in. Stable, so tabs keep the order they
        // were saved in within each group.
        {
            let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
            tabs.items.sort_by_key(|t| !t.pinned);
        }
        // Whichever was in front last time comes back in front, falling back to
        // the first if the saved index no longer points at anything.
        let front = tab_id_at_in(&state, session_active).or_else(|| tab_id_at_in(&state, 0));
        if let Some(id) = front {
            {
                let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
                tabs.active = id;
            }
            // The only one built at launch. A failure here is not fatal: the
            // fallback below still leaves a usable window.
            if let Err(e) = load_parked(app, &state, id) {
                eprintln!("[browser] could not load the restored tab: {e}");
            }
        }
        // Nothing restored, so there is no tab at all. Fall back rather than
        // leave an empty window.
        if tab_id_at_in(&state, 0).is_none() {
            open_tab_inner(app, &state, None, false)?;
        }
        relayout(app, &state)?;
        publish(app, &state);
    }

    let event_handle = app.clone();
    let event_state = state.clone();
    let geometry_owner = use_geometry;
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let _ = relayout(&event_handle, &event_state);
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
            // Only the first window writes geometry. Every window writing it
            // would mean the last one closed decides where the *first* one
            // opens next time, which is not what anyone moved it for.
            if geometry_owner {
                save_geometry(&event_handle);
            }
            // Saved before the state is forgotten, so this window's tabs are
            // still in the list being written.
            save_session(&event_handle);
        }
        // Destroyed rather than CloseRequested: the close can still be
        // cancelled, and forgetting a window that then stays open would leave
        // it with no tabs and no way to get any.
        WindowEvent::Destroyed => {
            event_handle.state::<Browser>().forget(&event_state.window);
            // Written again now the window is really gone, or a session saved
            // moments earlier would restore a window the user just closed.
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
    let state = focused_state(app).ok_or_else(|| "No window.".to_string())?;
    active_webview(app, &state)
}

fn active_webview(app: &AppHandle, state: &WindowState) -> Result<tauri::webview::Webview, String> {
    let label = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        tabs.active_tab().map(|t| t.label.clone())
    };

    label
        .and_then(|l| app.get_webview(&l))
        .ok_or_else(|| "No active tab.".to_string())
}

/// Adds a tab with no webview behind it, for session restore.
///
/// The strip gets a row with the saved URL and a title guessed from the host, so
/// it looks like any other tab. The webview is built by `activate_tab` the first
/// time it is actually looked at.
fn park_tab(app: &AppHandle, state: &WindowState, url: String, pinned: bool) -> u32 {
    let id = app.state::<Browser>().next_tab_id();
    let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");

    let label = format!("tab-{id}");

    let mut nav = NavState::default();
    nav.set_url(url);

    tabs.items.push(Tab {
        id,
        label,
        title: String::new(), // display_title falls back to the host
        nav,
        private: false, // a private tab is never saved, so never restored
        pinned,
        audible: false,
        muted: false,
        loaded: false,
    });
    id
}

/// Builds the webview for a parked tab, if it has not got one yet.
///
/// Everything a live tab needs is set up here rather than at restore, which is
/// the whole point: a restored tab you never click costs nothing.
fn load_parked(app: &AppHandle, state: &WindowState, id: u32) -> tauri::Result<()> {
    let Some((label, url)) = ({
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        tabs.items
            .iter()
            .find(|t| t.id == id && !t.loaded)
            .map(|t| {
                (
                    t.label.clone(),
                    t.nav.current().cloned().unwrap_or_default(),
                )
            })
    }) else {
        return Ok(()); // already loaded, or gone
    };

    let target = if url.is_empty() { home_url(app) } else { url };
    spawn_tab_webview(app, state, id, &label, &target, false)?;

    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        if let Some(tab) = tabs.tab_mut(id) {
            tab.loaded = true;
        }
    }

    crate::history::watch(app, id, &label);
    crate::history::watch_zoom(app, id, &label);
    crate::downloads::watch(app, &label);
    crate::permissions::watch(app, &label);
    crate::audio::watch(app, id, &label);
    crate::contextmenu::watch(app, &label);
    Ok(())
}

/// The real implementation, in Tauri's error type.
///
/// Split from the command so that `build` - which returns `tauri::Result` - can
/// open the first tab through the same path, instead of the command's
/// stringly-typed error being forced through a conversion at the call site.
fn open_tab_inner(
    app: &AppHandle,
    state: &WindowState,
    url: Option<String>,
    private: bool,
) -> tauri::Result<()> {
    let target = url.unwrap_or_else(|| home_url(app));

    // Before the layout below, not after: a tab opened from the panel has to end
    // up visible. The relayout at the end of this function is what applies it.
    dismiss_panel(state);

    let id = app.state::<Browser>().next_tab_id();
    let (label, previous_active) = {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");

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
            // Both start false and are corrected by audio.rs the moment the
            // runtime has anything to say. A tab cannot be audible before it
            // has loaded a page.
            audible: false,
            muted: false,
            loaded: true,
        });
        tabs.active = id;

        (label, previous_active)
    };

    // Roll back if the webview cannot be created.
    //
    // The tab has to be registered first, because the webview's own handlers
    // fire during construction and look it up by id. But leaving it there on
    // failure produced a tab with nothing behind it that was also the *active*
    // tab, so every subsequent navigate, reload and back resolved to a webview
    // that did not exist and failed. The chrome raises a dialog per failed
    // command, so the browser became unusable until that tab was closed.
    let spawned = spawn_tab_webview(app, state, id, &label, &target, private);

    // Subscribe to the runtime's history events once the webview exists. Only
    // on success: there is nothing to watch otherwise.
    if spawned.is_ok() {
        crate::history::watch(app, id, &label);
        crate::history::watch_zoom(app, id, &label);
        // Progress only. The start and finish records still come from
        // `on_download` above; this adds the byte count in between.
        crate::downloads::watch(app, &label);
        // Without this the runtime answers permission requests with its own
        // prompt, and Brume neither hears about it nor can change it later.
        crate::permissions::watch(app, &label);
        crate::audio::watch(app, id, &label);
        crate::contextmenu::watch(app, &label);
    }

    if let Err(e) = spawned {
        {
            let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
            tabs.items.retain(|t| t.id != id);
            // Only restore the old active tab if it is still open.
            if tabs.items.iter().any(|t| t.id == previous_active) {
                tabs.active = previous_active;
            }
        }
        let _ = relayout(app, state);
        publish(app, state);
        return Err(e);
    }

    relayout(app, state)?;
    publish(app, state);
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
    window: tauri::Window,
    url: Option<String>,
    private: Option<bool>,
) -> Result<(), String> {
    let state = state_for(&app, window.label());
    open_tab_inner(&app, &state, url, private.unwrap_or(false)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_inner(&app, id)
}

/// Closes a tab wherever it is.
///
/// Resolved from the tab rather than from a window parameter, so a shortcut and
/// a middle-click on another window's strip both land on the right list.
fn close_tab_inner(app: &AppHandle, id: u32) -> Result<(), String> {
    let Some(state) = window_of_tab(app, id) else {
        return Ok(());
    };
    // Cleared before the tab goes, so nothing downstream sees a split pointing
    // at something that no longer exists.
    {
        let mut split = state.split.lock().expect("split mutex poisoned");
        if *split == Some(id) {
            *split = None;
        }
    }

    let (label, closed_last, reopen_url) = {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");

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
            let mut closed = state.closed.lock().expect("closed mutex poisoned");
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
        // Closing the final tab closes the window, as it does everywhere else.
        // With more than one window open that now closes one window rather than
        // the browser, which is also what everywhere else does.
        if let Some(window) = app.get_window(&state.window) {
            let _ = window.close();
        }
        return Ok(());
    }

    relayout(app, &state).map_err(|e| e.to_string())?;
    save_session(app);
    publish(app, &state);
    Ok(())
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    let Some(state) = window_of_tab(&app, id) else {
        return Ok(());
    };
    let previous = {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let previous = tabs.active;
        if tabs.items.iter().any(|t| t.id == id) {
            tabs.active = id;
        }
        previous
    };

    // A tab restored from last session has no webview until now. Built before
    // the layout below, or there would be nothing to position.
    load_parked(&app, &state, id).map_err(|e| e.to_string())?;

    // The one arriving comes back first, so it is ready by the time it is shown.
    // The one leaving is only queued: see memory.rs for why it is not immediate.
    if previous != id {
        crate::memory::resume(&app, id);
        crate::memory::suspend_later(&app, previous);
    }
    // Clicking a tab is asking to see it, so the panel gets out of the way.
    dismiss_panel(&state);
    relayout(&app, &state).map_err(|e| e.to_string())?;
    publish(&app, &state);
    Ok(())
}

/// Navigates the active tab to whatever the address bar contained.
///
/// The search engine comes from settings rather than from the caller: the chrome
/// should not have to know, or be able to disagree about, which engine is current.
/// Async because it can close the panel, and closing the panel re-lays-out
/// webviews. Same rule as `set_panel`.
#[tauri::command]
pub async fn navigate(app: AppHandle, window: tauri::Window, input: String) -> Result<(), String> {
    let state = state_for(&app, window.label());
    let target = {
        let settings = app.state::<crate::settings::SettingsState>();
        let engine_id = settings.get().search_engine;
        // Through `selected` rather than `resolve`, so a user-defined engine is
        // reachable. `engine_id` alone can only name a built-in.
        let _ = engine_id;
        let engine = crate::search::selected(&settings, settings.is_dark(&app));
        crate::search::resolve_with(&input, &engine.template)
    };
    if target.is_empty() {
        return Ok(());
    }

    let url = target
        .parse()
        .map_err(|_| format!("That does not look like an address: {target}"))?;

    // After the parse, so a typo in the address bar does not close the panel on
    // its way to reporting itself.
    show_page(&app, &state)?;

    active_webview(&app, &state)?
        .navigate(url)
        .map_err(|e| e.to_string())
}

/// Closes the panel and applies the layout, for a command about to show a page.
///
/// Every caller is `async` for this reason: `relayout` moves and resizes
/// webviews, which is the thing sync commands must not do from the main thread.
fn show_page(app: &AppHandle, state: &WindowState) -> Result<(), String> {
    if dismiss_panel(state) {
        relayout(app, state).map_err(|e| e.to_string())?;
        publish(app, state);
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
pub fn print_page(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    active_webview(&app, &state)?
        .print()
        .map_err(|e| e.to_string())
}

/// Shows a tab beside the active one, or clears the split with `None`.
///
/// Async because it re-lays-out webviews and can build one: a parked tab put
/// into the split has to be loaded before it can be positioned.
///
/// The split is deliberately **not** saved to the session. It is a way of
/// looking at two things right now, not a property of the tabs, and restoring a
/// window into a split nobody asked for on launch would be a puzzle.
#[tauri::command]
pub async fn set_split(
    app: AppHandle,
    window: tauri::Window,
    id: Option<u32>,
) -> Result<(), String> {
    let state = state_for(&app, window.label());

    if let Some(id) = id {
        // Splitting a tab with itself is one pane with a gap down the side.
        let (exists, is_active) = {
            let tabs = state.tabs.lock().expect("tabs mutex poisoned");
            (tabs.items.iter().any(|t| t.id == id), tabs.active == id)
        };
        if !exists || is_active {
            return Ok(());
        }
        // Built before it is positioned. A restored tab has no webview until
        // something asks for it, and the split is asking.
        load_parked(&app, &state, id).map_err(|e| e.to_string())?;
    }

    *state.split.lock().expect("split mutex poisoned") = id;
    // Showing a page, so the panel gets out of the way, exactly as activating a
    // tab does.
    if id.is_some() {
        dismiss_panel(&state);
    }
    relayout(&app, &state).map_err(|e| e.to_string())?;
    publish(&app, &state);
    Ok(())
}

/// Opens DevTools for the active tab.
///
/// Needs the `devtools` feature on tauri, not just a debug build: `open_devtools`
/// is gated behind `any(debug_assertions, feature = "devtools")`, so without it a
/// release build has no such method and the browser ships unable to inspect
/// anything. The size that costs is measured in BUILD_NOTES rather than assumed.
///
/// Opens WebView2's own DevTools window, which is a separate top-level window
/// belonging to the runtime. Brume neither draws nor positions it.
#[tauri::command]
pub fn open_devtools(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    active_webview(&app, &state)?.open_devtools();
    Ok(())
}

/// Opens the active tab's markup in a new tab.
///
/// `view-source:` is the runtime's own, so there is nothing to render here and
/// nothing to fetch: Chromium serves the document it already has, which also
/// means the source shown is the one that was actually loaded rather than a
/// second request that could come back different.
///
/// Built here rather than typed, so it never goes through `search::resolve`.
/// `view-source:` is deliberately **not** in ALLOWED_SCHEMES: that list guards
/// what a person can put in the address bar, and adding a scheme there to serve
/// a button would widen the address bar to pay for a menu item.
#[tauri::command]
pub async fn view_source(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    let current = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        tabs.active_tab().and_then(|t| t.nav.current().cloned())
    };
    let Some(url) = current else {
        return Ok(());
    };
    // Nothing to look at: Brume's own new tab page, and anything already being
    // viewed as source. Stacking the prefix twice gives a page about a page.
    if is_new_tab(&url) || url.starts_with("view-source:") {
        return Ok(());
    }
    open_tab_inner(&app, &state, Some(format!("view-source:{url}")), false)
        .map_err(|e| e.to_string())
}

/// Toggles fullscreen.
///
/// The chrome goes with it. Tauri's fullscreen covers the whole window, and the
/// content webview is positioned relative to that window, so `relayout` puts the
/// page over the full screen once the toolbar is out of the way.
#[tauri::command]
pub async fn toggle_fullscreen(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    let now = window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(!now).map_err(|e| e.to_string())?;
    relayout(&app, &state).map_err(|e| e.to_string())?;
    Ok(())
}

/// Navigates the active tab to the homepage.
///
/// Resolved at press time rather than cached, so changing the homepage or the
/// search engine in Settings takes effect without a restart. That matters
/// because an empty homepage means "follow the search engine", so the
/// destination can change without the homepage setting itself changing.
#[tauri::command]
pub async fn go_home(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    let target = home_url(&app);
    let url = target
        .parse()
        .map_err(|_| format!("Homepage is not a valid address: {target}"))?;

    show_page(&app, &state)?;
    active_webview(&app, &state)?
        .navigate(url)
        .map_err(|e| e.to_string())
}

/// Walks the active tab's history by `delta` entries.
fn traverse(app: &AppHandle, state: &WindowState, forward: bool) -> Result<(), String> {
    // The runtime decides whether there is anywhere to go. Brume no longer
    // tracks an index it could disagree about.
    let webview = active_webview(app, state)?;
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
/// Records what the runtime says about a tab's audio, and republishes.
///
/// Called from audio.rs. Skips the publish when nothing the strip renders has
/// changed, for the same reason `update_traverse` does: these events fire freely
/// and republishing on every one of them would rebuild the tab strip constantly.
pub fn update_audio(app: &AppHandle, tab_id: u32, audible: bool, muted: bool) {
    let Some(state) = window_of_tab(app, tab_id) else {
        return;
    };
    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if tab.audible == audible && tab.muted == muted {
            return;
        }
        tab.audible = audible;
        tab.muted = muted;
    }
    publish(app, &state);
}

/// Whether the tab behind a webview label is private.
///
/// For contextmenu.rs, which has to decide what a link opened from this tab
/// inherits. Same rule `on_new_window` follows: the page asking is already in a
/// private context, so what it opens is too.
pub fn tab_is_private(app: &AppHandle, label: &str) -> bool {
    let Some(state) = window_of_tab_label(app, label) else {
        return false;
    };
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    tabs.items
        .iter()
        .find(|t| t.label == label)
        .is_some_and(|t| t.private)
}

/// The webview label for a tab id, for modules that need to reach one.
pub fn tab_label(app: &AppHandle, id: u32) -> Option<String> {
    let state = window_of_tab(app, id)?;
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    tabs.items
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.label.clone())
}

pub fn update_zoom(app: &AppHandle, tab_id: u32, zoom: f64) {
    let Some(state) = window_of_tab(app, tab_id) else {
        return;
    };
    let origin = {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if (tab.nav.zoom - zoom).abs() < 0.001 {
            return;
        }
        tab.nav.zoom = zoom;
        // Captured under the same lock, so the origin and the factor cannot be
        // taken from either side of a navigation.
        tab.nav.current().map(|u| origin_of(u)).unwrap_or_default()
    };

    // A private tab remembers nothing. Writing its zoom to settings would put a
    // site it visited on disk, which is the one thing private browsing is for.
    let private = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        tabs.items
            .iter()
            .find(|t| t.id == tab_id)
            .is_some_and(|t| t.private)
    };
    if !private && !origin.is_empty() {
        let _ = app
            .state::<crate::settings::SettingsState>()
            .set_site_zoom(&origin, zoom);
    }

    publish(app, &state);
}

/// Scheme and host, which is what a remembered zoom is keyed on.
///
/// Same shape permissions.rs uses, and for the same reason: zooming one article
/// is asking for the site to be bigger, not that one URL.
fn origin_of(url: &str) -> String {
    tauri::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| format!("{}://{}", u.scheme(), h)))
        .unwrap_or_default()
}

/// Applies a site's remembered zoom after it loads.
///
/// Spawned rather than called from the navigation handler. WebView2 warns
/// against reentering a webview from inside its own event, which is the rule
/// history.rs already follows by taking history state from `HistoryChanged`
/// instead of reading it during `publish`.
///
/// Needed on every navigation because WebView2's zoom belongs to the *webview*,
/// not the document: without this, following a link from a site zoomed to 150%
/// leaves the next site at 150% too.
fn apply_site_zoom(app: &AppHandle, tab_id: u32, url: &str) {
    let origin = origin_of(url);
    if origin.is_empty() {
        return;
    }
    let wanted = app
        .state::<crate::settings::SettingsState>()
        .site_zoom(&origin);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(label) = tab_label(&app, tab_id) else {
            return;
        };
        let Some(webview) = app.get_webview(&label) else {
            return;
        };
        // Read back rather than assumed: SetZoomFactor does not raise
        // ZoomFactorChanged, so nothing else would correct the indicator.
        if let Ok(actual) = crate::history::set_zoom(&webview, wanted) {
            update_zoom_display(&app, tab_id, actual);
        }
    });
}

/// Updates the indicator without writing the value back to settings.
///
/// Separate from `update_zoom` so applying a remembered zoom does not re-save
/// the thing it just read, which would rewrite settings.json on every
/// navigation to a site that has one.
fn update_zoom_display(app: &AppHandle, tab_id: u32, zoom: f64) {
    let Some(state) = window_of_tab(app, tab_id) else {
        return;
    };
    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if (tab.nav.zoom - zoom).abs() < 0.001 {
            return;
        }
        tab.nav.zoom = zoom;
    }
    publish(app, &state);
}

/// Sets the active tab's zoom. `1.0` is 100%.
///
/// WebView2 handles Ctrl+scroll and Ctrl+plus/minus itself, so this is not how
/// zooming normally happens. It exists so the toolbar control can reset, and so
/// the level can be driven from a test, which is the only way to prove the
/// ZoomFactorChanged watcher actually fires.
#[tauri::command]
pub fn set_zoom(app: AppHandle, window: tauri::Window, factor: f64) -> Result<(), String> {
    let state = state_for(&app, window.label());
    let Some(tab_id) = active_tab_id_in(&state) else {
        return Ok(());
    };
    let webview = active_webview(&app, &state)?;
    let actual = crate::history::set_zoom(&webview, factor)?;

    // Updated from the value read back, not left to the event. A programmatic
    // SetZoomFactor does not raise ZoomFactorChanged, so waiting for it would
    // leave the indicator showing the old level forever.
    update_zoom(&app, tab_id, actual);
    Ok(())
}

/// Puts the active tab back to 100%.
#[tauri::command]
pub fn reset_zoom(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    set_zoom(app, window, 1.0)
}

pub fn update_traverse(app: &AppHandle, tab_id: u32, can_back: bool, can_forward: bool) {
    let Some(state) = window_of_tab(app, tab_id) else {
        return;
    };
    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        let Some(tab) = tabs.tab_mut(tab_id) else {
            return;
        };
        if tab.nav.can_back == can_back && tab.nav.can_forward == can_forward {
            return; // nothing the chrome would render differently
        }
        tab.nav.can_back = can_back;
        tab.nav.can_forward = can_forward;
    }
    publish(app, &state);
}

#[tauri::command]
pub async fn go_back(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    show_page(&app, &state)?;
    traverse(&app, &state, false)
}

#[tauri::command]
pub async fn go_forward(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    show_page(&app, &state)?;
    traverse(&app, &state, true)
}

/// Reloads the active tab.
///
/// Uses the runtime's own reload rather than evaluating `location.reload()`.
/// Injected script only works on a document that hosts script in the first
/// place: a PDF in the built-in viewer, a bare image, or one of WebView2's own
/// error pages has no `location` worth calling, so the old approach failed
/// silently on exactly the pages a reload button is most wanted on.
#[tauri::command]
pub async fn reload(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    show_page(&app, &state)?;
    active_webview(&app, &state)?
        .reload()
        .map_err(|e| e.to_string())
}

/// Stops the active tab loading.
///
/// Still `window.stop()`, because neither Tauri nor wry surfaces WebView2's
/// `Stop()` - checked, there is no `Webview::stop`. Reaching it means going
/// through `with_webview` to the ICoreWebView2 directly, which is worth doing
/// alongside the other interop work rather than on its own.
#[tauri::command]
pub fn stop_loading(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let state = state_for(&app, window.label());
    active_webview(&app, &state)?
        .eval("window.stop()")
        .map_err(|e| e.to_string())?;

    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");
        if let Some(tab) = tabs.active_tab_mut() {
            tab.nav.loading = false;
        }
    }
    publish(&app, &state);
    Ok(())
}

/// Id of the active tab in one window.
fn active_tab_id_in(state: &WindowState) -> Option<u32> {
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    tabs.active_tab().map(|t| t.id)
}

/// Id of the active tab in the focused window.
///
/// For callers that act on "whatever is in front" and have no window of their
/// own: global shortcuts, and interop modules reacting to a runtime event.
pub fn active_tab_id(app: &AppHandle) -> Option<u32> {
    let state = focused_state(app)?;
    active_tab_id_in(&state)
}

/// The tab at a position in one window's strip.
fn tab_id_at_in(state: &WindowState, index: usize) -> Option<u32> {
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    tabs.items.get(index).map(|t| t.id)
}

/// Pins or unpins a tab, moving it to keep pinned tabs at the front.
///
/// Reordered rather than merely flagged, because a pinned tab in the middle of
/// the strip is a pinned tab you still have to hunt for. Pinning moves it to the
/// end of the pinned run; unpinning moves it to the front of the unpinned one,
/// so it lands where it would sit if it had never been pinned.
#[tauri::command]
pub async fn set_tab_pinned(app: AppHandle, id: u32, pinned: bool) -> Result<(), String> {
    let Some(state) = window_of_tab(&app, id) else {
        return Ok(());
    };
    {
        let mut tabs = state.tabs.lock().expect("tabs mutex poisoned");

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
    publish(&app, &state);
    Ok(())
}

/// Opens a copy of a tab beside it.
///
/// The URL only. Duplicating a tab's back history would mean copying the
/// runtime's own history, which is not something WebView2 exposes.
#[tauri::command]
pub async fn duplicate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    let Some(state) = window_of_tab(&app, id) else {
        return Ok(());
    };
    let source = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
        tabs.items
            .iter()
            .find(|t| t.id == id)
            .map(|t| (t.nav.current().cloned(), t.private))
    };

    let Some((Some(url), private)) = source else {
        return Ok(());
    };
    // Into the window the original is in, not the focused one. They are almost
    // always the same, and when they are not, beside the original is right.
    open_tab_inner(&app, &state, Some(url), private).map_err(|e| e.to_string())
}

/// Closes every tab except `id`. Pinned tabs are kept.
#[tauri::command]
pub async fn close_other_tabs(app: AppHandle, id: u32) -> Result<(), String> {
    let Some(state) = window_of_tab(&app, id) else {
        return Ok(());
    };
    let doomed: Vec<u32> = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
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
    let Some(state) = window_of_tab(&app, id) else {
        return Ok(());
    };
    let doomed: Vec<u32> = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
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
    let state = focused_state(app)?;
    tab_id_at_in(&state, index)
}

/// Id of the last tab, for Ctrl+9.
///
/// Ctrl+9 means "last tab", not "ninth tab", in every mainstream browser - it is
/// the one number that is positional rather than an index.
pub fn last_tab_id(app: &AppHandle) -> Option<u32> {
    let state = focused_state(app)?;
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");
    tabs.items.last().map(|t| t.id)
}

/// Reopens the most recently closed tab. Does nothing when none were closed.
///
/// Async for the same reason the tab commands are: it creates a webview.
///
/// Reopens into the focused window, from that window's own closed list. A tab
/// closed in one window does not come back in another: the list is per window
/// precisely so this cannot reach across.
pub async fn reopen_closed_tab(app: AppHandle) -> Result<(), String> {
    let Some(state) = focused_state(&app) else {
        return Ok(());
    };
    let url = {
        let mut closed = state.closed.lock().expect("closed mutex poisoned");
        closed.pop()
    };

    match url {
        Some(url) => open_tab_inner(&app, &state, Some(url), false).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Moves to the next or previous tab, wrapping at the ends.
///
/// Wrapping rather than stopping: Ctrl+Tab on the last tab going nowhere feels
/// broken, and every browser cycles. Wraps within the focused window rather than
/// crossing into another one, which is what every browser does too.
pub fn neighbour_tab_id(app: &AppHandle, forward: bool) -> Option<u32> {
    let state = focused_state(app)?;
    let tabs = state.tabs.lock().expect("tabs mutex poisoned");

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
pub fn browser_state(app: AppHandle, window: tauri::Window) -> BrowserState {
    let state = state_for(&app, window.label());
    current_state(&app, &state)
}

/// Expands the chrome over the whole window, or restores it.
///
/// Async for the same reason the tab commands are: it re-lays-out webviews.
#[tauri::command]
pub async fn set_panel(app: AppHandle, window: tauri::Window, open: bool) -> Result<(), String> {
    let state = state_for(&app, window.label());
    state.panel_open.store(open, Ordering::Relaxed);
    relayout(&app, &state).map_err(|e| e.to_string())?;
    publish(&app, &state);
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
pub async fn set_find_bar(app: AppHandle, window: tauri::Window, open: bool) -> Result<(), String> {
    let state = state_for(&app, window.label());
    state.find_open.store(open, Ordering::Relaxed);

    if !open {
        // Best effort: a tab with no page, or an older runtime with no find
        // support, should still be able to close the bar.
        let _ = crate::find::find_stop(app.clone());
    }

    relayout(&app, &state).map_err(|e| e.to_string())?;
    publish(&app, &state);
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

    // Every window, not just the one that asked. The bar is a preference, so a
    // second window left showing the opposite would read as a bug.
    for state in app.state::<Browser>().all() {
        relayout(&app, &state).map_err(|e| e.to_string())?;
        publish(&app, &state);
    }
    Ok(())
}

/// Says how far down an open overlay reaches, or 0 when none is open.
///
/// Used by the address bar dropdown and the permission prompt. The chrome sends
/// one number for all of them, because only the chrome knows what it drew and
/// only one of them can be the lowest.
///
/// Async because it re-lays-out webviews, same as `set_panel`.
///
/// Raising the chrome is what makes this an overlay rather than another bar. The
/// page is not moved and not hidden; see overlay.rs for why that needs a Win32
/// call at all.
#[tauri::command]
pub async fn set_chrome_overlay(
    app: AppHandle,
    window: tauri::Window,
    bottom: f64,
) -> Result<(), String> {
    let state = state_for(&app, window.label());
    // Clamped: the chrome asks for whatever it measured, and a runaway value
    // would cover the whole window with no visible way back to the page.
    let bottom = bottom.clamp(0.0, 720.0).round() as u32;
    let previous = state.overlay_bottom.swap(bottom, Ordering::Relaxed);

    if previous == bottom {
        return Ok(());
    }

    // Raise before laying out, drop after. Growing the chrome while it is still
    // underneath would paint the overlay behind the page for one frame.
    if bottom > 0 {
        crate::overlay::set_chrome_on_top(&app, &state.chrome, true);
    }
    relayout(&app, &state).map_err(|e| e.to_string())?;
    if bottom == 0 {
        crate::overlay::set_chrome_on_top(&app, &state.chrome, false);
    }
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
pub fn toggle_bookmark_active(app: AppHandle, window: tauri::Window) -> Result<bool, String> {
    let state = state_for(&app, window.label());
    let (url, title) = {
        let tabs = state.tabs.lock().expect("tabs mutex poisoned");
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
    // and would otherwise not hear that it just gained or lost a row. Every
    // window's bar, since they all show the same list.
    notify_bookmarks_everywhere(&app);
    publish(&app, &state);
    Ok(bookmarked)
}

/// Tells every window's chrome that the bookmark list moved.
///
/// The list is app-wide, so a bar in a second window is just as stale after a
/// change as the one that caused it.
pub fn notify_bookmarks_everywhere(app: &AppHandle) {
    for state in app.state::<Browser>().all() {
        let _ = app.emit_to(&state.chrome, BOOKMARKS_EVENT, ());
    }
}

/// Tells every window's chrome that the downloads list moved.
pub fn notify_downloads_everywhere(app: &AppHandle) {
    for state in app.state::<Browser>().all() {
        let _ = app.emit_to(&state.chrome, DOWNLOADS_EVENT, ());
    }
}

/// Sends an event to the chrome of whichever window holds a tab.
///
/// For the interop modules, whose handlers are attached to a content webview and
/// so know a tab rather than a window.
pub fn emit_to_tab_chrome<S: serde::Serialize + Clone>(
    app: &AppHandle,
    tab_label: &str,
    event: &str,
    payload: S,
) {
    if let Some(state) = window_of_tab_label(app, tab_label) {
        let _ = app.emit_to(&state.chrome, event, payload);
    }
}

/// Sends an event to the focused window's chrome.
///
/// For global shortcuts, which act on whatever is in front.
pub fn emit_to_focused_chrome<S: serde::Serialize + Clone>(
    app: &AppHandle,
    event: &str,
    payload: S,
) {
    if let Some(state) = focused_state(app) {
        let _ = app.emit_to(&state.chrome, event, payload);
    }
}

/// The focused window itself, for callers that have to hand one to a command.
///
/// A global shortcut has no originating webview, so this is how it says which
/// window it meant.
pub fn focused_window(app: &AppHandle) -> Option<tauri::Window> {
    let state = focused_state(app)?;
    app.get_window(&state.window)
}

/// Opens a new window, with one homepage tab.
///
/// Async for exactly the reason the tab commands are: it builds webviews, and a
/// sync command doing that from the main thread deadlocks.
#[tauri::command]
pub async fn open_window(app: AppHandle, private: Option<bool>) -> Result<(), String> {
    let label = next_window_label(&app);
    build_window(&app, &label, None, false).map_err(|e| e.to_string())?;

    // A private window is a normal window whose first tab is private. Every tab
    // opened from it inherits that, the same way `on_new_window` already does,
    // so the window stays private without a flag of its own to keep in step.
    if private.unwrap_or(false) {
        let state = state_for(&app, &label);
        let first = {
            let tabs = state.tabs.lock().expect("tabs mutex poisoned");
            tabs.items.first().map(|t| t.id)
        };
        open_tab_inner(&app, &state, None, true).map_err(|e| e.to_string())?;
        if let Some(id) = first {
            close_tab_inner(&app, id)?;
        }
    }
    Ok(())
}

/// Moves a tab into a window of its own.
///
/// The tab is rebuilt rather than reparented: WebView2 gives no way to move a
/// webview between windows, so the page is reloaded at its current URL. Scroll
/// position and anything typed into a form are lost, which is worth saying out
/// loud because it is the one thing this cannot preserve.
#[tauri::command]
pub async fn move_tab_to_new_window(app: AppHandle, id: u32) -> Result<(), String> {
    let Some(source) = window_of_tab(&app, id) else {
        return Ok(());
    };
    // Nothing to do if it is the only tab: the window it would move to is the
    // window it is already in, and closing the source would take it with it.
    let url = {
        let tabs = source.tabs.lock().expect("tabs mutex poisoned");
        if tabs.items.len() < 2 {
            return Ok(());
        }
        tabs.items
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.nav.current().cloned())
    };
    let Some(url) = url else {
        return Ok(());
    };

    let label = next_window_label(&app);
    let target = crate::settings::SessionWindow {
        tabs: vec![crate::settings::SessionTab { url, pinned: false }],
        active: 0,
    };
    build_window(&app, &label, Some(&target), false).map_err(|e| e.to_string())?;

    // Only after the new window exists. Closing first would leave the tab gone
    // with nowhere to arrive if the build failed.
    close_tab_inner(&app, id)?;
    save_session(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The traversal tests that used to live here are gone with the stack they
    // exercised. Back and forward are WebView2's now, and verifying them needs a
    // running webview rather than a unit test - done through tools/cdp.ps1.
    // What is left here is the part Brume still decides for itself.

    #[test]
    fn a_chrome_label_can_never_be_mistaken_for_a_content_webview() {
        // This is a security boundary, not a naming convention.
        // capabilities/default.json grants Brume's commands to `chrome-*`, and
        // content webviews are `tab-{id}`. If those namespaces ever overlapped,
        // the capability would match a webview rendering an arbitrary website
        // and hand it the IPC bridge for every command Brume exposes.
        for window in ["main", "win-2", "win-99"] {
            let chrome = chrome_label(window);
            assert!(
                chrome.starts_with("chrome-"),
                "{chrome} must match the capability glob"
            );
            assert!(
                !chrome.starts_with("tab-"),
                "{chrome} must not look like a content webview"
            );
        }
        // And from the other side: no tab label may match `chrome-*`.
        for id in [0u32, 1, 7, 4242] {
            let tab = format!("tab-{id}");
            assert!(
                !tab.starts_with("chrome-"),
                "{tab} must not match the capability glob"
            );
        }
    }

    #[test]
    fn window_labels_are_unique_and_never_reused() {
        // A recycled label would let a stale handler write into whichever window
        // took the name, which is the same reason tab labels are never reused.
        let browser = Browser::default();
        let first = browser.next_window.fetch_add(1, Ordering::Relaxed);
        let second = browser.next_window.fetch_add(1, Ordering::Relaxed);
        assert_ne!(first, second);
        // `main` is 1, so generated labels start past it and cannot collide.
        assert!(first >= 2, "win-{first} would collide with main");
    }

    #[test]
    fn tab_ids_are_allocated_across_windows_not_within_one() {
        // Ids have to be unique app-wide: a webview label is `tab-{id}` and
        // labels are app-wide, so per-window counters would hand two windows
        // the same `tab-1`.
        let browser = Browser::default();
        let a = browser.next_tab_id();
        let b = browser.next_tab_id();
        assert_ne!(a, b);
        assert_eq!(b, a + 1);
    }

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
            audible: false,
            muted: false,
            loaded: true,
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
