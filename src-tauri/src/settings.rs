//! Persistent application settings.
//!
//! Stored as JSON next to the rest of Brume's data, in the OS app-data
//! directory. Deliberately a plain file rather than a database: the settings
//! are a handful of scalars, and a file can be inspected and repaired with a
//! text editor when something goes wrong.
//!
//! The one subtlety here is where `auto_update` initially comes from. The
//! installer asks the question before the app has ever run, and writes the
//! answer to the registry. The app reads it exactly once - on the first launch,
//! when no settings file exists yet - and from then on this file is
//! authoritative. That ordering is what stops a later reinstall from silently
//! overriding a preference the user has since changed in Settings.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

/// Where the NSIS installer records the auto-update choice.
///
/// This string is coupled to `bundle.publisher` and `productName` in
/// tauri.conf.json, because NSIS derives the key from them as
/// `Software\<manufacturer>\<product>`. Change either and this must change too -
/// which is exactly why `publisher` is pinned explicitly rather than left to be
/// inferred from the bundle identifier. See docs/INSTALLER.md.
const INSTALLER_REGISTRY_KEY: &str = r"Software\London Christensen\Brume";

#[derive(Serialize, Deserialize, Clone, Debug)]
// `default` on the container means a settings file written by an older build,
// missing fields a newer build expects, still loads instead of being discarded.
#[serde(default)]
pub struct Settings {
    /// Check for a new version on launch. Never affects the manual check, which
    /// stays available regardless.
    pub auto_update: bool,

    /// Id of the active search engine, from `search::ENGINES`.
    ///
    /// Stored as the id rather than the URL template so that fixing a template
    /// in a later version reaches everyone, instead of leaving users pinned to
    /// whatever string was copied into their settings file the day they chose it.
    pub search_engine: String,

    /// "system", "light" or "dark".
    ///
    /// Dark is the default rather than system: Brume's chrome is dark-first and
    /// the brand is built around Ink, so following a light OS by default would
    /// show most users something the design was not drawn for.
    pub theme: String,

    /// Where new tabs open. Empty means "follow the search engine".
    ///
    /// Empty is the default rather than a baked-in URL, so that choosing Mojeek
    /// moves the new-tab page too. Someone who sets an explicit homepage has
    /// said what they want and it is left alone.
    pub homepage: String,

    /// Whether the bookmarks bar sits under the toolbar.
    ///
    /// Off by default. On first run there is nothing to put in it, and an empty
    /// strip costs the page 32px to say so. Ctrl+Shift+B and the Settings switch
    /// both turn it on, and it is remembered here rather than per session
    /// because a bar you asked for should still be there next launch.
    ///
    /// Its own key rather than a field on some wider "chrome" object, for the
    /// reason `session_tabs` records: serde rejects a field of the wrong type
    /// outright, and an unparseable settings file is moved aside wholesale, so
    /// changing the shape of an existing key costs the user every other setting.
    /// Adding one is free, and an older build ignores it.
    pub show_bookmarks_bar: bool,

    /// Tabs open when Brume last closed, restored on the next launch.
    ///
    /// Empty means "open the homepage", which is both the first-run case and
    /// what happens after every tab was closed by hand.
    ///
    /// Named `session_tabs` rather than reusing the older `session` key, which
    /// held bare URL strings. serde rejects a field of the wrong type outright,
    /// and because a settings file that will not parse is moved aside wholesale,
    /// reusing the name would have cost the user every other setting too. An
    /// unknown key is ignored, so old files simply start with no session.
    pub session_tabs: Vec<SessionTab>,

    /// Index into `session_tabs` of the tab that was in front.
    pub session_active: usize,

    /// Every window that was open, from 0.7.0 onwards.
    ///
    /// A third session key rather than a change to the two above, for the
    /// reason they already document: serde rejects a field of the wrong type
    /// outright and an unparseable settings file is moved aside wholesale, so
    /// turning `session_tabs` into a list of lists would have cost every user
    /// upgrading from 0.6.0 their entire settings file.
    ///
    /// The old keys are still read when this is empty, which is exactly the
    /// upgrade case: a 0.6.0 session becomes one window. They are no longer
    /// written, so a downgrade loses the session but nothing else.
    #[serde(default)]
    pub session_windows: Vec<SessionWindow>,

    /// Last window geometry, or `None` until the first window closes.
    ///
    /// Kept here rather than in its own file so it inherits this module's
    /// BOM-stripping and corrupt-file handling. It is not a preference, and the
    /// Settings panel deliberately does not show it.
    pub window: Option<WindowGeometry>,
}

/// One restored tab.
///
/// URL and pinned state only. A tab's own back history is WebView2's and dies
/// with the webview, so there is nothing more to keep.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionTab {
    pub url: String,
    #[serde(default)]
    pub pinned: bool,
}

/// One window's worth of session.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SessionWindow {
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
    /// Index into `tabs` of the one that was in front.
    #[serde(default)]
    pub active: usize,
}

