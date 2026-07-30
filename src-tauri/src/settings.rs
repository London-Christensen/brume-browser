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
}

impl Default for Settings {
    fn default() -> Self {
        Self { auto_update: true }
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
    /// Never fails: a missing, unreadable or corrupt file falls back to
    /// defaults. Settings are not important enough to refuse to start over.
    pub fn load(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let path = dir.join("settings.json");

        let existing = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok());

        let settings = match existing {
            Some(loaded) => loaded,
            None => {
                // First run. Inherit the installer's answer if there is one.
                let seeded = Settings {
                    auto_update: installer_preference().unwrap_or(true),
                    ..Settings::default()
                };
                let state = Self {
                    path: path.clone(),
                    current: Mutex::new(seeded.clone()),
                };
                // Best effort: if this fails the app still works, it just
                // re-seeds from the registry next launch.
                let _ = state.persist(&seeded);
                return state;
            }
        };

        Self {
            path,
            current: Mutex::new(settings),
        }
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
