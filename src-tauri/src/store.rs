//! Local persistence: browsing history, bookmarks and downloads.
//!
//! # Why not SQLite
//!
//! `rusqlite` bundles the SQLite C library - roughly 1-1.5 MB on a binary that
//! is currently under 5 MB. That is a ~25% increase to store a list of URLs, and
//! hard to justify against the premise of this project. serde_json is already
//! present for settings, so this costs nothing new.
//!
//! # Two files, two shapes, for a reason
//!
//! ```text
//!   history.jsonl     append-only, one JSON object per line
//!   bookmarks.json    a single array, rewritten atomically
//! ```
//!
//! History is **append-only** because it is written on every single page load.
//! Re-serialising an entire growing file that often would be genuinely wasteful -
//! appending one line is O(1) and touches only the tail. JSONL also degrades
//! well: a torn write from a crash costs the last line, not the whole file, and
//! the parser simply skips lines it cannot read.
//!
//! Bookmarks are small, edited rarely, and need whole-list operations like
//! removal, so a plain array rewritten atomically is the better fit.
//!
//! # When to revisit this
//!
//! JSONL means every history query loads and parses the file. At the cap below
//! that is a few megabytes and milliseconds, which is fine for a personal
//! browser. If history ever needs full-text search, or ranking by visit
//! frequency across a much larger corpus, that is the point where SQLite earns
//! its size. The read/write surface is confined to this module so that swap
//! stays contained.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

/// Maximum history entries kept on disk.
///
/// Compaction runs once at startup rather than on every write, so the file can
/// drift above this between sessions. That is deliberate: trimming during a
/// navigation would turn an O(1) append into an O(n) rewrite, which is the exact
/// cost this format exists to avoid.
const HISTORY_CAP: usize = 20_000;

/// Maximum finished downloads kept on disk. Far smaller than history: this is a
/// list a person actually reads, and a thousand old rows help nobody.
const DOWNLOAD_CAP: usize = 500;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        // Only reachable if the system clock is before 1970. Recording 0 beats
        // refusing to record.
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Unix seconds.
    pub visited_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bookmark {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub added_at: i64,
}

/// A finished download, kept on disk.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub url: String,
    pub filename: String,
    /// Where it landed. Empty when the runtime did not say.
    pub path: String,
    pub finished_at: i64,
    pub success: bool,
}

/// A download still running. In memory only.
///
/// Not persisted, because a download interrupted by a quit did not finish and
/// there is nothing useful to resume or report on next launch. Only completed
/// downloads earn a line on disk.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActiveDownload {
    pub url: String,
    pub filename: String,
    pub started_at: i64,
}

/// What the downloads panel renders.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsView {
    pub active: Vec<ActiveDownload>,
    pub finished: Vec<Download>,
}

pub struct Store {
    history_path: PathBuf,
    bookmarks_path: PathBuf,
    downloads_path: PathBuf,
    /// Bookmarks live in memory: the list is small, and every read wants all of
    /// it. History is not cached, because it is large and read only when the
    /// user actually opens the panel.
    bookmarks: Mutex<Vec<Bookmark>>,
    /// Downloads currently running, keyed by nothing in particular: the list is
    /// never more than a handful long, so a scan is cheaper than a map.
    active_downloads: Mutex<Vec<ActiveDownload>>,
}

/// Writes a file atomically.
///
/// Serialising directly over the destination leaves a truncated file if the
/// process dies mid-write, which for bookmarks means losing all of them. Writing
/// a sibling and renaming means the destination is either the old contents or
/// the new ones, never a half-written mixture.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Strips a UTF-8 byte-order mark.
///
/// These files are meant to be inspectable and editable by hand, and on Windows
/// a BOM is easy to acquire by accident - Notepad adds one, so does PowerShell's
/// `Set-Content -Encoding utf8`. serde_json rejects it. Same reasoning as
/// settings.rs, where this cost a real bug.
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

