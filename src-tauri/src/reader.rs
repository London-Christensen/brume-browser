//! Reader mode: an article, without everything around it.
//!
//! # Why this injects into the page
//!
//! The obvious design is to extract the article, hand the text to Rust, and
//! render it in a Brume-served page. It cannot be built, and the reason is a
//! property of this codebase worth protecting rather than working around.
//!
//! **The content webview holds no capabilities.** `capabilities/default.json` is
//! scoped to `chrome-*` and a tab is `tab-N`, so a page cannot invoke a command.
//! That is the whole thing standing between every website on the internet and
//! Brume's IPC bridge, and a reader is not a reason to widen it.
//!
//! A Brume-served reader page cannot fetch the article either: it would be on
//! `tauri.localhost` and the article is not, so CORS refuses. Fetching in Rust
//! means an HTTP client dependency, in a project whose premise is not being
//! large, to re-request a document the browser is already holding.
//!
//! So the reader replaces the document in place. Nothing has to travel back, and
//! the capability boundary is never approached.
//!
//! # This is the opposite of what find.rs decided, deliberately
//!
//! `find.rs` refused to inject, because a search has no business altering the
//! document it is searching and injected highlights fight the page's own CSS.
//!
//! Reader mode's entire purpose is to alter presentation. It is invoked
//! deliberately, and turning it off reloads the page, which puts the original
//! back exactly as the server sent it. Mutating is the feature here rather than
//! a side effect of one.
//!
//! # Extraction is hand written
//!
//! Readability.js is Apache-2.0 and would be compatible, but it is roughly 100KB
//! of JavaScript against a 5MB binary whose entire argument is size. What is here
//! is a scoring heuristic over text density, which gets most articles.
//!
//! It is a heuristic and it is stated as one. `reader_failed` is emitted when
//! nothing scores well enough, so a page it cannot read says so instead of
//! showing a blank frame.

use tauri::{AppHandle, Manager};

/// Told to the chrome when a page turns out not to be an article.
pub const READER_STATE_EVENT: &str = "brume://reader-state";

/// What the script sets the title to when it finds no article.
///
/// The title is the only channel a capability-less page has back to Brume: the
/// runtime reports `DocumentTitleChanged` whether or not the page can call
/// anything.
///
/// **No leading or trailing whitespace, and that is not a style choice.** The
/// first version was `" brume-reader-failed"`, with a space to make an accidental
/// collision impossible. `document.title`'s getter strips and collapses ASCII
/// whitespace per the HTML spec, so the space was gone before Brume ever saw the
/// string and the sentinel could never match. Measured on 2026-08-07: the page
/// set a leading space and the runtime reported the title starting at `b`.
///
/// Uniqueness comes from the shape of the string instead. A page whose real
/// title is this has bigger problems than a reader that declines to run.
pub const FAILED_SENTINEL: &str = "brume::reader::no-article";

