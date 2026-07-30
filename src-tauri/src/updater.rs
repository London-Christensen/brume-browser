//! Update checking and installation.
//!
//! Two entry points, and the difference between them is the whole design:
//!
//!   * The **launch check** runs only when `auto_update` is on, and is silent
//!     about everything except finding an update. No network, no release
//!     published yet, GitHub having a bad morning - none of that is the user's
//!     problem, and a browser that opens with an error box because it could not
//!     reach a server is worse than one that says nothing.
//!
//!   * The **manual check** reports every outcome, including "you are already
//!     up to date" and any failure. Someone who just pressed a button deserves
//!     an answer.
//!
//! Updates are never installed silently. The user is always asked first.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

/// Release notes can be arbitrarily long, and a native message box has no
/// scrollbar worth the name. Long notes are trimmed with a pointer to the full
/// text rather than being allowed to push the buttons off screen.
const MAX_NOTES_CHARS: usize = 600;

/// The outcomes a check can actually *return*.
///
/// There is deliberately no `Installing` variant. Accepting an update never
/// returns: `download_and_install` ends the process on Windows, and `restart()`
/// diverges. A variant the caller could never observe would be a lie in the
/// API, and the compiler says so - it flags it as never constructed.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CheckOutcome {
    /// Already on the newest version.
    UpToDate,
    /// An update exists and the user chose not to take it right now.
    Declined,
}

fn truncate_notes(body: Option<&str>) -> String {
    let notes = body.unwrap_or("").trim();
    if notes.is_empty() {
        return String::new();
    }
    if notes.chars().count() <= MAX_NOTES_CHARS {
        return format!("\n\n{notes}");
    }
    let clipped: String = notes.chars().take(MAX_NOTES_CHARS).collect();
    format!("\n\n{clipped}…\n\n(See the full release notes on GitHub.)")
}

/// Runs a check. `user_initiated` decides how loudly it reports itself.
pub async fn run_check(app: AppHandle, user_initiated: bool) -> Result<CheckOutcome, String> {
    let updater = app
        .updater()
        .map_err(|e| format!("Could not start the updater: {e}"))?;

    let found = updater
        .check()
        .await
        .map_err(|e| format!("Could not check for updates: {e}"))?;

    let Some(update) = found else {
        if user_initiated {
            app.dialog()
                .message(format!(
                    "Brume {} is the latest version.",
                    app.package_info().version
                ))
                .title("No updates available")
                .kind(MessageDialogKind::Info)
                .blocking_show();
        }
        return Ok(CheckOutcome::UpToDate);
    };

    // The restart warning belongs in this prompt, not after it.
    //
    // On Windows the installer cannot replace a running executable, so
    // download_and_install exits this process as part of doing its job. Any
    // dialog shown after that call would never appear - the user would simply
    // see Brume vanish. Saying it up front is the only honest place to say it.
    let message = format!(
        "Brume {} is available. You have {}.\
         \n\nBrume will close and reopen to finish installing.{}",
        update.version,
        update.current_version,
        truncate_notes(update.body.as_deref())
    );

    let accepted = app
        .dialog()
        .message(message)
        .title("Update available")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Update now".to_string(),
            "Later".to_string(),
        ))
        .blocking_show();

    if !accepted {
        return Ok(CheckOutcome::Declined);
    }

    update
        .download_and_install(|_downloaded, _total| {}, || {})
        .await
        .map_err(|e| format!("The update could not be installed: {e}"))?;

    // Usually unreachable on Windows, because the call above ends the process.
    // Kept for the case where it returns instead, so the new version is running
    // rather than the old one lingering.
    app.restart();
}

/// Fired once at startup when `auto_update` is enabled.
///
/// Swallows every failure on purpose - see the module docs.
pub async fn run_launch_check(app: AppHandle) {
    match run_check(app, false).await {
        Ok(outcome) => {
            if outcome != CheckOutcome::UpToDate {
                eprintln!("[updater] launch check: {outcome:?}");
            }
        }
        Err(err) => {
            // Deliberately only to the console. A browser that greets you with
            // an error box because a server was unreachable is worse than one
            // that quietly tries again next launch.
            eprintln!("[updater] launch check failed: {err}");
        }
    }
}

/// The manual "Check for updates" action. Reports every outcome.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<CheckOutcome, String> {
    let result = run_check(app.clone(), true).await;

    if let Err(ref message) = result {
        app.dialog()
            .message(message.clone())
            .title("Update check failed")
            .kind(MessageDialogKind::Warning)
            .blocking_show();
    }

    result
}
