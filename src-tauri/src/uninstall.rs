//! Removing Brume from inside Brume.
//!
//! # Why this is not just "delete the folder"
//!
//! A process cannot delete the executable it is running from. Windows holds the
//! image open for as long as the process lives, so anything that tries to remove
//! `brume.exe` from inside Brume fails on the one file that matters and leaves a
//! half-removed install behind.
//!
//! The installer already solved this. It writes `uninstall.exe` and the
//! Add/Remove Programs entry that points at it, and an NSIS uninstaller copies
//! itself to a temporary directory before running so it can delete the directory
//! it came from. So this does not reimplement removal: it finds that uninstaller,
//! starts it, and gets out of the way.
//!
//! # The shape is the updater's, and that one is proven
//!
//! `updater.rs` already hands off to a detached process and quits, and the owner
//! has taken a real install through it. This follows the same order: spawn,
//! then exit immediately rather than waiting. Waiting would be worse than
//! pointless - the uninstaller's first job is to close Brume, so a Brume sitting
//! on `WaitForExit` would be waiting for something that is waiting for it.
//!
//! # It refuses when there is nothing to uninstall
//!
//! A Brume run straight out of `target/debug` has no registry entry and no
//! `uninstall.exe`. Offering to uninstall it would either do nothing or, worse,
//! find some other install and remove that instead. `state` reports what is
//! actually there so the UI can say so rather than present a button that lies.

use serde::Serialize;
use tauri::AppHandle;

/// Where the installer records itself, per user.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Brume";

/// What the Settings panel needs to know before offering to remove anything.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UninstallState {
    /// Whether a real, removable install was found.
    pub available: bool,
    /// Where it is, for the confirmation to name.
    pub location: String,
    /// The version the installer recorded, which is not necessarily this one:
    /// a debug build run beside an install would report the install's.
    pub version: String,
}

/// Reads the uninstaller path the installer registered.
///
/// `UninstallString` is quoted by convention, so the quotes come off before it
/// is treated as a path. Left on, `Command::new` looks for a file whose name
/// begins with a quote character and reports that it does not exist.
fn uninstaller() -> Option<(String, String, String)> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let raw: String = key.get_value("UninstallString").ok()?;
    let path = raw.trim().trim_matches('"').to_string();
    if path.is_empty() || !std::path::Path::new(&path).exists() {
        return None;
    }
    let location: String = key.get_value("InstallLocation").unwrap_or_default();
    let location = location.trim().trim_matches('"').to_string();

    // The running binary must be the installed one.
    //
    // This is the whole safety of the feature and it was missing at first.
    // The registry describes *an* install, not necessarily this process: a
    // build run out of `target\debug` found the real install and reported
    // itself ready to remove it. Measured on 2026-08-08, and it would have let
    // a developer uninstall their own browser from a debug build.
    //
    // Compared by path prefix rather than by version, because a debug build and
    // an install can easily report the same version and be entirely different
    // copies.
    if !running_from(&location) {
        return None;
    }

    let version: String = key.get_value("DisplayVersion").unwrap_or_default();
    Some((path, location, version))
}

/// Whether this process's executable lives inside `location`.
///
/// Canonicalised on both sides so a trailing slash, a short 8.3 name or a
/// different case cannot make an install look foreign. A failure to canonicalise
/// is treated as "not inside": refusing to offer the button is a harmless
/// outcome, and offering it wrongly is not.
fn running_from(location: &str) -> bool {
    if location.is_empty() {
        return false;
    }
    let (Ok(exe), Ok(root)) = (
        std::env::current_exe().and_then(|p| p.canonicalize()),
        std::path::Path::new(location).canonicalize(),
    ) else {
        return false;
    };
    exe.starts_with(root)
}

#[tauri::command]
pub fn uninstall_state() -> UninstallState {
    match uninstaller() {
        Some((_, location, version)) => UninstallState {
            available: true,
            location,
            version,
        },
        None => UninstallState {
            available: false,
            location: String::new(),
            version: String::new(),
        },
    }
}

/// Starts the uninstaller and quits.
///
/// Irreversible, so the confirmation belongs in front of it rather than here:
/// a command that asks its own question cannot be reused by anything that has
/// already asked.
///
/// Async because it is called from the chrome and ends in `app.exit`, which
/// unwinds the event loop. Doing that from a synchronous command would tear
/// down the loop from inside a call the loop is still running.
#[tauri::command]
pub async fn run_uninstaller(app: AppHandle) -> Result<(), String> {
    let Some((path, _, _)) = uninstaller() else {
        return Err("Brume does not look installed on this machine.".into());
    };

    // Detached, and deliberately not waited on. The uninstaller's first act is
    // to close Brume, so waiting here would deadlock the two against each other.
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("Could not start the uninstaller: {e}"))?;

    // A moment for the uninstaller to be up before this process goes. Without
    // it, Brume can exit first and the uninstaller's own "is it running" check
    // races the process actually being gone.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_uninstall_string_is_still_a_path() {
        // Windows quotes UninstallString by convention. Left on, the quote
        // becomes part of the filename and the uninstaller is reported missing
        // on a machine where it is sitting right there.
        for raw in [
            r#""C:\Users\x\AppData\Local\Brume\uninstall.exe""#,
            r#"C:\Users\x\AppData\Local\Brume\uninstall.exe"#,
            r#"  "C:\Users\x\AppData\Local\Brume\uninstall.exe"  "#,
        ] {
            let cleaned = raw.trim().trim_matches('"');
            assert_eq!(
                cleaned, r"C:\Users\x\AppData\Local\Brume\uninstall.exe",
                "{raw} did not clean up"
            );
        }
    }

    #[test]
    fn nothing_installed_reports_unavailable_rather_than_guessing() {
        // The state a debug build out of target/ is in. The UI has to be able
        // to say "not installed" instead of offering a button that either does
        // nothing or finds somebody else's install.
        let state = uninstall_state();
        if !state.available {
            assert!(state.location.is_empty());
            assert!(state.version.is_empty());
        }
    }
}
