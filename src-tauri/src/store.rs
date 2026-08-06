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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// Bytes so far, and how many are expected.
    ///
    /// `total` is 0 when the server never said, which is every chunked
    /// response. There is no percentage to draw in that case, so the UI shows
    /// the count alone rather than a bar stuck at zero.
    pub received: i64,
    pub total: i64,
}

/// One row in the address bar dropdown.
///
/// Flattened from two different records on purpose. The chrome should not have
/// to merge a bookmark list and a history list and get the ordering right; it
/// renders whatever order this arrives in.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// "bookmark" or "history". Only used to pick the icon.
    pub kind: &'static str,
    pub title: String,
    pub url: String,
    /// Unix seconds: when it was bookmarked, or last visited.
    pub at: i64,
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
    /// When the chrome was last told about download progress. See
    /// `update_download_progress` for why this is throttled at all.
    last_progress_emit: Mutex<Instant>,
}

/// How often progress may reach the chrome.
///
/// BytesReceivedChanged fires once per network read, which on a fast connection
/// is hundreds of times a second, and every one of those makes the downloads
/// panel rebuild its list. 200ms is faster than anyone reads a number and slow
/// enough to cost nothing.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

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
            last_progress_emit: Mutex::new(Instant::now()),
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

        let needle = query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let mut out: Vec<Visit> = raw
            .lines()
            .rev() // newest first without sorting the whole file
            // A line that will not parse is skipped rather than aborting the
            // read - one torn line from a crash should not hide the rest.
            .filter_map(|line| serde_json::from_str::<Visit>(line).ok())
            .filter(|v| match &needle {
                None => true,
                Some(q) => v.url.to_lowercase().contains(q) || v.title.to_lowercase().contains(q),
            })
            .take(limit)
            .collect();

        out.shrink_to_fit();
        out
    }

    pub fn clear_history(&self) -> Result<(), String> {
        write_atomic(&self.history_path, "")
    }

    /// Removes one visit.
    ///
    /// A full rewrite of the file, which is precisely what the append-only
    /// format exists to avoid. That is fine here: this runs when someone deletes
    /// a row by hand, not on every page load, so paying O(n) once for a thing
    /// that happens rarely is the right trade. Recording a visit stays O(1).
    ///
    /// Matched on the URL *and* the timestamp, so deleting one visit to a page
    /// does not silently take every other visit to it with it. Lines that will
    /// not parse are kept rather than dropped: a torn line is not the line the
    /// user asked to remove, and quietly discarding it would turn a delete into
    /// a repair nobody asked for.
    pub fn remove_visit(&self, url: &str, visited_at: i64) -> Result<(), String> {
        let Ok(raw) = fs::read_to_string(&self.history_path) else {
            return Ok(()); // no file, nothing to remove
        };
        write_atomic(&self.history_path, &without_visit(&raw, url, visited_at))
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

    /// Adds a bookmark unless the URL is already there.
    ///
    /// Add-only, unlike `toggle_bookmark`, which is what the star needs and
    /// exactly the wrong thing for an import: toggling would *remove* every
    /// bookmark the two browsers had in common.
    ///
    /// Returns whether it was added.
    pub fn add_bookmark(&self, url: &str, title: &str, added_at: i64) -> bool {
        if url.is_empty() {
            return false;
        }
        let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
        if items.iter().any(|b| b.url == url) {
            return false;
        }
        let next_id = items.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        items.push(Bookmark {
            id: next_id,
            url: url.to_string(),
            title: title.to_string(),
            // The source's own date, so an import does not claim every
            // bookmark was made today and reorder the bar by accident.
            added_at,
        });
        true
    }

    /// Writes the current list out. For callers that batched several adds.
    pub fn flush_bookmarks(&self) -> Result<(), String> {
        let snapshot = self
            .bookmarks
            .lock()
            .expect("bookmarks mutex poisoned")
            .clone();
        self.persist_bookmarks(&snapshot)
    }

    /// Renames a bookmark, leaving its URL and position alone.
    ///
    /// Worth having on its own once bookmarks can be imported: a title that made
    /// sense as a page title is often not what you want on a 172px strip.
    pub fn rename_bookmark(&self, id: u64, title: &str) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            match items.iter_mut().find(|b| b.id == id) {
                Some(b) => b.title = title.trim().to_string(),
                None => return Ok(()),
            }
            items.clone()
        };
        self.persist_bookmarks(&snapshot)
    }

    pub fn remove_bookmark(&self, id: u64) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            items.retain(|b| b.id != id);
            items.clone()
        };
        self.persist_bookmarks(&snapshot)
    }

    // --- address bar suggestions ----------------------------------------

    /// What to offer for a partly typed address.
    ///
    /// Ranking, decided deliberately and cheap enough to run per keystroke:
    ///
    ///   1. Bookmarks before history. A bookmark is a page someone said they
    ///      wanted back; a history row is only a page they happened to open.
    ///   2. Within each, a prefix match before a substring match. Typing "git"
    ///      should offer github before a page with "git" buried in its title.
    ///   3. Within that, most recent first.
    ///
    /// Deliberately not frecency. History is append-only JSONL with no index, so
    /// counting visits means either a full scan per keystroke or real storage;
    /// store.rs already records that as the point where SQLite earns its size.
    /// Recency answers most of the same question for none of the cost.
    ///
    /// Deduplicated by URL, because a page visited twenty times is twenty lines
    /// in history and one useful suggestion.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }

        let rank = |url: &str, title: &str| match_rank(url, title, &needle);

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut scored: Vec<(u8, u8, i64, Suggestion)> = Vec::new();

        for b in self
            .bookmarks
            .lock()
            .expect("bookmarks mutex poisoned")
            .iter()
        {
            if let Some(r) = rank(&b.url, &b.title) {
                if seen.insert(b.url.clone()) {
                    scored.push((
                        0,
                        r,
                        b.added_at,
                        Suggestion {
                            kind: "bookmark",
                            title: b.title.clone(),
                            url: b.url.clone(),
                            at: b.added_at,
                        },
                    ));
                }
            }
        }

        // `history` already returns newest first and filters by query, so this
        // reads the file once rather than once per candidate. The cap is
        // generous because dedup and ranking happen after it.
        for v in self.history(Some(query), 400) {
            if let Some(r) = rank(&v.url, &v.title) {
                if seen.insert(v.url.clone()) {
                    scored.push((
                        1,
                        r,
                        v.visited_at,
                        Suggestion {
                            kind: "history",
                            title: v.title.clone(),
                            url: v.url.clone(),
                            at: v.visited_at,
                        },
                    ));
                }
            }
        }

        // Kind, then match quality, then newest first.
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(b.2.cmp(&a.2)));
        scored.into_iter().take(limit).map(|s| s.3).collect()
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
            received: 0,
            total: 0,
        });
    }

    /// Records progress against a running download, and says whether the chrome
    /// is due an update.
    ///
    /// The throttle lives here rather than in the chrome so the decision is made
    /// before anything crosses the IPC boundary. A finished download always gets
    /// through, so the bar lands on full instead of stopping wherever the
    /// throttle last let something past.
    ///
    /// A URL with no matching row is ignored rather than inserted. Brume's
    /// DownloadStarting handler and wry's are both subscribed to the same event
    /// and nothing specifies which runs first, so the first tick can arrive
    /// before `begin_download` has made a row for it. It fires many more times.
    pub fn update_download_progress(&self, url: &str, received: i64, total: i64) -> bool {
        let complete = {
            let mut active = self
                .active_downloads
                .lock()
                .expect("active downloads mutex poisoned");
            let Some(row) = active.iter_mut().find(|d| d.url == url) else {
                return false;
            };
            row.received = received;
            row.total = total;
            total > 0 && received >= total
        };

        let mut last = self
            .last_progress_emit
            .lock()
            .expect("progress mutex poisoned");
        if complete || last.elapsed() >= PROGRESS_EMIT_INTERVAL {
            *last = Instant::now();
            return true;
        }
        false
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
            path: path
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
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

/// How well a candidate matches what was typed, or `None` for not at all.
///
/// Lower is better, so the values sort naturally. Free-standing rather than a
/// closure inside `suggest` so the ranking can be tested without a Store and a
/// history file: the ordering *is* the feature, and it is the part most likely
/// to be changed later by someone who has not read why.
///
/// `needle` is expected already lowercased and trimmed by the caller.
fn match_rank(url: &str, title: &str, needle: &str) -> Option<u8> {
    let url_l = url.to_lowercase();
    let title_l = title.to_lowercase();

    // The scheme is noise when matching what someone typed. Nobody types
    // "https://" to find github, so a prefix match has to be against the bare
    // host or every suggestion would rank as a mere substring.
    let bare = url_l
        .strip_prefix("https://")
        .or_else(|| url_l.strip_prefix("http://"))
        .unwrap_or(&url_l);
    let bare = bare.strip_prefix("www.").unwrap_or(bare);

    if bare.starts_with(needle) {
        Some(0)
    } else if title_l.starts_with(needle) {
        Some(1)
    } else if bare.contains(needle) || title_l.contains(needle) {
        Some(2)
    } else {
        None
    }
}

/// History minus one visit, as text ready to write back.
///
/// Split out from `remove_visit` so the line handling can be tested without
/// touching the filesystem. The two properties that matter are both easy to get
/// wrong: only the named visit goes, and a line that will not parse stays.
fn without_visit(raw: &str, url: &str, visited_at: i64) -> String {
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| match serde_json::from_str::<Visit>(line) {
            Ok(v) => !(v.url == url && v.visited_at == visited_at),
            // A torn line is not the line anyone asked to remove, and silently
            // dropping it would turn a delete into a repair.
            Err(_) => true,
        })
        .collect();

    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
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
pub fn history(store: State<'_, Store>, query: Option<String>, limit: Option<usize>) -> Vec<Visit> {
    store.history(query.as_deref(), limit.unwrap_or(300))
}

#[tauri::command]
pub fn clear_history(store: State<'_, Store>) -> Result<(), String> {
    store.clear_history()
}

#[tauri::command]
pub fn remove_visit(store: State<'_, Store>, url: String, visited_at: i64) -> Result<(), String> {
    store.remove_visit(&url, visited_at)
}

#[tauri::command]
pub fn bookmarks(store: State<'_, Store>) -> Vec<Bookmark> {
    store.bookmarks()
}

/// Suggestions for the address bar. Runs per keystroke, so it is kept cheap.
#[tauri::command]
pub fn suggest(store: State<'_, Store>, query: String, limit: Option<usize>) -> Vec<Suggestion> {
    store.suggest(&query, limit.unwrap_or(8))
}

/// Tells the chrome the bookmark list changed.
///
/// Done here rather than left to whoever called the command, because the
/// bookmarks bar renders the list and has no other way to hear about a removal
/// from the panel. Best effort: a failed emit is a stale bar, not a lost
/// bookmark, and the bookmark is already on disk by this point.
fn notify_bookmarks(app: &AppHandle) {
    let _ = app.emit_to(
        crate::browser::CHROME_LABEL,
        crate::browser::BOOKMARKS_EVENT,
        (),
    );
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
pub fn rename_bookmark(
    app: AppHandle,
    store: State<'_, Store>,
    id: u64,
    title: String,
) -> Result<(), String> {
    store.rename_bookmark(id, &title)?;
    notify_bookmarks(&app);
    Ok(())
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
    fn a_prefix_beats_a_substring_and_the_scheme_is_ignored() {
        // The ordering is the feature. 0 beats 1 beats 2, and None is no match.
        assert_eq!(match_rank("https://github.com/", "GitHub", "git"), Some(0));
        // www. is stripped too, or every www host would rank a tier low.
        assert_eq!(
            match_rank("https://www.github.com/", "GitHub", "git"),
            Some(0)
        );
        // Title prefix is worth more than a substring buried in a URL.
        assert_eq!(
            match_rank("https://example.com/x", "Gitting on", "git"),
            Some(1)
        );
        // Both the host and the title only *contain* it, so this is the bottom
        // tier. Note the title matters as much as the URL: "GitLab" as a title
        // would prefix-match and rank a tier higher, which is correct and is
        // why the real gitlab entry ranks below github.
        assert_eq!(
            match_rank("https://about.gitlab.com/", "Software lifecycle", "git"),
            Some(2)
        );
        assert_eq!(match_rank("https://example.com/", "Example", "git"), None);
        // Matching is case-insensitive on both sides; the caller lowercases the
        // needle, so an upper-case page title must still be found.
        assert_eq!(match_rank("https://GITHUB.com/", "GITHUB", "git"), Some(0));
    }

    #[test]
    fn removing_one_visit_leaves_the_others_and_keeps_torn_lines() {
        let line = |url: &str, at: i64| {
            serde_json::to_string(&Visit {
                url: url.into(),
                title: "t".into(),
                visited_at: at,
            })
            .unwrap()
        };
        // Same URL twice, so this also proves a delete does not take every
        // visit to a page with it.
        let raw = format!(
            "{}\n{}\n{}\n{{\"url\": \"https://torn.example",
            line("https://a.test/", 1),
            line("https://a.test/", 2),
            line("https://b.test/", 3),
        );

        let out = without_visit(&raw, "https://a.test/", 2);
        assert!(out.contains("\"visited_at\":1"), "the other visit stays");
        assert!(!out.contains("\"visited_at\":2"), "the named visit goes");
        assert!(out.contains("b.test"), "unrelated rows stay");
        assert!(out.contains("torn.example"), "an unparseable line is kept");
        assert!(out.ends_with('\n'), "the file stays newline terminated");
    }

    #[test]
    fn removing_the_only_visit_leaves_an_empty_file_not_a_stray_newline() {
        let raw = serde_json::to_string(&Visit {
            url: "https://a.test/".into(),
            title: "t".into(),
            visited_at: 7,
        })
        .unwrap();
        assert_eq!(without_visit(&raw, "https://a.test/", 7), "");
    }

    #[test]
    fn a_filename_is_recovered_from_the_url_when_nothing_else_offers_one() {
        // Reached when a download finishes without its start being seen and the
        // runtime reported no path. A row with no name is no use to anyone.
        assert_eq!(
            filename_from_url("https://example.com/a/b/report.pdf"),
            "report.pdf"
        );
        // The query string is not part of the last segment.
        assert_eq!(
            filename_from_url("https://example.com/get.zip?token=abc"),
            "get.zip"
        );
        // Percent-encoding is left as-is rather than decoded: this is a label,
        // and decoding could reintroduce a path separator.
        assert_eq!(
            filename_from_url("https://example.com/my%2Ffile.txt"),
            "my%2Ffile.txt"
        );
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
