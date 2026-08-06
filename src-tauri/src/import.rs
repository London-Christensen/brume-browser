//! Bringing bookmarks over from another browser.
//!
//! # Why this exists at all
//!
//! Nobody moves browser without their bookmarks. Everything else Brume does is
//! irrelevant if the first thing it asks is that you rebuild a list you have
//! spent years collecting.
//!
//! # Only Chromium browsers, and that is a deliberate stopping point
//!
//! Chrome, Edge, Brave and the rest keep bookmarks in a **plain JSON file** at a
//! predictable path, so reading them needs nothing but serde and a path. Firefox
//! keeps them in `places.sqlite`, which would mean either bundling a SQLite
//! driver - the ~1.5 MB `store.rs` explicitly refused for Brume's own storage -
//! or parsing the file format by hand.
//!
//! Taking the easy 90% now beats taking none of it while waiting to do all of
//! it. Firefox is noted here rather than half-attempted.
//!
//! # Folders are flattened, and the UI says so
//!
//! Brume's bookmarks are a flat list, so a folder tree cannot survive the trip.
//! Flattening loses the grouping but keeps every bookmark, which is the right
//! way round: a bookmark you can find in a long list still works, and one that
//! was silently dropped does not.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// A browser Brume can read bookmarks out of.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    /// Stable id passed back to `import_bookmarks`.
    pub id: String,
    pub name: String,
    /// How many bookmarks are in there, so the button can say what it will do.
    pub count: usize,
}

/// One node of a Chromium bookmarks file.
///
/// Only the fields Brume reads. Chromium writes several more - guid, meta_info,
/// sync ids - and serde ignores what is not asked for, so a browser adding
/// another one does not break this.
#[derive(Deserialize)]
struct Node {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    children: Vec<Node>,
    /// Chromium's own epoch, as a decimal string. See `chrome_time_to_unix`.
    #[serde(default)]
    date_added: String,
}

#[derive(Deserialize)]
struct Roots {
    #[serde(default)]
    bookmark_bar: Option<Node>,
    #[serde(default)]
    other: Option<Node>,
    #[serde(default)]
    synced: Option<Node>,
}

#[derive(Deserialize)]
struct BookmarksFile {
    roots: Roots,
}

/// Chromium counts microseconds since 1601-01-01; Unix counts seconds since
/// 1970-01-01. The gap between those two epochs is a fixed 11644473600 seconds.
///
/// Anything unparseable or before the Unix epoch falls back to 0, which reads as
/// "long ago" and sorts to the end rather than claiming today's date.
fn chrome_time_to_unix(raw: &str) -> i64 {
    raw.parse::<i64>()
        .map(|micros| micros / 1_000_000 - 11_644_473_600)
        .unwrap_or(0)
        .max(0)
}

/// Where each browser keeps its default profile's bookmarks.
///
/// All of these are the same Chromium layout, which is the whole reason this is
/// cheap. Only the default profile: picking between profiles is a question the
/// user cannot answer from a Settings row, and getting it wrong imports someone
/// else's bookmarks.
fn candidates() -> Vec<(&'static str, &'static str, PathBuf)> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let base = PathBuf::from(local);
    vec![
        (
            "edge",
            "Microsoft Edge",
            base.join(r"Microsoft\Edge\User Data\Default\Bookmarks"),
        ),
        (
            "chrome",
            "Google Chrome",
            base.join(r"Google\Chrome\User Data\Default\Bookmarks"),
        ),
        (
            "brave",
            "Brave",
            base.join(r"BraveSoftware\Brave-Browser\User Data\Default\Bookmarks"),
        ),
        (
            "vivaldi",
            "Vivaldi",
            base.join(r"Vivaldi\User Data\Default\Bookmarks"),
        ),
        (
            "opera",
            "Opera",
            base.join(r"..\Roaming\Opera Software\Opera Stable\Bookmarks"),
        ),
    ]
}

/// Flattens a node tree into (url, title, added_at).
fn collect(node: &Node, out: &mut Vec<(String, String, i64)>) {
    if node.kind == "url" {
        if let Some(url) = &node.url {
            // Only real web addresses. A Chromium file can carry javascript:
            // bookmarklets, and importing one would put a script behind a
            // one-click button on the bookmarks bar.
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push((
                    url.clone(),
                    node.name.clone(),
                    chrome_time_to_unix(&node.date_added),
                ));
            }
        }
        return;
    }
    for child in &node.children {
        collect(child, out);
    }
}

fn read_source(path: &PathBuf) -> Option<Vec<(String, String, i64)>> {
    let raw = std::fs::read_to_string(path).ok()?;
    // Same BOM guard the rest of Brume's file reading uses.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let parsed: BookmarksFile = serde_json::from_str(raw).ok()?;

    let mut out = Vec::new();
    for root in [
        &parsed.roots.bookmark_bar,
        &parsed.roots.other,
        &parsed.roots.synced,
    ]
    .into_iter()
    .flatten()
    {
        collect(root, &mut out);
    }
    Some(out)
}

