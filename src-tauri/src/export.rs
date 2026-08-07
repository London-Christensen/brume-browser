//! Writing bookmarks out, in the format every other browser still reads.
//!
//! # Why Netscape HTML
//!
//! It is thirty years old, it is ugly, and Chrome, Edge and Firefox all import
//! it without being asked twice. Anything tidier would be a format only Brume
//! understands, which defeats the point of an export.
//!
//! Writing it costs no dependency: it is string building over a list that is
//! already in memory. That matters here for the same reason `store.rs` refused
//! SQLite, and it is why this is hand-rolled rather than templated.
//!
//! # It doubles as the backup this file has never had
//!
//! `bookmarks.json` has no history and no undo. Until 0.6.0 it did not even keep
//! a copy when it failed to parse. An export is the one way to take a snapshot
//! somewhere else, which is worth more than the tidiness of the output.

use tauri::{AppHandle, State};

use crate::store::{Bookmark, Store};

/// Escapes text for HTML.
///
/// A bookmark title is whatever a page called itself, so it can hold anything,
/// including markup. `&` has to go first or it would double-escape the entities
/// the later replacements introduce.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Renders one level and everything under it.
///
/// Terminates because `repair_tree` has already broken any cycle: a parent chain
/// always reaches the root, so the recursion cannot revisit a folder.
fn render(items: &[Bookmark], parent: Option<u64>, depth: usize, out: &mut String) {
    let pad = "    ".repeat(depth);
    out.push_str(&format!("{pad}<DL><p>\n"));
    for b in items.iter().filter(|b| b.parent == parent) {
        if b.is_folder {
            out.push_str(&format!(
                "{pad}    <DT><H3 ADD_DATE=\"{}\">{}</H3>\n",
                b.added_at,
                escape(&b.title)
            ));
            render(items, Some(b.id), depth + 1, out);
        } else {
            out.push_str(&format!(
                "{pad}    <DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>\n",
                escape(&b.url),
                b.added_at,
                escape(&b.title)
            ));
        }
    }
    out.push_str(&format!("{pad}</DL><p>\n"));
}

/// The whole list as a Netscape bookmark file.
pub fn to_netscape(items: &[Bookmark]) -> String {
    let mut out = String::from(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n\
         <!-- This is an automatically generated file. -->\n\
         <META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n\
         <TITLE>Bookmarks</TITLE>\n\
         <H1>Bookmarks</H1>\n",
    );
    render(items, None, 0, &mut out);
    out
}

/// Saves every bookmark to a file the user picks. False if they cancelled.
///
/// `async` for the reason `installer-shell` documents at length: the blocking
/// dialog call must not run on the main thread, because it waits on messages
/// that the event loop it is blocking would have to pump.
#[tauri::command]
pub async fn export_bookmarks(app: AppHandle, store: State<'_, Store>) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;

    // Rendered before the dialog opens, so no lock is held while it is up.
    let html = to_netscape(&store.bookmarks());

    let Some(target) = app
        .dialog()
        .file()
        .set_file_name("brume-bookmarks.html")
        .add_filter("Web page", &["html"])
        .blocking_save_file()
    else {
        return Ok(false);
    };

    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, html).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: u64, parent: Option<u64>, url: &str, title: &str) -> Bookmark {
        Bookmark {
            id,
            url: url.into(),
            title: title.into(),
            added_at: 100,
            parent,
            is_folder: false,
        }
    }

    fn folder(id: u64, parent: Option<u64>, title: &str) -> Bookmark {
        Bookmark {
            id,
            url: String::new(),
            title: title.into(),
            added_at: 100,
            parent,
            is_folder: true,
        }
    }

    #[test]
    fn a_folder_nests_its_contents_rather_than_listing_them_alongside() {
        let items = vec![
            folder(1, None, "Reading"),
            link(2, Some(1), "https://inside.test/", "Inside"),
            link(3, None, "https://outside.test/", "Outside"),
        ];
        let html = to_netscape(&items);

        // The nested link must appear between the folder heading and the close
        // of the inner list, not after it.
        let heading = html.find("Reading").expect("folder heading");
        let inside = html.find("inside.test").expect("nested link");
        let outside = html.find("outside.test").expect("root link");
        assert!(heading < inside, "the link sits under its folder");
        assert!(inside < outside, "and before the root-level one");
        assert_eq!(html.matches("<DL><p>").count(), 2, "one list per level");
        assert_eq!(html.matches("</DL><p>").count(), 2);
    }

    #[test]
    fn a_title_cannot_smuggle_markup_into_the_file() {
        // Titles come from whatever a page called itself, so this is the one
        // place an export could produce a file that does something.
        let items = vec![link(
            1,
            None,
            "https://a.test/?a=1&b=2",
            "<script>alert(\"x\")</script> & more",
        )];
        let html = to_netscape(&items);
        assert!(!html.contains("<script>"), "markup is escaped");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&quot;x&quot;"));
        assert!(html.contains("a=1&amp;b=2"), "and so is the URL");
        assert!(!html.contains("&amp;amp;"), "but never twice");
    }

    #[test]
    fn an_empty_list_is_still_a_file_another_browser_will_accept() {
        let html = to_netscape(&[]);
        assert!(html.starts_with("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
        assert!(html.contains("<DL><p>") && html.contains("</DL><p>"));
    }
}