/// Where the window was, in logical pixels.
///
/// Logical rather than physical so the window comes back the same apparent size
/// when it is restored on a display with a different scale factor - storing
/// physical pixels means a window saved at 150% reopens two thirds the size at
/// 100%.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Restored as maximised, with the values above kept as the size to return
    /// to when it is un-maximised.
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            search_engine: crate::search::DEFAULT_ENGINE_ID.to_string(),
            theme: "dark".to_string(),
            homepage: String::new(),
            show_bookmarks_bar: false,
            session_tabs: Vec::new(),
            session_active: 0,
            session_windows: Vec::new(),
            window: None,
        }
    }
}

/// Reads the preference the installer recorded.
///
/// Returns `None` when there is no installed-by-installer Brume to read from -
/// most commonly a `tauri dev` run, where falling back to the default is right.
fn installer_preference() -> Option<bool> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(INSTALLER_REGISTRY_KEY)
        .ok()
        .and_then(|key| key.get_value::<u32, _>("AutoUpdate").ok())
        .map(|value| value != 0)
}

pub struct SettingsState {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsState {
    /// Loads settings from disk, creating them on first run.
    ///
    /// Never fails to produce a usable state: settings are not important enough
    /// to refuse to start over. It does, however, refuse to *destroy* anything -
    /// see the corrupt-file branch.
    pub fn load(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let path = dir.join("settings.json");

        let raw = fs::read_to_string(&path).ok();

        let parsed = raw.as_deref().and_then(|text| {
            // Strip a UTF-8 byte-order mark before parsing.
            //
            // serde_json rejects a leading BOM, and on Windows a BOM is easy to
            // acquire by accident: Notepad adds one, and PowerShell's
            // `Set-Content -Encoding utf8` adds one on 5.1. Without this, a
            // hand-edited settings file parses as corrupt and every setting
            // silently reverts - which is exactly how this was found.
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            serde_json::from_str::<Settings>(text).ok()
        });

        if let Some(settings) = parsed {
            return Self {
                path,
                current: Mutex::new(settings),
            };
        }

        // A file that exists but will not parse is kept, not overwritten.
        //
        // This file is meant to be hand-editable, so an unparseable one is
        // usually a typo - a trailing comma, a smart quote pasted from a
        // document. Silently replacing it means the user loses every setting and
        // has nothing left to look at to work out why.
        if raw.is_some() {
            let backup = path.with_extension("json.bak");
            match fs::rename(&path, &backup) {
                Ok(()) => eprintln!(
                    "[settings] {} could not be parsed; the original was kept at {}",
                    path.display(),
                    backup.display()
                ),
                Err(e) => eprintln!(
                    "[settings] could not parse or preserve {}: {e}",
                    path.display()
                ),
            }
        }

        // First run, or recovery. Inherit the installer's answer if there is one.
        let seeded = Settings {
            auto_update: installer_preference().unwrap_or(true),
            ..Settings::default()
        };
        let state = Self {
            path,
            current: Mutex::new(seeded.clone()),
        };
        // Best effort: if this fails the app still works, it just re-seeds next
        // launch.
        let _ = state.persist(&seeded);
        state
    }

    /// Writes settings out, atomically.
    ///
    /// Shares `store::write_atomic` rather than calling `fs::write` directly.
    /// Serialising straight over the destination leaves a truncated file if the
    /// process dies mid-write, and a truncated settings.json is every setting
    /// gone at once. store.rs already reasoned this through for bookmarks; the
    /// argument is identical here and the two had simply drifted apart.
    ///
    /// It matters more since the window geometry started being saved on close,
    /// because that write happens exactly when the process is about to exit.
    fn persist(&self, settings: &Settings) -> Result<(), String> {
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        crate::store::write_atomic(&self.path, &json)
    }

    pub fn get(&self) -> Settings {
        self.current
            .lock()
            .expect("settings mutex poisoned")
            .clone()
    }

    /// Applies a change and writes it out.
    ///
    /// One helper rather than a setter per field: every setter needs the same
    /// lock-mutate-clone-persist sequence, and repeating it is how one of them
    /// eventually forgets to persist.
    fn update<F: FnOnce(&mut Settings)>(&self, change: F) -> Result<(), String> {
        let updated = {
            let mut guard = self.current.lock().expect("settings mutex poisoned");
            change(&mut guard);
            guard.clone()
        };
        self.persist(&updated)
    }

    pub fn set_auto_update(&self, enabled: bool) -> Result<(), String> {
        self.update(|s| s.auto_update = enabled)
    }

    pub fn set_search_engine(&self, id: &str) -> Result<(), String> {
        // Resolved through engine_by_id so an unknown id cannot be stored;
        // it falls back to the default rather than leaving search broken.
        let resolved = crate::search::engine_by_id(id).id.to_string();
        self.update(|s| s.search_engine = resolved)
    }

    pub fn set_homepage(&self, url: &str) -> Result<(), String> {
        let trimmed = url.trim().to_string();
        self.update(|s| s.homepage = trimmed)
    }

