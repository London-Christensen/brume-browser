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
use tauri::{AppHandle, State};

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

pub(crate) fn now_unix() -> i64 {
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
    /// The folder this sits in. `None` is the root of the bar.
    ///
    /// A pointer rather than nesting, so the file stays a flat JSON array and
    /// every bookmarks.json written before folders existed is still a valid one.
    /// `#[serde(default)]` is load-bearing rather than tidy: without it those
    /// files fail to parse outright.
    #[serde(default)]
    pub parent: Option<u64>,
    /// A folder rather than a link. `url` is unused and empty when this is set.
    ///
    /// Folders share the id space with bookmarks, which is why `max + 1` still
    /// allocates safely and a folder can never collide with a link.
    #[serde(default)]
    pub is_folder: bool,
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
    /// it.
    bookmarks: Mutex<Vec<Bookmark>>,
    /// History, parsed, while the panel that is reading it stays open.
    ///
    /// It used to be read straight off disk every time, and the comment here
    /// said that was fine "because it is large and read only when the user
    /// actually opens the panel". True until 0.6.0 put a search box on it. With
    /// one, that is 20,000 lines parsed on every keystroke, which is word for
    /// word the bug fixed in 0.3.0.
    ///
    /// Filled on the first read, dropped by `release_history` when the panel
    /// closes, and invalidated by anything that writes. Roughly 3 MB at the cap,
    /// and only while someone is looking at it.
    history_cache: Mutex<Option<Vec<Visit>>>,
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
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
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

/// What was found in the bookmarks file.
///
/// Split out from `Store::load` so the decision can be tested without touching a
/// disk, the same way `without_visit` is. The distinction that matters is the
/// last variant: a file that is absent and a file that is unreadable are not the
/// same thing, and treating them alike is what made this a data-loss bug.
enum BookmarksFile {
    /// Nothing on disk. Normal on first run, and nothing to preserve.
    Missing,
    /// Parsed cleanly.
    Parsed(Vec<Bookmark>),
    /// Present and unreadable. The bytes on disk are still the only copy.
    Unreadable,
}

/// Classifies the bookmarks file. `None` in means it was not there at all.
fn read_bookmarks(raw: Option<&str>) -> BookmarksFile {
    let Some(text) = raw else {
        return BookmarksFile::Missing;
    };
    match serde_json::from_str::<Vec<Bookmark>>(strip_bom(text)) {
        Ok(items) => BookmarksFile::Parsed(items),
        Err(_) => BookmarksFile::Unreadable,
    }
}

/// Forces the bookmark list back into a shape the rest of the code can trust.
///
/// Every invariant lives here rather than at each call site, so there is one
/// place to read and one place to get wrong. Two things are repaired:
///
///   1. A `parent` that does not name a real folder is reset to root. That
///      covers a dangling id, a link used as a parent, and a file edited by
///      hand.
///   2. A cycle is broken by detaching whatever cannot reach the root. A folder
///      that is its own ancestor would otherwise be invisible in the tree while
///      still taking up space in the file, and every walk over it would loop.
///
/// Run on load, so nothing downstream ever has to cope with a broken tree. The
/// second pass is quadratic in the worst case, which is irrelevant here: this is
/// a bookmarks list, and it runs once at startup.
fn repair_tree(items: &mut [Bookmark]) {
    let folders: std::collections::HashSet<u64> =
        items.iter().filter(|b| b.is_folder).map(|b| b.id).collect();
    for b in items.iter_mut() {
        if let Some(p) = b.parent {
            if !folders.contains(&p) {
                b.parent = None;
            }
        }
    }

    // Anything that can walk up to the root is fine. Grow that set until it
    // stops growing; whatever is left over is in a cycle.
    let mut rooted: std::collections::HashSet<u64> = items
        .iter()
        .filter(|b| b.parent.is_none())
        .map(|b| b.id)
        .collect();
    loop {
        let mut grew = false;
        for b in items.iter() {
            if rooted.contains(&b.id) {
                continue;
            }
            if let Some(p) = b.parent {
                if rooted.contains(&p) {
                    rooted.insert(b.id);
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    for b in items.iter_mut() {
        if !rooted.contains(&b.id) {
            b.parent = None;
        }
    }
}

/// Appends an entry and returns its id.
///
/// The one place ids are allocated. `max + 1` over the whole list means folders
/// and links share a space and can never collide. It does reuse an id after the
/// highest-numbered entry is deleted, which `repair_tree` cannot detect because
/// such a parent still resolves, just to the wrong entry. Left as it is
/// deliberately: a persisted counter would mean a top-level object rather than
/// an array, and that shape change is what the folder model was built to avoid.
fn push_entry(
    items: &mut Vec<Bookmark>,
    url: &str,
    title: &str,
    added_at: i64,
    parent: Option<u64>,
    is_folder: bool,
) -> u64 {
    let id = items.iter().map(|b| b.id).max().unwrap_or(0) + 1;
    items.push(Bookmark {
        id,
        url: url.to_string(),
        title: title.to_string(),
        added_at,
        parent,
        is_folder,
    });
    id
}

/// Removes one entry, lifting a folder's direct children up a level.
///
/// Split from `Store::remove_bookmark` so the promotion can be tested without a
/// disk. Only direct children move: anything deeper keeps its own parent and
/// travels with it.
fn remove_entry(items: &mut Vec<Bookmark>, id: u64) {
    let Some(pos) = items.iter().position(|b| b.id == id) else {
        return;
    };
    let gone = items.remove(pos);
    if gone.is_folder {
        for b in items.iter_mut() {
            if b.parent == Some(id) {
                b.parent = gone.parent;
            }
        }
    }
}

/// Moves an entry to position `index` among its siblings.
///
/// Split out because the arithmetic is the easy part to get wrong, and it was:
/// the first version moved an entry that was already in the right place, past
/// unrelated entries sitting between it and its next sibling. Sibling order came
/// out correct, so the UI would have looked right while the file churned on
/// every no-op move.
///
/// Two rules keep it honest. An entry already at `index` is left alone. And the
/// insertion point is taken from the sibling that should precede it, not from
/// the end of the array, so an entry stays next to its own siblings instead of
/// drifting past everything else.
fn move_within_siblings(items: &mut Vec<Bookmark>, id: u64, index: usize) {
    let Some(from) = items.iter().position(|b| b.id == id) else {
        return;
    };
    let parent = items[from].parent;

    let sibs: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent == parent)
        .map(|(i, _)| i)
        .collect();
    // It is its own sibling, so this cannot be empty and cannot miss.
    let current = sibs.iter().position(|&i| i == from).unwrap_or(0);
    let target = index.min(sibs.len().saturating_sub(1));
    if current == target {
        return;
    }

    let entry = items.remove(from);
    // Recomputed after the removal: every later index has shifted down by one.
    let sibs: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent == parent)
        .map(|(i, _)| i)
        .collect();
    let to = if index == 0 {
        sibs.first().copied().unwrap_or(items.len())
    } else if let Some(&prev) = sibs.get(index - 1) {
        prev + 1
    } else {
        // Past the end of the sibling list means straight after the last one.
        sibs.last().map(|&i| i + 1).unwrap_or(items.len())
    };
    items.insert(to, entry);
}

/// Whether moving `id` under `new_parent` would make it its own ancestor.
///
/// Checked before a move rather than repaired after one, because refusing is
/// honest and detaching the subtree afterwards would silently move things the
/// user never touched.
fn would_cycle(items: &[Bookmark], id: u64, new_parent: Option<u64>) -> bool {
    let mut cursor = new_parent;
    // Bounded by the list length: a malformed file cannot spin this forever.
    for _ in 0..=items.len() {
        match cursor {
            None => return false,
            Some(p) if p == id => return true,
            Some(p) => cursor = items.iter().find(|b| b.id == p).and_then(|b| b.parent),
        }
    }
    true
}

/// Moves an unreadable bookmarks file aside so its contents survive.
///
/// Mirrors settings.rs, including overwriting an older `.bak`: two corruptions
/// in a row means the first backup describes a list the user has already carried
/// on without, so the newer file is the more useful one to keep.
fn preserve_unreadable(path: &Path) {
    let backup = path.with_extension("json.bak");
    match fs::rename(path, &backup) {
        Ok(()) => eprintln!(
            "[store] {} could not be parsed; the original was kept at {}",
            path.display(),
            backup.display()
        ),
        Err(e) => eprintln!(
            "[store] could not parse or preserve {}: {e}",
            path.display()
        ),
    }
}

impl Store {
    /// Loads from a directory it is told about rather than one it resolves.
    ///
    /// Since 0.9.0 the directory belongs to a profile, and only `profiles.rs`
    /// knows which one is active. Resolving it here as well would be a second
    /// answer to the same question, and the two would eventually differ.
    pub fn load(app: &AppHandle, dir: PathBuf) -> Self {
        let _ = app;
        // Created up front. A named profile's directory does not exist until
        // something writes to it, and the first write here is an atomic rename
        // that would fail against a missing parent.
        let _ = fs::create_dir_all(&dir);

        let store = Self {
            history_path: dir.join("history.jsonl"),
            bookmarks_path: dir.join("bookmarks.json"),
            downloads_path: dir.join("downloads.jsonl"),
            bookmarks: Mutex::new(Vec::new()),
            history_cache: Mutex::new(None),
            active_downloads: Mutex::new(Vec::new()),
            last_progress_emit: Mutex::new(Instant::now()),
        };

        // A bookmarks file that exists but will not parse is kept, not replaced.
        //
        // This was `.ok().unwrap_or_default()`, which turned every parse failure
        // into an empty list, and the next bookmark added then wrote that empty
        // list back over the file. There was no `.bak` either, so the bookmarks
        // were not merely unreadable, they were gone, and nothing said so.
        let raw = fs::read_to_string(&store.bookmarks_path).ok();
        let mut loaded = match read_bookmarks(raw.as_deref()) {
            BookmarksFile::Parsed(items) => items,
            BookmarksFile::Missing => Vec::new(),
            BookmarksFile::Unreadable => {
                preserve_unreadable(&store.bookmarks_path);
                Vec::new()
            }
        };
        repair_tree(&mut loaded);
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

        // Appended to the cache rather than invalidating it. Browsing with the
        // history panel open would otherwise re-parse the whole file on every
        // page load, which is the cost the cache exists to avoid.
        if let Some(cached) = self
            .history_cache
            .lock()
            .expect("history cache poisoned")
            .as_mut()
        {
            cached.push(visit);
        }
    }

    /// Throws away the parsed history.
    ///
    /// Called by every write, and by `release_history` when the panel closes. A
    /// stale cache here would show a page that was just deleted, or hide one
    /// just visited.
    fn invalidate_history(&self) {
        *self.history_cache.lock().expect("history cache poisoned") = None;
    }

    /// Drops the cache. The panel calls this when it closes.
    pub fn release_history(&self) {
        self.invalidate_history();
    }

    /// Most recent visits first, optionally filtered.
    ///
    /// The parse is cached, so typing in the search box does not re-read the
    /// file per keystroke. Filtering still runs over every visit, which is a
    /// scan of a few tens of thousands of short strings and costs nothing next
    /// to the parse it replaces.
    pub fn history(&self, query: Option<&str>, limit: usize) -> Vec<Visit> {
        let mut cache = self.history_cache.lock().expect("history cache poisoned");
        if cache.is_none() {
            let parsed = match fs::read_to_string(&self.history_path) {
                Ok(raw) => raw
                    .lines()
                    // A line that will not parse is skipped rather than aborting
                    // the read - one torn line from a crash should not hide the
                    // rest.
                    .filter_map(|line| serde_json::from_str::<Visit>(line).ok())
                    .collect(),
                Err(_) => Vec::new(),
            };
            *cache = Some(parsed);
        }
        let all = cache.as_ref().expect("just filled");

        let needle = query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let mut out: Vec<Visit> = all
            .iter()
            .rev() // newest first without sorting: the file is already in order
            .filter(|v| match &needle {
                None => true,
                Some(q) => v.url.to_lowercase().contains(q) || v.title.to_lowercase().contains(q),
            })
            .take(limit)
            .cloned()
            .collect();

        out.shrink_to_fit();
        out
    }

    /// Clears history, either entirely or back to a cutoff.
    ///
    /// `since` is a Unix timestamp: everything visited at or after it goes, and
    /// anything older stays. `None` clears the lot.
    pub fn clear_history(&self, since: Option<i64>) -> Result<(), String> {
        self.invalidate_history();
        match since {
            None => write_atomic(&self.history_path, ""),
            Some(cutoff) => {
                let Ok(raw) = fs::read_to_string(&self.history_path) else {
                    return Ok(()); // no file, nothing to clear
                };
                write_atomic(&self.history_path, &without_visits_since(&raw, cutoff))
            }
        }
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
        self.invalidate_history();
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
            .any(|b| !b.is_folder && b.url == url)
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

            if let Some(pos) = items.iter().position(|b| !b.is_folder && b.url == url) {
                items.remove(pos);
                (items.clone(), false)
            } else {
                // The star bookmarks to the root. Filing it somewhere is a
                // separate action, in the manager.
                push_entry(&mut items, url, title, now_unix(), None, false);
                (items.clone(), true)
            }
        };

        self.persist_bookmarks(&snapshot)?;
        Ok(now_bookmarked)
    }

    /// Adds a bookmark without writing, for callers batching several inserts.
    ///
    /// Add-only, unlike `toggle_bookmark`, which is what the star needs and
    /// exactly the wrong thing for an import: toggling would *remove* every
    /// bookmark the two browsers had in common.
    ///
    /// No duplicate check either, which it used to have. Import now lands in its
    /// own dated folder, so the source tree is reproduced as it stands and a URL
    /// held in two of its folders stays in both. Skipping duplicates would have
    /// made a second import produce an empty folder, which reads as broken.
    /// Nothing is ever removed, which was the actual guarantee worth keeping.
    ///
    /// Returns whether it was added.
    pub fn add_bookmark(&self, url: &str, title: &str, added_at: i64, parent: Option<u64>) -> bool {
        if url.is_empty() {
            return false;
        }
        let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
        // The source's own date, so an import does not claim every bookmark was
        // made today and reorder the bar by accident.
        push_entry(&mut items, url, title, added_at, parent, false);
        true
    }

    /// Adds a folder without writing, for callers batching several inserts.
    ///
    /// The unvalidated sibling of `create_folder`: an import builds parents
    /// before children, so every parent it names is one it just made.
    pub fn add_folder(&self, title: &str, parent: Option<u64>) -> u64 {
        let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
        push_entry(&mut items, "", title, now_unix(), parent, true)
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

    /// Removes a bookmark, or a folder without its contents.
    ///
    /// Deleting a folder moves what was inside it up one level rather than
    /// taking it along. Chrome and Edge delete the subtree, and this
    /// deliberately does not: bookmarks.json has no undo and, until 0.6.0, no
    /// backup either, so no single click should be able to lose a collection.
    /// The cost is that emptying a folder scatters its contents into the level
    /// above instead of tidying them away.
    pub fn remove_bookmark(&self, id: u64) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            remove_entry(&mut items, id);
            items.clone()
        };
        self.persist_bookmarks(&snapshot)
    }

    /// Creates a folder and returns its id.
    ///
    /// Empty titles are rejected rather than defaulted: an unnamed folder in the
    /// bar is a blank chip nobody can identify or find again.
    pub fn create_folder(&self, title: &str, parent: Option<u64>) -> Result<u64, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("a folder needs a name".into());
        }
        let (snapshot, id) = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            if let Some(p) = parent {
                if !items.iter().any(|b| b.id == p && b.is_folder) {
                    return Err("no such folder".into());
                }
            }
            let id = push_entry(&mut items, "", title, now_unix(), parent, true);
            (items.clone(), id)
        };
        self.persist_bookmarks(&snapshot)?;
        Ok(id)
    }

    /// Files a bookmark or folder under another folder, or at the root.
    pub fn move_bookmark(&self, id: u64, parent: Option<u64>) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            if !items.iter().any(|b| b.id == id) {
                return Ok(());
            }
            if let Some(p) = parent {
                if p == id {
                    return Err("a folder cannot contain itself".into());
                }
                if !items.iter().any(|b| b.id == p && b.is_folder) {
                    return Err("no such folder".into());
                }
                if would_cycle(&items, id, parent) {
                    return Err("that would put a folder inside itself".into());
                }
            }
            match items.iter_mut().find(|b| b.id == id) {
                Some(b) => b.parent = parent,
                None => return Ok(()),
            }
            items.clone()
        };
        self.persist_bookmarks(&snapshot)
    }

    /// Moves an entry to a position among its siblings.
    ///
    /// Order is the array's own order within a parent, so there is no position
    /// field to keep consistent. `index` counts only siblings; anything past the
    /// end lands last.
    pub fn reorder_bookmark(&self, id: u64, index: usize) -> Result<(), String> {
        let snapshot = {
            let mut items = self.bookmarks.lock().expect("bookmarks mutex poisoned");
            move_within_siblings(&mut items, id, index);
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
            // A folder has no URL to go to. Offering one would put a row in the
            // dropdown that navigates nowhere.
            if b.is_folder {
                continue;
            }
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

/// History minus everything at or after `cutoff`, ready to write back.
///
/// Split out to be testable, same as `without_visit`. A line that will not parse
/// is kept: it cannot be shown to be recent, and throwing away what it cannot
/// read would make "clear the last hour" quietly delete a torn line from years
/// ago.
fn without_visits_since(raw: &str, cutoff: i64) -> String {
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| match serde_json::from_str::<Visit>(line) {
            Ok(v) => v.visited_at < cutoff,
            Err(_) => true,
        })
        .collect();
    if kept.is_empty() {
        String::new()
    } else {
        format!("{}\n", kept.join("\n"))
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
pub fn clear_history(store: State<'_, Store>, since: Option<i64>) -> Result<(), String> {
    store.clear_history(since)
}

/// Frees the parsed history when the panel that was reading it closes.
#[tauri::command]
pub fn release_history(store: State<'_, Store>) {
    store.release_history();
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
    // Every window. The list is app-wide, so a bar in a second window is just
    // as stale after a change as the one that caused it.
    crate::browser::notify_bookmarks_everywhere(app);
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
pub fn create_folder(
    app: AppHandle,
    store: State<'_, Store>,
    title: String,
    parent: Option<u64>,
) -> Result<u64, String> {
    let id = store.create_folder(&title, parent)?;
    notify_bookmarks(&app);
    Ok(id)
}

#[tauri::command]
pub fn move_bookmark(
    app: AppHandle,
    store: State<'_, Store>,
    id: u64,
    parent: Option<u64>,
) -> Result<(), String> {
    store.move_bookmark(id, parent)?;
    notify_bookmarks(&app);
    Ok(())
}

#[tauri::command]
pub fn reorder_bookmark(
    app: AppHandle,
    store: State<'_, Store>,
    id: u64,
    index: usize,
) -> Result<(), String> {
    store.reorder_bookmark(id, index)?;
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

    #[test]
    fn clearing_a_range_keeps_everything_older_than_the_cutoff() {
        let line = |url: &str, at: i64| {
            serde_json::to_string(&Visit {
                url: url.into(),
                title: "t".into(),
                visited_at: at,
            })
            .unwrap()
        };
        let raw = format!(
            "{}\n{}\n{}\n{{\"url\": \"https://torn.example",
            line("https://old.test/", 1_000),
            line("https://edge.test/", 2_000),
            line("https://new.test/", 3_000),
        );

        // The cutoff is inclusive: a visit exactly at it is inside the range
        // being cleared, or "the last hour" would leave the oldest second.
        let out = without_visits_since(&raw, 2_000);
        assert!(out.contains("old.test"), "older than the cutoff stays");
        assert!(!out.contains("edge.test"), "exactly at the cutoff goes");
        assert!(!out.contains("new.test"), "newer than the cutoff goes");
        assert!(
            out.contains("torn.example"),
            "a line that will not parse is kept: it cannot be shown to be recent"
        );

        // A cutoff before everything is the same as clearing the lot, except
        // for the torn line, which still cannot be judged.
        let all = without_visits_since(&raw, 0);
        assert!(!all.contains("old.test") && !all.contains("new.test"));
        assert!(all.contains("torn.example"));
    }

    #[test]
    fn a_missing_bookmarks_file_is_not_the_same_as_an_unreadable_one() {
        // The entire point of the fix. Both used to produce an empty list, and
        // the next edit wrote that back over whatever had been there.
        assert!(matches!(read_bookmarks(None), BookmarksFile::Missing));
        assert!(matches!(
            read_bookmarks(Some("{not json")),
            BookmarksFile::Unreadable
        ));
    }

    #[test]
    fn an_unreadable_bookmarks_file_is_never_reported_as_empty() {
        // Each of these is a real way the file gets damaged: a write truncated
        // by a crash, a hand edit that drops a bracket, and the wrong shape
        // entirely, which is what a type change to Bookmark would produce.
        for raw in ["", "[{\"id\":1,", "{\"bookmarks\":[]}"] {
            assert!(
                matches!(read_bookmarks(Some(raw)), BookmarksFile::Unreadable),
                "{raw:?} must not be mistaken for an empty list"
            );
        }
        // An empty list, though, really is one, and must not go to .bak.
        assert!(matches!(read_bookmarks(Some("[]")), BookmarksFile::Parsed(b) if b.is_empty()));
    }

    /// Shorthand for the tree tests. `p` is the parent, `f` whether it folders.
    fn node(id: u64, p: Option<u64>, f: bool) -> Bookmark {
        Bookmark {
            id,
            url: if f {
                String::new()
            } else {
                format!("https://{id}.test/")
            },
            title: format!("n{id}"),
            added_at: id as i64,
            parent: p,
            is_folder: f,
        }
    }

    #[test]
    fn a_bookmarks_file_written_before_folders_existed_still_loads() {
        // The upgrade path, and the reason both fields carry serde(default).
        // Without them this parses as corrupt and 0.6.0 eats everyone's
        // bookmarks the first time they open it.
        let old = r#"[{"id":1,"url":"https://a.test/","title":"A","added_at":5}]"#;
        let BookmarksFile::Parsed(items) = read_bookmarks(Some(old)) else {
            panic!("a pre-folder bookmarks file must still parse");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].parent, None, "it lands at the root");
        assert!(!items[0].is_folder);
    }

    #[test]
    fn repair_drops_parents_that_do_not_name_a_real_folder() {
        let mut items = vec![
            node(1, None, true),      // a real folder
            node(2, Some(1), false),  // correctly filed, must be left alone
            node(3, Some(99), false), // parent does not exist
            node(4, Some(2), false),  // parent is a bookmark, not a folder
        ];
        repair_tree(&mut items);
        assert_eq!(items[1].parent, Some(1), "a valid parent is not touched");
        assert_eq!(items[2].parent, None, "a dangling parent goes to root");
        assert_eq!(items[3].parent, None, "a link cannot be a parent");
    }

    #[test]
    fn repair_breaks_a_cycle_rather_than_looping_on_it() {
        // Two folders each claiming the other. Reachable from nothing, so both
        // detach; without this every walk over the tree spins forever.
        let mut items = vec![node(1, Some(2), true), node(2, Some(1), true)];
        repair_tree(&mut items);
        assert!(items.iter().all(|b| b.parent.is_none()));
    }

    #[test]
    fn repair_leaves_a_healthy_tree_exactly_as_it_found_it() {
        // The control. Without it, "reset every parent to root" would pass all
        // three tests above.
        let before = vec![
            node(1, None, true),
            node(2, Some(1), true),
            node(3, Some(2), false),
            node(4, None, false),
        ];
        let mut after = before.clone();
        repair_tree(&mut after);
        let parents: Vec<_> = after.iter().map(|b| b.parent).collect();
        assert_eq!(parents, vec![None, Some(1), Some(2), None]);
    }

    #[test]
    fn a_folder_cannot_be_moved_inside_itself_or_its_own_descendant() {
        let items = vec![
            node(1, None, true),
            node(2, Some(1), true),
            node(3, Some(2), true),
            node(4, None, true),
        ];
        assert!(would_cycle(&items, 1, Some(1)), "into itself");
        assert!(would_cycle(&items, 1, Some(3)), "into its own grandchild");
        assert!(!would_cycle(&items, 1, Some(4)), "into an unrelated folder");
        assert!(!would_cycle(&items, 1, None), "out to the root");
    }

    #[test]
    fn deleting_a_folder_keeps_what_was_inside_it() {
        // The deliberate difference from Chrome. Direct children come up one
        // level; a grandchild stays with its own parent and travels with it.
        let mut items = vec![
            node(1, None, true),
            node(2, Some(1), true),
            node(3, Some(2), false),
            node(4, Some(1), false),
        ];
        remove_entry(&mut items, 1);
        assert_eq!(items.len(), 3, "nothing else was removed");
        assert_eq!(items[0].parent, None, "the subfolder came up to root");
        assert_eq!(items[1].parent, Some(2), "the grandchild stayed put");
        assert_eq!(items[2].parent, None, "the loose bookmark came up too");
    }

    #[test]
    fn reordering_counts_siblings_and_ignores_everything_else() {
        // Interleaved parents, so a naive absolute-index move lands wrong.
        let order = |items: &[Bookmark]| items.iter().map(|b| b.id).collect::<Vec<_>>();
        let start = vec![
            node(1, None, true),
            node(10, Some(1), false),
            node(20, None, false),
            node(11, Some(1), false),
            node(12, Some(1), false),
        ];

        let mut items = start.clone();
        move_within_siblings(&mut items, 12, 0);
        assert_eq!(order(&items), vec![1, 12, 10, 20, 11], "to the front");

        let mut items = start.clone();
        move_within_siblings(&mut items, 10, 2);
        assert_eq!(order(&items), vec![1, 20, 11, 12, 10], "to the back");

        let mut items = start.clone();
        move_within_siblings(&mut items, 10, 99);
        assert_eq!(
            order(&items),
            vec![1, 20, 11, 12, 10],
            "past the end is last"
        );

        // Already first, so nothing should move: not even past the unrelated
        // entry sitting between it and its next sibling.
        let mut items = start.clone();
        move_within_siblings(&mut items, 10, 0);
        assert_eq!(order(&items), order(&start), "a no-op move changes nothing");
    }

    #[test]
    fn a_bom_does_not_make_the_bookmarks_file_look_unreadable() {
        // Without strip_bom in this path, a file Notepad had touched would be
        // moved aside and every bookmark would vanish from the UI.
        let one = serde_json::to_string(&vec![node(1, None, false)]).unwrap();
        let out = read_bookmarks(Some(&format!("\u{feff}{one}")));
        assert!(matches!(out, BookmarksFile::Parsed(b) if b.len() == 1));
    }
}
