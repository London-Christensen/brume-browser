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

// Both commands below are `async`, and neither is optional about it.
//
// Tauri runs a *synchronous* command on the main thread, which is also the
// thread the event loop pumps. A sync command that blocks therefore blocks the
// whole UI. `async` moves it onto the async runtime instead, leaving the main
// thread free to keep drawing.
//
// This is the same trap the browser documents for `add_child`, arrived at from
// the other direction: there the symptom is a deadlock, here it is a frozen
// window. The rule is one rule - do not block the main thread - and the shell
// had simply never had it applied.

/// Opens the native folder picker.
///
/// `blocking_pick_folder` is explicitly documented as unsafe to call on the main
/// thread ("should *NOT* be used when running on the main thread"), because it
/// waits on a dialog whose messages the event loop it is blocking would have to
/// pump. The plugin's own example puts it in an `async fn` for this reason.
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle) -> Option<String> {
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
///
/// Async because `status()` waits for NSIS to finish, which takes seconds. Run
/// on the main thread that stalled the event loop for the whole install, so the
/// "Installing" screen froze mid-animation and Windows greyed the window out as
/// not responding - during the one stretch where the UI most needs to look alive.
#[tauri::command]
async fn install(dir: String, auto_update: bool) -> Result<(), String> {
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

/// Flags that mean "do not show a UI".
///
/// `/UPDATE` is what the updater sends. `/S` and `/P` are NSIS's own silent and
/// passive switches, and someone passing either to this exe is asking for the
/// plain installer too. Any of them means the shell should get out of the way.
const NO_UI_FLAGS: &[&str] = &["/UPDATE", "/S", "/P"];

/// This exe's command line with the program name removed.
///
/// Read raw rather than rebuilt from `args_os()`. The arguments have to reach
/// NSIS exactly as they arrived: `/D` in particular consumes the rest of the
/// line verbatim, and re-quoting a split argv changes what NSIS parses.
fn raw_argument_tail() -> String {
    use windows_sys::Win32::System::Environment::GetCommandLineW;

    let full = unsafe {
        let ptr = GetCommandLineW();
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    };

    strip_program_name(&full).to_string()
}

/// Drops argv[0] from a raw Windows command line.
///
/// Split out from the `GetCommandLineW` call so the quoting rules can be tested
/// without a running process. argv[0] is quoted when the path contains spaces
/// and bare otherwise, and `%LOCALAPPDATA%` sits under `C:\Users\<name>`, which
/// is a space away from being the interesting case for most people.
fn strip_program_name(command_line: &str) -> &str {
    let rest = if let Some(stripped) = command_line.strip_prefix('"') {
        stripped.split_once('"').map(|(_, rest)| rest).unwrap_or("")
    } else {
        command_line
            .find(char::is_whitespace)
            .map_or("", |i| &command_line[i..])
    };
    rest.trim_start()
}

/// Does this command line ask for the installer without a UI?
fn wants_no_ui(argument_tail: &str) -> bool {
    argument_tail
        .split_whitespace()
        .any(|arg| NO_UI_FLAGS.iter().any(|f| arg.eq_ignore_ascii_case(f)))
}

/// Runs the embedded NSIS installer with this process's own arguments, and
/// returns its exit code.
///
/// This is the path an auto-update takes. Tauri's updater downloads whatever
/// `latest.json` points at and runs it with `/P /R /UPDATE /ARGS ...`, so when
/// the manifest points here, the shell has to behave like the installer it
/// wraps rather than opening a window nobody asked for.
///
/// It costs nothing to carry: the NSIS installer is already inside this binary,
/// which is why publishing it separately was redundant.
fn forward_to_payload(args: &str) -> i32 {
    let installer = match stage_payload() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("[setup] could not unpack the installer: {e}");
            return 1;
        }
    };

    let mut cmd = Command::new(&installer);
    cmd.creation_flags(CREATE_NO_WINDOW);
    if !args.is_empty() {
        // raw_arg, for the same reason the line was read raw in the first place.
        cmd.raw_arg(args);
    }

    match cmd.status() {
        Ok(status) => status.code().unwrap_or(0),
        Err(e) => {
            eprintln!("[setup] could not start the installer: {e}");
            1
        }
    }
}

fn main() {
    // Update mode, before anything else.
    //
    // Checked ahead of the WebView2 probe and ahead of Tauri, because an update
    // must not depend on either. The machine already has Brume installed, so
    // WebView2 is present, and drawing UI would be wrong even if it were not.
    let tail = raw_argument_tail();
    if wants_no_ui(&tail) {
        std::process::exit(forward_to_payload(&tail));
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_program_name_is_dropped_with_its_quotes() {
        // The real case: an install path under C:\Users\<name>\AppData is
        // quoted by the caller whenever the account name contains a space.
        let line = r#""C:\Users\A B\Brume-Setup.exe" /P /R /UPDATE"#;
        assert_eq!(strip_program_name(line), "/P /R /UPDATE");
    }

    #[test]
    fn an_unquoted_program_name_is_dropped() {
        assert_eq!(
            strip_program_name(r"C:\tmp\Brume-Setup.exe /S"),
            "/S"
        );
    }

    #[test]
    fn a_bare_program_name_leaves_nothing() {
        assert_eq!(strip_program_name(r"C:\tmp\Brume-Setup.exe"), "");
        assert_eq!(strip_program_name(r#""C:\tmp\Brume-Setup.exe""#), "");
    }

    #[test]
    fn the_updater_command_line_is_recognised() {
        // Exactly what tauri-plugin-updater builds for a passive NSIS install:
        // install_mode.nsis_args() ++ /UPDATE ++ /ARGS ++ the app's own args.
        let tail = strip_program_name(r#""C:\x\Brume-Setup.exe" /P /R /UPDATE /ARGS"#);
        assert_eq!(tail, "/P /R /UPDATE /ARGS");
        assert!(wants_no_ui(tail), "an update must never draw the shell UI");
    }

    #[test]
    fn a_plain_double_click_still_gets_the_ui() {
        let tail = strip_program_name(r#""C:\Downloads\Brume-Setup.exe""#);
        assert!(tail.is_empty());
        assert!(!wants_no_ui(tail), "a human double-click must show the UI");
    }

    #[test]
    fn flags_are_matched_whole_and_case_insensitively() {
        assert!(wants_no_ui("/update"));
        assert!(wants_no_ui("/p"));
        // Not a substring match: /UPDATER and /SILENTLY are not our flags, and
        // /D swallows the rest of the line so a path could contain anything.
        assert!(!wants_no_ui("/UPDATER"));
        assert!(!wants_no_ui(r"/D=C:\Users\ps\Brume"));
        assert!(!wants_no_ui(r"/D=C:\update\Brume"));
    }
}
