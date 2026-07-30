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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            search_engine: crate::search::DEFAULT_ENGINE_ID.to_string(),
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
                Err(e) => eprintln!("[settings] could not parse or preserve {}: {e}", path.display()),
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

    fn persist(&self, settings: &Settings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).map_err(|e| e.to_string())
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings mutex poisoned").clone()
    }

    pub fn set_auto_update(&self, enabled: bool) -> Result<(), String> {
        let updated = {
            let mut guard = self.current.lock().expect("settings mutex poisoned");
            guard.auto_update = enabled;
            guard.clone()
        };
        self.persist(&updated)
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
