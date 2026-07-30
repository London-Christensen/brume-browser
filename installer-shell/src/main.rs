// Brume Setup - the installer shell.
//
// This is a thin, good-looking face over the NSIS installer rather than a
// replacement for it. The custom UI collects two decisions from a human, then
// runs the real installer silently underneath.
//
// The split matters for updates. Tauri's updater applies an update by running
// the NSIS installer with /P /UPDATE and no UI at all, so this shell is only
// ever seen on a first install. Replacing NSIS outright would have meant
// reimplementing the uninstaller, the Add/Remove Programs registration and the
// update-apply path; wrapping it costs none of that.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The NSIS installer, baked in at compile time by build.rs so that the shipped
/// Brume-Setup.exe is a single self-contained file.
const PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.exe"));

/// Keeps a console window from flashing up behind spawned helper processes.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Microsoft's registration GUID for the Evergreen WebView2 runtime.
const WEBVIEW2_CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Is the WebView2 runtime present?
///
/// The runtime registers under one of three locations depending on whether it
/// was installed per-machine or per-user, so all three are checked. A key that
/// exists with an empty or zeroed `pv` value means a broken or partially removed
/// install and is treated as absent.
fn webview2_available() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let candidates = [
        (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
        ),
        (
            HKEY_CURRENT_USER,
            format!(r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
        ),
    ];

    candidates.iter().any(|(root, path)| {
        RegKey::predef(*root)
            .open_subkey(path)
            .ok()
            .and_then(|key| key.get_value::<String, _>("pv").ok())
            .is_some_and(|version| !version.is_empty() && version != "0.0.0.0")
    })
}

/// Writes the embedded NSIS installer out to a temporary file so it can be run.
fn stage_payload() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("brume-setup");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("brume-installer.exe");
    std::fs::write(&path, PAYLOAD)?;
    Ok(path)
}

#[tauri::command]
fn default_install_dir() -> String {
    // Matches the default the NSIS installer would pick for a currentUser
    // install, so the field is pre-filled with what actually happens if the
    // user changes nothing.
    std::env::var("LOCALAPPDATA")
        .map(|base| format!(r"{base}\Brume"))
        .unwrap_or_else(|_| r"C:\Brume".to_string())
}

#[tauri::command]
fn pick_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    // The native folder picker is the one piece of UI here that is not ours.
    // Reimplementing IFileDialog to theme it is not worth the surface area.
    app.dialog()
        .file()
        .set_title("Choose where to install Brume")
        .blocking_pick_folder()
        .and_then(|picked| picked.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

/// Runs the real installer.
#[tauri::command]
fn install(dir: String, auto_update: bool) -> Result<(), String> {
    let dir = dir.trim().trim_end_matches('\\').to_string();
    if dir.is_empty() {
        return Err("Choose a folder to install into.".into());
    }

    let installer =
        stage_payload().map_err(|e| format!("Could not unpack the installer: {e}"))?;

    let mut cmd = Command::new(&installer);
    cmd.creation_flags(CREATE_NO_WINDOW);

    // raw_arg throughout, because NSIS parses this command line itself and
    // Rust's usual quoting corrupts it.
    //
    // /D has three rules that are easy to trip over: it must come last, it must
    // NOT be quoted, and it consumes the remainder of the line verbatim. That
    // last rule is also what lets an unquoted path containing spaces work.
    cmd.raw_arg("/S");
    if !auto_update {
        cmd.raw_arg("/NOAUTOUPDATE");
    }
    cmd.raw_arg(format!("/D={dir}"));

    let status = cmd
        .status()
        .map_err(|e| format!("Could not start the installer: {e}"))?;

    if !status.success() {
        return Err(format!(
            "The installer exited with code {}.",
            status.code().unwrap_or(-1)
        ));
    }

    // Trust but verify. A zero exit code from a silent NSIS run is not proof
    // that the payload actually landed - a bad /D path fails quietly.
    if !Path::new(&dir).join("brume.exe").exists() {
        return Err(format!(
            "The installer reported success but Brume is not in {dir}."
        ));
    }

    Ok(())
}

#[tauri::command]
fn launch_brume(dir: String) -> Result<(), String> {
    let exe = PathBuf::from(dir.trim_end_matches('\\')).join("brume.exe");
    Command::new(&exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start Brume: {e}"))
}

fn main() {
    // This has to happen before Tauri starts.
    //
    // The shell draws its UI in WebView2, so if WebView2 is missing it cannot
    // render so much as an error message - tauri::Builder::run would simply
    // fail. Hand off to the NSIS installer instead, which ships a bootstrapper
    // that installs the runtime. The user gets the plain installer rather than
    // the styled one, which is a fair trade for it working at all.
    if !webview2_available() {
        if let Ok(installer) = stage_payload() {
            let _ = Command::new(installer).spawn();
        }
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            default_install_dir,
            pick_folder,
            install,
            launch_brume
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Brume installer");
}