    pub fn set_theme(&self, theme: &str) -> Result<(), String> {
        // Anything unrecognised becomes dark rather than being stored, so a
        // hand-edited settings file cannot leave the UI in an undefined state.
        let normalised = match theme {
            "light" | "system" => theme,
            _ => "dark",
        }
        .to_string();
        self.update(|s| s.theme = normalised)
    }

    /// Whether the bookmarks bar is showing.
    ///
    /// A field read rather than `get().show_bookmarks_bar`, because `get` clones
    /// the whole struct - session tab list included - and this is on the layout
    /// path, read every time a content webview is positioned.
    pub fn show_bookmarks_bar(&self) -> bool {
        self.current
            .lock()
            .expect("settings mutex poisoned")
            .show_bookmarks_bar
    }

    pub fn set_show_bookmarks_bar(&self, show: bool) -> Result<(), String> {
        self.update(|s| s.show_bookmarks_bar = show)
    }

    pub fn set_window(&self, geometry: WindowGeometry) -> Result<(), String> {
        self.update(|s| s.window = Some(geometry))
    }

    /// Records every open window and which tab was in front in each.
    pub fn set_session(&self, windows: Vec<SessionWindow>) -> Result<(), String> {
        self.update(|s| {
            s.session_windows = windows;
            // The 0.6.0 keys are left exactly as they were rather than cleared.
            // A downgrade then still restores the session it last understood,
            // instead of opening to nothing.
        })
    }

    /// The session to restore, one entry per window.
    ///
    /// Falls back to the single-window keys when the new one is empty, which is
    /// every install upgrading from 0.6.0 or earlier: their one session becomes
    /// one window.
    pub fn session(&self) -> Vec<SessionWindow> {
        let current = self.get();
        if !current.session_windows.is_empty() {
            return current.session_windows;
        }
        if current.session_tabs.is_empty() {
            return Vec::new();
        }
        vec![SessionWindow {
            tabs: current.session_tabs,
            active: current.session_active,
        }]
    }

    pub fn window(&self) -> Option<WindowGeometry> {
        self.get().window
    }

    /// Whether the UI is currently dark.
    ///
    /// "system" is resolved by asking the window what the OS gave it, rather
    /// than guessing. If there is no window yet, dark is the safer default -
    /// Brume's chrome is dark-first, so that is the palette everything else is
    /// drawn to match.
    pub fn is_dark(&self, app: &AppHandle) -> bool {
        match self.get().theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => app
                .get_window(crate::browser::WINDOW_LABEL)
                .and_then(|w| w.theme().ok())
                .map(|t| t == tauri::Theme::Dark)
                .unwrap_or(true),
        }
    }

    /// The URL a new tab should open, or empty for Brume's own new tab page.
    ///
    /// Empty used to mean "the active engine's landing page", so that choosing
    /// Mojeek moved the new-tab destination too. That was a nice property and
    /// the wrong default: it meant a request to a search engine every time a tab
    /// was opened, before anything had been typed, on a browser that leads with
    /// privacy. browser.rs turns empty into a local page instead.
    ///
    /// Anyone who preferred the old behaviour sets the engine's URL here
    /// explicitly, which is one field and does exactly what it says.
    pub fn resolved_homepage(&self, _app: &AppHandle) -> String {
        self.get().homepage
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, SettingsState>) -> Settings {
    state.get()
}

#[tauri::command]
pub fn set_auto_update(state: State<'_, SettingsState>, enabled: bool) -> Result<(), String> {
    state.set_auto_update(enabled)
}

#[tauri::command]
pub fn set_search_engine(state: State<'_, SettingsState>, id: String) -> Result<(), String> {
    state.set_search_engine(&id)
}

#[tauri::command]
pub fn set_homepage(state: State<'_, SettingsState>, url: String) -> Result<(), String> {
    state.set_homepage(&url)
}

/// Stores the theme and applies it to the window frame.
///
/// The chrome restyles itself in CSS, but the title bar is drawn by Windows and
/// only changes if the window is told - otherwise a light UI keeps a dark title
/// bar bolted to the top of it.
#[tauri::command]
pub fn set_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    app.state::<SettingsState>().set_theme(&theme)?;

    let resolved = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        // None hands the decision back to the OS.
        _ => None,
    };

    if let Some(window) = app.get_window(crate::browser::WINDOW_LABEL) {
        let _ = window.set_theme(resolved);
    }
    Ok(())
}

/// The active search engine's own landing page, in the current theme.
///
/// Offered as a one-click fill for the Homepage field rather than as a default.
/// It used to be what an empty homepage meant, and that was the wrong default -
/// every new tab became a request to a search engine. Keeping it reachable is
/// the difference between changing a default and removing a capability.
#[tauri::command]
pub fn engine_homepage(app: tauri::AppHandle) -> String {
    let state = app.state::<SettingsState>();
    let dark = state.is_dark(&app);
    crate::search::engine_by_id(&state.get().search_engine)
        .home_for(dark)
        .to_string()
}

/// The running version, for the updates section of Settings.
#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}
