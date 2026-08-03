//! Local persistence: browsing history and bookmarks.
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
use tauri::{AppHandle, Manager, State};

/// Maximum history entries kept on disk.
///
/// Compaction runs once at startup rather than on every write, so the file can
/// drift above this between sessions. That is deliberate: trimming during a
/// navigation would turn an O(1) append into an O(n) rewrite, which is the exact
/// cost this format exists to avoid.
const HISTORY_CAP: usize = 20_000;

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

pub struct Store {
    history_path: PathBuf,
    bookmarks_path: PathBuf,
    /// Bookmarks live in memory: the list is small, and every read wants all of
    /// it. History is not cached, because it is large and read only when the
    /// user actually opens the panel.
    bookmarks: Mutex<Vec<Bookmark>>,
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
            bookmarks: Mutex::new(Vec::new()),
        };

        let loaded = fs::read_to_string(&store.bookmarks_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<Bookmark>>(strip_bom(&raw)).ok())
            .unwrap_or_default();
        *store.bookmarks.lock().expect("bookmarks mutex poisoned") = loaded;

        store.compact_history();
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

#[tauri::command]
pub fn toggle_bookmark(
    store: State<'_, Store>,
    url: String,
    title: String,
) -> Result<bool, String> {
    store.toggle_bookmark(&url, &title)
}

#[tauri::command]
pub fn remove_bookmark(store: State<'_, Store>, id: u64) -> Result<(), String> {
    store.remove_bookmark(id)
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
    fn bom_prefixed_json_still_parses() {
        // Notepad and PowerShell's Set-Content -Encoding utf8 both add a BOM,
        // and serde_json rejects it outright.
        let raw = "\u{feff}[]";
        assert!(serde_json::from_str::<Vec<Bookmark>>(strip_bom(raw)).is_ok());
        assert!(serde_json::from_str::<Vec<Bookmark>>(raw).is_err());
    }
}