impl Store {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));

        let store = Self {
            history_path: dir.join("history.jsonl"),
            bookmarks_path: dir.join("bookmarks.json"),
            downloads_path: dir.join("downloads.jsonl"),
            bookmarks: Mutex::new(Vec::new()),
            active_downloads: Mutex::new(Vec::new()),
        };

        let loaded = fs::read_to_string(&store.bookmarks_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<Bookmark>>(strip_bom(&raw)).ok())
            .unwrap_or_default();
        *store.bookmarks.lock().expect("bookmarks mutex poisoned") = loaded;

        store.compact_history();
        store.compact_downloads();
        store
    }

    // --- history --------------------------------------------------------

    /// Trims history to the cap, newest kept.
    ///
    /// Runs once at startup. A no-op when the file is already within the cap, so
    /// the common case costs one read and no write.
    fn compact_history(&self) {
        let Ok(raw) = fs::read_to_string(&self.history_path) else {
            return;
        };
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= HISTORY_CAP {
            return;
        }
        let kept = lines[lines.len() - HISTORY_CAP..].join("\n");
        let _ = write_atomic(&self.history_path, &format!("{kept}\n"));
    }

    pub fn record_visit(&self, url: &str, title: &str) {
        // Nothing is gained by recording the blank page a webview shows before
        // its first navigation.
        if url.is_empty() || url.starts_with("about:") {
            return;
        }

        let visit = Visit {
            url: url.to_string(),
            title: title.to_string(),
            visited_at: now_unix(),
        };

        let Ok(line) = serde_json::to_string(&visit) else {
            return;
        };

        if let Some(parent) = self.history_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Append, never rewrite. Failure here is silent on purpose: history is a
        // convenience, and a browser that refuses to navigate because it could
        // not write a log line would be worse than one that quietly forgets.
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Most recent visits first, optionally filtered.
    pub fn history(&self, query: Option<&str>, limit: usize) -> Vec<Visit> {
        let Ok(raw) = fs::read_to_string(&self.history_path) else {
            return Vec::new();
        };

        let needle = query.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());

        let mut out: Vec<Visit> = raw
            .lines()
            .rev() // newest first without sorting the whole file
            // A line that will not parse is skipped rather than aborting the
            // read - one torn line from a crash should not hide the rest.
            .filter_map(|line| serde_json::from_str::<Visit>(line).ok())
            .filter(|v| match &needle {
                None => true,
                Some(q) => {
                    v.url.to_lowercase().contains(q) || v.title.to_lowercase().contains(q)
                }
            })
            .take(limit)
            .collect();

        out.shrink_to_fit();
        out
    }

    pub fn clear_history(&self) -> Result<(), String> {
        write_atomic(&self.history_path, "")
    }

    // --- bookmarks ------------------------------------------------------

    fn persist_bookmarks(&self, items: &[Bookmark]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(items).map_err(|e| e.to_string())?;
        write_atomic(&self.bookmarks_path, &json)
    }

    pub fn bookmarks(&self) -> Vec<Bookmark> {
        self.bookmarks
            .lock()
            .expect("bookmarks mutex poisoned")
            .clone()
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks
            .lock()
            .expect("bookmarks mutex poisoned")
            .iter()
            .any(|b| b.url == url)
    }

    /// Adds a bookmark, or removes it if the URL is already bookmarked.
    ///
    /// One toggle rather than separate add and remove commands, because the
    /// toolbar control is a single star whose meaning depends on current state.
    /// Returns whether the URL is bookmarked afterwards.
    pub fn toggle_bookmark(&self, url: &str, title: &str) -> Result<bool, String> {
        if url.is_empty() {
            return Ok(false);
        }

        let (snapshot, now_bookmarked) = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");

            if let Some(pos) = items.iter().position(|b| b.url == url) {
                items.remove(pos);
                (items.clone(), false)
            } else {
                let next_id = items.iter().map(|b| b.id).max().unwrap_or(0) + 1;
                items.push(Bookmark {
                    id: next_id,
                    url: url.to_string(),
                    title: title.to_string(),
                    added_at: now_unix(),
                });
                (items.clone(), true)
            }
        };

        self.persist_bookmarks(&snapshot)?;
        Ok(now_bookmarked)
    }

    pub fn remove_bookmark(&self, id: u64) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            items.retain(|b| b.id != id);
            items.clone()
        };
        self.persist_bookmarks(&snapshot)
    }

    // --- downloads ------------------------------------------------------

    /// Records a download starting.
    pub fn begin_download(&self, url: &str, filename: &str) {
        let mut active = self
            .active_downloads
            .lock()
            .expect("active downloads mutex poisoned");
        active.push(ActiveDownload {
            url: url.to_string(),
            filename: filename.to_string(),
            started_at: now_unix(),
        });
    }

    /// Moves a download out of the running list and onto disk.
    ///
    /// Matched back to its start by URL, taking the oldest match. The runtime
    /// gives no download id across the two events, and the same URL fetched
    /// twice is the only ambiguous case; oldest-first at least pairs them in
    /// the order they were started.
    pub fn finish_download(&self, url: &str, path: Option<&Path>, success: bool) {
        let started = {
            let mut active = self
                .active_downloads
                .lock()
                .expect("active downloads mutex poisoned");
            active
                .iter()
                .position(|d| d.url == url)
                .map(|i| active.remove(i))
        };

        // Fall back to the URL's last segment: a download can finish without a
        // start ever having been seen, and a row with no name is no use.
        let filename = started
            .as_ref()
            .map(|d| d.filename.clone())
            .filter(|f| !f.is_empty())
            .or_else(|| {
                path.and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| filename_from_url(url));

        let record = Download {
            url: url.to_string(),
            filename,
            path: path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            finished_at: now_unix(),
            success,
        };

        let Ok(line) = serde_json::to_string(&record) else {
            return;
        };
        if let Some(parent) = self.downloads_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Append and stay quiet on failure, same as history: a browser that
        // refused to download because it could not write a log line would be
        // the worse bug.
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.downloads_path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Running downloads first, then finished ones newest-first.
    pub fn downloads(&self, limit: usize) -> DownloadsView {
        let active = self
            .active_downloads
            .lock()
            .expect("active downloads mutex poisoned")
            .clone();

        let finished = fs::read_to_string(&self.downloads_path)
            .map(|raw| {
                raw.lines()
                    .rev()
                    .filter_map(|l| serde_json::from_str::<Download>(l).ok())
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default();

        DownloadsView { active, finished }
    }

    /// Clears the finished list. Running downloads are left alone.
    pub fn clear_downloads(&self) -> Result<(), String> {
        write_atomic(&self.downloads_path, "")
    }

    /// Trims the downloads log, same reasoning as history.
    fn compact_downloads(&self) {
        let Ok(raw) = fs::read_to_string(&self.downloads_path) else {
            return;
        };
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= DOWNLOAD_CAP {
            return;
        }
        let kept = lines[lines.len() - DOWNLOAD_CAP..].join("\n");
        let _ = write_atomic(&self.downloads_path, &format!("{kept}\n"));
    }
}

/// Last path segment of a URL, for when nothing better is available.
fn filename_from_url(url: &str) -> String {
    tauri::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(|x| x.to_string()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "download".to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn history(
    store: State<'_, Store>,
    query: Option<String>,
    limit: Option<usize>,
) -> Vec<Visit> {
    store.history(query.as_deref(), limit.unwrap_or(300))
}

#[tauri::command]
pub fn clear_history(store: State<'_, Store>) -> Result<(), String> {
    store.clear_history()
}

#[tauri::command]
pub fn bookmarks(store: State<'_, Store>) -> Vec<Bookmark> {
    store.bookmarks()
}

/// Tells the chrome the bookmark list changed.
///
/// Done here rather than left to whoever called the command, because the
/// bookmarks bar renders the list and has no other way to hear about a removal
/// from the panel. Best effort: a failed emit is a stale bar, not a lost
/// bookmark, and the bookmark is already on disk by this point.
fn notify_bookmarks(app: &AppHandle) {
    let _ = app.emit_to(crate::browser::CHROME_LABEL, crate::browser::BOOKMARKS_EVENT, ());
}

#[tauri::command]
pub fn toggle_bookmark(
    app: AppHandle,
    store: State<'_, Store>,
    url: String,
    title: String,
) -> Result<bool, String> {
    let bookmarked = store.toggle_bookmark(&url, &title)?;
    notify_bookmarks(&app);
    Ok(bookmarked)
}

#[tauri::command]
pub fn remove_bookmark(app: AppHandle, store: State<'_, Store>, id: u64) -> Result<(), String> {
    store.remove_bookmark(id)?;
    notify_bookmarks(&app);
    Ok(())
}

#[tauri::command]
pub fn downloads(store: State<'_, Store>, limit: Option<usize>) -> DownloadsView {
    store.downloads(limit.unwrap_or(200))
}

#[tauri::command]
pub fn clear_downloads(store: State<'_, Store>) -> Result<(), String> {
    store.clear_downloads()
}

/// Opens Explorer with the file selected.
///
/// Selected rather than opened: running a downloaded file on the user's behalf
/// is not something a browser should do from a list view, and Explorer showing
/// it in place is what every browser's "show in folder" does.
///
/// The path is checked against the recorded downloads before use. It arrives
/// over IPC from the chrome, and handing an arbitrary caller-supplied string to
/// a process launch is how that becomes something worse than it looks.
#[tauri::command]
pub fn reveal_download(store: State<'_, Store>, path: String) -> Result<(), String> {
    let known = store
        .downloads(usize::MAX)
        .finished
        .into_iter()
        .any(|d| d.path == path && !d.path.is_empty());

    if !known {
        return Err("That file is not in the downloads list.".into());
    }
    if !Path::new(&path).exists() {
        return Err("That file is no longer there.".into());
    }

    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open Explorer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torn_line_does_not_hide_the_rest_of_history() {
        // Simulates a crash mid-append: the last line is incomplete.
        let good = serde_json::to_string(&Visit {
            url: "https://example.com".into(),
            title: "Example".into(),
            visited_at: 1,
        })
        .unwrap();
        let raw = format!("{good}\n{{\"url\": \"https://truncated.com");

        let parsed: Vec<Visit> = raw
            .lines()
            .filter_map(|l| serde_json::from_str::<Visit>(l).ok())
            .collect();

        assert_eq!(parsed.len(), 1, "the intact line should still be readable");
        assert_eq!(parsed[0].url, "https://example.com");
    }

    #[test]
    fn a_filename_is_recovered_from_the_url_when_nothing_else_offers_one() {
        // Reached when a download finishes without its start being seen and the
        // runtime reported no path. A row with no name is no use to anyone.
        assert_eq!(filename_from_url("https://example.com/a/b/report.pdf"), "report.pdf");
        // The query string is not part of the last segment.
        assert_eq!(filename_from_url("https://example.com/get.zip?token=abc"), "get.zip");
        // Percent-encoding is left as-is rather than decoded: this is a label,
        // and decoding could reintroduce a path separator.
        assert_eq!(filename_from_url("https://example.com/my%2Ffile.txt"), "my%2Ffile.txt");
    }

    #[test]
    fn a_url_with_no_usable_segment_still_produces_a_name() {
        for url in [
            "https://example.com/",
            "https://example.com",
            "not a url at all",
            "",
        ] {
            assert_eq!(filename_from_url(url), "download", "for {url:?}");
        }
    }

    #[test]
    fn bom_prefixed_json_still_parses() {
        // Notepad and PowerShell's Set-Content -Encoding utf8 both add a BOM,
        // and serde_json rejects it outright.
        let raw = "\u{feff}[]";
        assert!(serde_json::from_str::<Vec<Bookmark>>(strip_bom(raw)).is_ok());
        assert!(serde_json::from_str::<Vec<Bookmark>>(raw).is_err());
    }
}