/// The extraction and the layout, as one script.
///
/// Kept as a single injected string rather than a file the page loads, because a
/// page load would be a second request Brume would have to serve to an origin
/// that must not be able to reach anything of Brume's.
///
/// Returns nothing. See the module docs: nothing can come back from here.
fn reader_script(dark: bool) -> String {
    // Brume's own tokens, passed in rather than read from a stylesheet the page
    // cannot see. Kept to the few that matter, so this does not become a second
    // copy of the palette to keep in step.
    let (bg, fg, muted, accent) = if dark {
        ("#101418", "#e6e9ec", "#93a1ad", "#7cc4ff")
    } else {
        ("#fbfcfd", "#16191c", "#5d6b78", "#0b6bcb")
    };

    format!(
        r##"
(function () {{
  if (window.__brumeReader) return;

  /* Score every candidate by how much of it is text rather than markup.
   *
   * Paragraph count and text length, minus a penalty for links, which is what
   * separates an article from a navigation column with the same word count. */
  function score(node) {{
    var text = (node.innerText || "").trim();
    if (text.length < 250) return 0;
    var paras = node.querySelectorAll("p").length;
    if (paras < 2) return 0;
    var linkText = 0;
    node.querySelectorAll("a").forEach(function (a) {{
      linkText += (a.innerText || "").length;
    }});
    var linkRatio = linkText / Math.max(1, text.length);
    if (linkRatio > 0.5) return 0;
    return text.length * paras * (1 - linkRatio);
  }}

  var best = null;
  var bestScore = 0;
  /* article and main first: a page that says which part is the article is
   * telling the truth more often than a heuristic guesses it. */
  var candidates = document.querySelectorAll("article, main, [role=main], div, section");
  for (var i = 0; i < candidates.length; i++) {{
    var s = score(candidates[i]);
    if (s > bestScore) {{ bestScore = s; best = candidates[i]; }}
  }}

  if (!best) {{
    /* Says so rather than showing an empty frame. The chrome hears about it
     * through the title, which is the one channel a capability-less page has.
     *
     * Put back immediately afterwards. The title is the signal, not a place to
     * leave a message: left set, the tab strip would show the sentinel as the
     * page's name and the user would be looking at Brume's plumbing. The
     * timeout is what lets the runtime raise the change event before the
     * original goes back. */
    var prev = document.title;
    document.title = "{failed}";
    setTimeout(function () {{ document.title = prev; }}, 60);
    return;
  }}

  var heading = document.querySelector("h1");
  var title = (heading && heading.innerText.trim()) || document.title;
  var body = best.cloneNode(true);
  /* Everything that is not prose. Kept as a list rather than an allowlist so an
   * unusual but harmless element survives. */
  body.querySelectorAll(
    "script, style, iframe, form, button, input, nav, aside, footer, header, video, audio, svg, noscript"
  ).forEach(function (n) {{ n.remove(); }});

  var doc =
    '<meta name="viewport" content="width=device-width, initial-scale=1">' +
    '<style>' +
    'html,body{{margin:0;padding:0;background:{bg};color:{fg};}}' +
    'body{{font:17px/1.7 Georgia,"Times New Roman",serif;}}' +
    '.r{{max-width:42rem;margin:0 auto;padding:56px 24px 96px;}}' +
    '.r h1{{font:600 30px/1.25 system-ui,sans-serif;margin:0 0 6px;}}' +
    '.r .src{{color:{muted};font:13px system-ui,sans-serif;margin:0 0 32px;}}' +
    '.r p{{margin:0 0 1.15em;}}' +
    '.r a{{color:{accent};}}' +
    '.r img{{max-width:100%;height:auto;}}' +
    '.r h2,.r h3{{font-family:system-ui,sans-serif;line-height:1.3;margin:1.8em 0 .5em;}}' +
    '.r blockquote{{margin:1.2em 0;padding-left:1em;border-left:2px solid {muted};color:{muted};}}' +
    '.r pre{{overflow-x:auto;padding:12px;background:rgba(127,127,127,.12);border-radius:6px;font-size:14px;}}' +
    '</style>' +
    '<div class="r"><h1></h1><p class="src"></p><div class="c"></div></div>';

  document.documentElement.innerHTML = "<head></head><body>" + doc + "</body>";
  /* textContent, never innerHTML, for the two strings that came from the page.
   * The body below is markup the page already rendered, but a title assigned as
   * HTML would be a way to smuggle script back in through the cleaner. */
  document.querySelector(".r h1").textContent = title;
  document.querySelector(".r .src").textContent = location.host;
  document.querySelector(".r .c").appendChild(body);
  document.title = title;
  window.__brumeReader = true;
}})();
"##,
        bg = bg,
        fg = fg,
        muted = muted,
        accent = accent,
        failed = FAILED_SENTINEL,
    )
}

/// Turns the reader on for a tab. Off is a reload, done by the caller.
///
/// Fire and forget: `eval` cannot return a value, and the module docs explain
/// why nothing is allowed to come back anyway. It also fails silently on a
/// document that hosts no script - a PDF, a bare image, one of WebView2's error
/// pages - which is the documented `eval` trap and is the right outcome here,
/// since none of those is an article.
pub fn apply(app: &AppHandle, label: &str, dark: bool) -> Result<(), String> {
    let webview = app
        .get_webview(label)
        .ok_or_else(|| "That tab is not open.".to_string())?;
    webview
        .eval(reader_script(dark))
        .map_err(|e| format!("Could not reach the page: {e}"))
}
