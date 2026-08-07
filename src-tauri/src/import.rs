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
//! # Folders come across, into a dated folder of their own
//!
//! Until 0.6.0 Brume's bookmarks were a flat list, so a tree could not survive
//! the trip and this deliberately flattened rather than dropping anything. The
//! folder model removed that limit, and the structure now comes over intact.
//!
//! Every import lands under one folder named for the source and the day, so a
//! second import cannot collide with the first and the whole thing is one
//! deletion to undo. That is also why nothing is skipped as a duplicate any
//! more: the tree is reproduced as it stands, and a URL the source keeps in two
//! folders stays in both. Nothing existing is ever touched or removed, which was
//! the guarantee that actually mattered.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

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

/// One entry on its way in, with the source's structure still attached.
///
/// An intermediate rather than building `store::Bookmark` directly, because ids
/// and parents belong to the store and are only known once it starts inserting.
#[derive(Debug, PartialEq)]
pub enum Imported {
    Link {
        url: String,
        title: String,
        added_at: i64,
    },
    Folder {
        title: String,
        children: Vec<Imported>,
    },
}

/// Turns a node tree into `Imported`, keeping the folders.
///
/// Empty folders are dropped. A source often carries several, and reproducing
/// them means an import whose result is mostly things to tidy up.
fn collect(node: &Node) -> Option<Imported> {
    if node.kind == "url" {
        let url = node.url.as_ref()?;
        // Only real web addresses. A Chromium file can carry javascript:
        // bookmarklets, and importing one would put a script behind a
        // one-click button on the bookmarks bar.
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return None;
        }
        return Some(Imported::Link {
            url: url.clone(),
            title: node.name.clone(),
            added_at: chrome_time_to_unix(&node.date_added),
        });
    }

    let children: Vec<Imported> = node.children.iter().filter_map(collect).collect();
    if children.is_empty() {
        return None;
    }
    Some(Imported::Folder {
        title: node.name.clone(),
        children,
    })
}

/// How many links are in a tree, for the count on the import button.
fn count_links(items: &[Imported]) -> usize {
    items
        .iter()
        .map(|i| match i {
            Imported::Link { .. } => 1,
            Imported::Folder { children, .. } => count_links(children),
        })
        .sum()
}

fn read_source(path: &PathBuf) -> Option<Vec<Imported>> {
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
        // The roots themselves are unwrapped: "Bookmarks bar" as a folder inside
        // a dated folder is one level of nesting nobody asked for. Their
        // children keep whatever structure they had.
        match collect(root) {
            Some(Imported::Folder { children, .. }) => out.extend(children),
            Some(link) => out.push(link),
            None => {}
        }
    }
    Some(out)
}

/// Names of the months, for the folder an import lands in.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Formats a Unix timestamp as "6 August 2026".
///
/// Hand-rolled from the civil-from-days algorithm rather than taking a date
/// crate, for the same reason `store.rs` refused SQLite: this is one label on
/// one folder, and it is not worth a dependency in a 5 MB binary.
fn format_day(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400) + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
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
                count: count_links(&found),
            })
        })
        .collect()
}

/// Inserts a tree under `parent`, depth first. Returns how many links landed.
///
/// Parents are created before their children so every parent id named here is
/// one this call just made, which is what lets it use the store's unvalidated
/// `add_folder` and `add_bookmark`.
fn insert_tree(store: &crate::store::Store, items: &[Imported], parent: Option<u64>) -> usize {
    let mut added = 0usize;
    for item in items {
        match item {
            Imported::Link {
                url,
                title,
                added_at,
            } => {
                if store.add_bookmark(url, title, *added_at, parent) {
                    added += 1;
                }
            }
            Imported::Folder { title, children } => {
                let id = store.add_folder(title, parent);
                added += insert_tree(store, children, Some(id));
            }
        }
    }
    added
}

