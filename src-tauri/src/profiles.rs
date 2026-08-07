//! Separate identities: bookmarks, history, cookies and settings per profile.
//!
//! # The list cannot live in a profile
//!
//! Everything else moves into a per-profile directory, but the list of profiles
//! and which one is active cannot: there would be nothing to read at startup to
//! find out where to look. `profiles.json` sits in the config root beside the
//! profile directories and is the one file that stays global.
//!
//! # The default profile keeps the paths it already has
//!
//! An existing install's `settings.json`, `bookmarks.json`, `history.jsonl` and
//! `downloads.jsonl` sit directly in the config root. They stay there. Moving
//! them into a `default/` subdirectory would be a migration, and `store.rs`
//! records at length what this project's migrations cost when one goes wrong.
//!
//! A named profile gets `profiles/<id>/` and starts empty, which is what a new
//! profile is for. So upgrading to 0.9.0 changes nothing on disk until someone
//! actually makes a second profile.
//!
//! # Switching relaunches, and that is not laziness
//!
//! A content webview's data directory is fixed when the webview is built, and
//! `Store` and `SettingsState` are resolved once at startup and handed to Tauri
//! as managed state. Switching in place would mean rebuilding every content
//! webview in every window, swapping both stores underneath whatever was reading
//! them, and leaving WebView2 holding two environments for the same process.
//!
//! Relaunching gets all of that for free, and the session is restored per window
//! anyway, so what comes back is what was there. The cost is a visible restart,
//! which is honest about what is happening rather than hiding it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// A profile, as stored and as shown.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Stable and generated, never the name. Renaming must not move a directory.
    pub id: String,
    pub name: String,
}

/// The global list. The only file that is not per profile.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Profiles {
    /// The default is implicit and always first; this holds only the extra ones.
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Id of the active profile, or empty for the default.
    #[serde(default)]
    pub active: String,
}

/// Name shown for the profile everybody starts with.
pub const DEFAULT_NAME: &str = "Default";

fn config_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn list_path(app: &AppHandle) -> PathBuf {
    config_root(app).join("profiles.json")
}

/// Reads the list, or an empty one.
///
/// An unreadable file is treated as absent rather than moved aside, unlike
/// `bookmarks.json`. There is nothing here a user typed: losing it means the
/// extra profiles stop being listed, and their directories are still on disk to
/// be recovered by hand. Preserving a `.bak` of a file Brume wrote itself would
/// be ceremony for no gain.
pub fn load(app: &AppHandle) -> Profiles {
    std::fs::read_to_string(list_path(app))
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<Profiles>(raw.strip_prefix('\u{feff}').unwrap_or(&raw)).ok()
        })
        .unwrap_or_default()
}

fn save(app: &AppHandle, list: &Profiles) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    crate::store::write_atomic(&list_path(app), &json)
}

/// Where the active profile's data lives.
///
/// The config root itself for the default profile, so an existing install is
/// untouched. See the module docs.
pub fn active_dir(app: &AppHandle) -> PathBuf {
    let list = load(app);
    dir_for(app, &list.active)
}

/// The directory for one profile id. Empty means the default.
pub fn dir_for(app: &AppHandle, id: &str) -> PathBuf {
    let root = config_root(app);
    if id.is_empty() {
        root
    } else {
        root.join("profiles").join(id)
    }
}

/// Where a profile's WebView2 data goes.
///
/// Under the profile rather than beside it, so cookies and cache belong to the
/// same directory as the bookmarks, and deleting a profile takes both.
///
/// The default profile gets an explicit subdirectory here even though its Brume
/// files stay in the root, because it never had one: WebView2's data was
/// wherever Tauri put it by default, and naming it now would move an existing
/// install's cookies. Left as `None` for the default so nothing changes.
pub fn webview_dir(app: &AppHandle, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        None
    } else {
        Some(dir_for(app, id).join("webview"))
    }
}

/// Every profile, default first, with the active one marked.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[tauri::command]
pub fn profiles(app: AppHandle) -> Vec<ProfileView> {
    let list = load(&app);
    let mut out = vec![ProfileView {
        id: String::new(),
        name: DEFAULT_NAME.to_string(),
        active: list.active.is_empty(),
    }];
    out.extend(list.profiles.iter().map(|p| ProfileView {
        id: p.id.clone(),
        name: p.name.clone(),
        active: p.id == list.active,
    }));
    out
}

/// Creates a profile and returns its id. Does not switch to it.
#[tauri::command]
pub fn create_profile(app: AppHandle, name: String) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the profile a name".into());
    }
    let mut list = load(&app);

    // Highest suffix seen plus one, never reused, for the reason tab and window
    // labels are not: a recycled id would silently adopt a deleted profile's
    // directory, and with it their bookmarks and cookies.
    let next = list
        .profiles
        .iter()
        .filter_map(|p| p.id.strip_prefix("p-"))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let id = format!("p-{next}");

    std::fs::create_dir_all(dir_for(&app, &id)).map_err(|e| e.to_string())?;
    list.profiles.push(Profile {
        id: id.clone(),
        name: name.to_string(),
    });
    save(&app, &list)?;
    Ok(id)
}

#[tauri::command]
pub fn rename_profile(app: AppHandle, id: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the profile a name".into());
    }
    if id.is_empty() {
        return Err("The default profile cannot be renamed".into());
    }
    let mut list = load(&app);
    match list.profiles.iter_mut().find(|p| p.id == id) {
        Some(p) => p.name = name.to_string(),
        None => return Err("No such profile".into()),
    }
    save(&app, &list)
}

/// Deletes a profile and everything in it.
///
/// The directory goes with the entry. A profile whose bookmarks and cookies
/// survived its deletion would be a privacy surprise, which is the opposite of
/// what a separate profile is for.
#[tauri::command]
pub fn delete_profile(app: AppHandle, id: String) -> Result<(), String> {
    if id.is_empty() {
        return Err("The default profile cannot be deleted".into());
    }
    let list = load(&app);
    if list.active == id {
        return Err("That is the profile you are using".into());
    }

    let dir = dir_for(&app, &id);
    // Guarded rather than trusted. `id` arrives over IPC, and a path that
    // escaped the profiles directory would delete something else entirely.
    let root = config_root(&app).join("profiles");
    if !dir.starts_with(&root) || dir == root {
        return Err("Refusing to delete that path".into());
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }

    let mut list = list;
    list.profiles.retain(|p| p.id != id);
    save(&app, &list)
}

/// Switches profile and relaunches. See the module docs for why it relaunches.
#[tauri::command]
pub fn switch_profile(app: AppHandle, id: String) -> Result<(), String> {
    let mut list = load(&app);
    if !id.is_empty() && !list.profiles.iter().any(|p| p.id == id) {
        return Err("No such profile".into());
    }
    if list.active == id {
        return Ok(());
    }
    list.active = id;
    save(&app, &list)?;

    // Written before the relaunch, so the next process reads the new active
    // profile from disk. Nothing is passed on the command line: an argument
    // would be a second source of truth for the same answer.
    app.restart();
}

/// Whether a path is inside the active profile. Used by the store's tests.
#[allow(dead_code)]
pub fn is_within(dir: &Path, root: &Path) -> bool {
    dir.starts_with(root)
}