/// Which browsers are actually installed and have bookmarks to give.
///
/// Reads each file rather than only checking it exists, so the count is real and
/// a browser with an empty list does not offer an import that does nothing.
#[tauri::command]
pub fn import_sources() -> Vec<ImportSource> {
    candidates()
        .into_iter()
        .filter_map(|(id, name, path)| {
            let found = read_source(&path)?;
            if found.is_empty() {
                return None;
            }
            Some(ImportSource {
                id: id.to_string(),
                name: name.to_string(),
                count: found.len(),
            })
        })
        .collect()
}

/// Copies one browser's bookmarks in. Returns how many were added.
///
/// Duplicates by URL are skipped rather than merged or replaced: running the
/// import twice is a thing people do, and it should be a no-op the second time
/// instead of doubling the list.
#[tauri::command]
pub fn import_bookmarks(
    app: AppHandle,
    store: State<'_, crate::store::Store>,
    source: String,
) -> Result<usize, String> {
    let (_, _, path) = candidates()
        .into_iter()
        .find(|(id, _, _)| *id == source)
        .ok_or_else(|| format!("Unknown source: {source}"))?;

    let found = read_source(&path)
        .ok_or("Could not read that browser's bookmarks. Is it still installed?")?;

    let mut added = 0usize;
    for (url, title, at) in found {
        if store.add_bookmark(&url, &title, at) {
            added += 1;
        }
    }
    // Written once at the end rather than per bookmark. Importing a thousand
    // would otherwise rewrite the file a thousand times.
    store.flush_bookmarks()?;

    let _ = app.emit_to(
        crate::browser::CHROME_LABEL,
        crate::browser::BOOKMARKS_EVENT,
        (),
    );
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_timestamps_become_unix_seconds() {
        // 13412390135172997 microseconds since 1601 is 2026-02-01 or thereabouts;
        // the exact date matters less than it landing in a sane modern range.
        let t = chrome_time_to_unix("13412390135172997");
        assert!(t > 1_700_000_000, "should be well after 2023, got {t}");
        assert!(t < 2_000_000_000, "should be well before 2033, got {t}");

        // Chromium writes "0" for never-used entries, and garbage should not
        // panic or produce a date in the future.
        assert_eq!(chrome_time_to_unix("0"), 0);
        assert_eq!(chrome_time_to_unix(""), 0);
        assert_eq!(chrome_time_to_unix("not a number"), 0);
    }

    #[test]
    fn only_web_urls_are_imported() {
        // A javascript: bookmarklet on the bookmarks bar would be a one-click
        // script, so the filter is a security decision rather than tidiness.
        let node = Node {
            name: "n".into(),
            kind: "folder".into(),
            url: None,
            date_added: "0".into(),
            children: vec![
                Node {
                    name: "ok".into(),
                    kind: "url".into(),
                    url: Some("https://a.test/".into()),
                    children: vec![],
                    date_added: "0".into(),
                },
                Node {
                    name: "plain".into(),
                    kind: "url".into(),
                    url: Some("http://b.test/".into()),
                    children: vec![],
                    date_added: "0".into(),
                },
                Node {
                    name: "bad".into(),
                    kind: "url".into(),
                    url: Some("javascript:alert(1)".into()),
                    children: vec![],
                    date_added: "0".into(),
                },
                Node {
                    name: "file".into(),
                    kind: "url".into(),
                    url: Some("file:///C:/x".into()),
                    children: vec![],
                    date_added: "0".into(),
                },
            ],
        };
        let mut out = Vec::new();
        collect(&node, &mut out);
        assert_eq!(out.len(), 2, "only http and https should survive");
        assert!(out.iter().all(|(u, _, _)| u.starts_with("http")));
    }

    #[test]
    fn nested_folders_are_flattened_not_dropped() {
        let deep = Node {
            name: "root".into(),
            kind: "folder".into(),
            url: None,
            date_added: "0".into(),
            children: vec![Node {
                name: "inner".into(),
                kind: "folder".into(),
                url: None,
                date_added: "0".into(),
                children: vec![Node {
                    name: "buried".into(),
                    kind: "url".into(),
                    url: Some("https://deep.test/".into()),
                    children: vec![],
                    date_added: "0".into(),
                }],
            }],
        };
        let mut out = Vec::new();
        collect(&deep, &mut out);
        assert_eq!(out.len(), 1, "a bookmark two folders down still comes over");
        assert_eq!(out[0].0, "https://deep.test/");
    }
}