/// Copies one browser's bookmarks in. Returns how many links were added.
///
/// Everything lands under one folder named for the source and the day. A second
/// import therefore sits beside the first rather than merging into it, and is
/// undone by deleting one folder.
#[tauri::command]
pub fn import_bookmarks(
    app: AppHandle,
    store: State<'_, crate::store::Store>,
    source: String,
) -> Result<usize, String> {
    let (_, name, path) = candidates()
        .into_iter()
        .find(|(id, _, _)| *id == source)
        .ok_or_else(|| format!("Unknown source: {source}"))?;

    let found = read_source(&path)
        .ok_or("Could not read that browser's bookmarks. Is it still installed?")?;
    if found.is_empty() {
        return Ok(0);
    }

    let root = store.add_folder(
        &format!("{name}, {}", format_day(crate::store::now_unix())),
        None,
    );
    let added = insert_tree(&store, &found, Some(root));

    // Written once at the end rather than per bookmark. Importing a thousand
    // would otherwise rewrite the file a thousand times.
    store.flush_bookmarks()?;

    crate::browser::notify_bookmarks_everywhere(&app);
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
        let Some(Imported::Folder { children, .. }) = collect(&node) else {
            panic!("a folder with usable children should come across");
        };
        assert_eq!(children.len(), 2, "only http and https should survive");
        assert!(children.iter().all(|c| matches!(
            c,
            Imported::Link { url, .. } if url.starts_with("http")
        )));
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
        // Inverted in 0.6.0. This used to assert the tree was flattened, which
        // was the honest thing to do while the store held a flat list.
        let Some(Imported::Folder { title, children }) = collect(&deep) else {
            panic!("the outer folder should come across");
        };
        assert_eq!(title, "root");
        assert_eq!(children.len(), 1);
        let Imported::Folder { title, children } = &children[0] else {
            panic!("the inner folder should still be a folder, not flattened");
        };
        assert_eq!(title, "inner");
        assert_eq!(
            children[0],
            Imported::Link {
                url: "https://deep.test/".into(),
                title: "buried".into(),
                added_at: 0,
            },
            "a bookmark two folders down comes over, still two folders down"
        );
    }

    #[test]
    fn an_empty_folder_is_not_worth_importing() {
        // Sources carry plenty of these, and reproducing them means an import
        // whose result is mostly things to tidy up.
        let empty = Node {
            name: "nothing here".into(),
            kind: "folder".into(),
            url: None,
            date_added: "0".into(),
            children: vec![Node {
                name: "also empty".into(),
                kind: "folder".into(),
                url: None,
                date_added: "0".into(),
                children: vec![],
            }],
        };
        assert!(collect(&empty).is_none());
    }

    #[test]
    fn the_import_folder_is_named_for_the_day_it_happened() {
        // Checked against dates worked out independently rather than against
        // the same arithmetic run twice.
        assert_eq!(format_day(0), "1 January 1970");
        // 20671 days after the epoch, counted out by hand.
        assert_eq!(format_day(1_785_974_400), "6 August 2026");
        // A leap day, which is where a hand-rolled calendar goes wrong.
        assert_eq!(format_day(1_709_164_800), "29 February 2024");
        // One second before midnight UTC must still be the same day.
        assert_eq!(format_day(1_786_060_799), "6 August 2026");
        // And the next second must not be.
        assert_eq!(format_day(1_786_060_800), "7 August 2026");
    }

    #[test]
    fn counting_links_ignores_the_folders_around_them() {
        let tree = vec![
            Imported::Link {
                url: "https://a.test/".into(),
                title: "a".into(),
                added_at: 0,
            },
            Imported::Folder {
                title: "f".into(),
                children: vec![
                    Imported::Link {
                        url: "https://b.test/".into(),
                        title: "b".into(),
                        added_at: 0,
                    },
                    Imported::Folder {
                        title: "g".into(),
                        children: vec![Imported::Link {
                            url: "https://c.test/".into(),
                            title: "c".into(),
                            added_at: 0,
                        }],
                    },
                ],
            },
        ];
        assert_eq!(count_links(&tree), 3);
    }
}
