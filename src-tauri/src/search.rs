//! Turning what the user typed into somewhere to go.
//!
//! The address bar is one field doing two jobs - navigation and search - and the
//! guessing between them is the entire content of this module. Deliberately
//! separated from the browser plumbing so it can be reasoned about and tested on
//! its own, which matters because the heuristic is the sort of thing that
//! accumulates edge cases forever.

use serde::{Deserialize, Serialize};

/// A search engine, as a name and a URL template.
///
/// Kept as plain data rather than an enum so that adding an engine is a one-line
/// change to `ENGINES` with nothing else to update. `{query}` is the only
/// placeholder.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchEngine {
    /// Stable identifier used in settings. Never shown to the user.
    pub id: &'static str,
    pub name: &'static str,
    pub template: &'static str,
}

/// The engines offered out of the box.
///
/// DuckDuckGo is the default: privacy-respecting, no query or IP logging, and
/// reliable coverage because it blends its own crawler with partner results.
/// Mojeek and Brave are offered because both run genuinely independent indexes -
/// most "alternative" engines are Bing or Google with a different logo.
///
/// # About the DuckDuckGo query string
///
/// Out of the box DuckDuckGo results carry third-party ads and a rotating set of
/// house promotions for their own browser. Both are switchable off through
/// documented URL parameters, which is better than a content blocker here: the
/// page never renders the clutter at all, and the setting travels in the URL
/// rather than depending on a cookie that a privacy-focused browser is likely to
/// clear.
///
/// The two families do different jobs, verified rather than assumed:
///
/// | Parameter        | Effect                                              |
/// |------------------|-----------------------------------------------------|
/// | `k1=-1`          | Third-party ads off. Does **not** touch house promos |
/// | `kak kax kaq kap kao` | DuckDuckGo's own promo and newsletter messages, including the "Download Browser" panel |
/// | `kae=d`          | Dark theme, so results match Brume's chrome          |
///
/// `k1=-1` alone still leaves the browser advert on the page - that was measured,
/// not guessed, so please do not "simplify" this back to a single parameter.
pub const ENGINES: &[SearchEngine] = &[
    SearchEngine {
        id: "duckduckgo",
        name: "DuckDuckGo",
        template: "https://duckduckgo.com/?q={query}&k1=-1&kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1&kae=d",
    },
    SearchEngine {
        id: "duckduckgo-lite",
        name: "DuckDuckGo Lite",
        // DuckDuckGo's own no-JavaScript endpoint. Plain numbered results, no
        // ads, no promos, no instant answers, and a fraction of the payload.
        // Nothing to switch off because nothing is there in the first place.
        //
        // Caveat: it renders light, and no parameter changes that. The page
        // ships literally zero stylesheets, so there is nothing to theme and
        // nothing to respond to prefers-color-scheme either - it simply inherits
        // the engine's default black-on-white. Restyling it would mean injecting
        // a stylesheet into the content webview, which is a user-styles feature
        // rather than a search setting.
        template: "https://lite.duckduckgo.com/lite/?q={query}",
    },
    SearchEngine {
        id: "mojeek",
        name: "Mojeek",
        // Independent crawler, no ads, no house promotions, and roughly half the
        // page weight of a DuckDuckGo result page. `theme=dark` is honoured, so
        // unlike Lite this one matches Brume's chrome.
        template: "https://www.mojeek.com/search?q={query}&theme=dark",
    },
    SearchEngine {
        id: "brave",
        name: "Brave Search",
        template: "https://search.brave.com/search?q={query}",
    },
];

pub const DEFAULT_ENGINE_ID: &str = "duckduckgo";

pub fn engine_by_id(id: &str) -> &'static SearchEngine {
    ENGINES
        .iter()
        .find(|e| e.id == id)
        // An unknown id means a hand-edited settings file or an engine removed
        // in a later version. Falling back beats refusing to search.
        .unwrap_or(&ENGINES[0])
}

/// Percent-encodes a search term for use in a query string.
///
/// Hand-rolled rather than pulling in a crate: this is the only place Brume
/// needs it, and the rule is short. Everything outside the unreserved set from
/// RFC 3986 is escaped, and spaces become `+` as query strings expect.
fn encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Schemes Brume will hand to the webview as-is.
///
/// Deliberately a allowlist. `javascript:` is the reason - pasting a
/// `javascript:` URL into the address bar of a page you are logged into is a
/// classic self-XSS delivery mechanism, and every mainstream browser strips it
/// for that reason. Treating unknown schemes as a search is the safe default.
const ALLOWED_SCHEMES: &[&str] = &["http://", "https://", "file://", "about:"];

/// Does this look like an address rather than something to search for?
fn looks_like_url(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();

    if ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return true;
    }

    // A scheme we do not allow: not a URL as far as we are concerned, so it
    // falls through to search rather than being handed to the webview.
    if let Some(colon) = lower.find(':') {
        let scheme = &lower[..colon];
        if !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-')
            && lower[colon..].starts_with("//")
        {
            return false;
        }
    }

    // localhost, with or without a port.
    let host = lower.split('/').next().unwrap_or("");
    if host == "localhost" || host.starts_with("localhost:") {
        return true;
    }

    // Anything with whitespace is a search. This is what separates
    // "example.com" from "what is example.com".
    if input.chars().any(char::is_whitespace) {
        return false;
    }

    // A bare dotted name with a plausible TLD. Requiring at least two
    // characters after the final dot keeps "1.5" and "v2.0" as searches.
    match host.rsplit_once('.') {
        Some((before, tld)) => {
            !before.is_empty()
                && tld.len() >= 2
                && tld.chars().all(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// Resolves address bar input to a URL to navigate to.
pub fn resolve(input: &str, engine_id: &str) -> String {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return String::new();
    }

    if looks_like_url(trimmed) {
        let lower = trimmed.to_ascii_lowercase();
        if ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s)) {
            return trimmed.to_string();
        }
        // Scheme-less but clearly an address. HTTPS by default; a site that only
        // speaks HTTP will redirect, and defaulting the other way would silently
        // downgrade every navigation.
        return format!("https://{trimmed}");
    }

    engine_by_id(engine_id)
        .template
        .replace("{query}", &encode_query(trimmed))
}

#[tauri::command]
pub fn search_engines() -> Vec<SearchEngine> {
    ENGINES.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ddg(input: &str) -> String {
        resolve(input, DEFAULT_ENGINE_ID)
    }

    #[test]
    fn full_urls_pass_through_untouched() {
        assert_eq!(ddg("https://example.com/a?b=c"), "https://example.com/a?b=c");
        assert_eq!(ddg("http://example.com"), "http://example.com");
    }

    #[test]
    fn bare_domains_get_https() {
        assert_eq!(ddg("example.com"), "https://example.com");
        assert_eq!(ddg("news.bbc.co.uk/sport"), "https://news.bbc.co.uk/sport");
    }

    #[test]
    fn localhost_is_an_address_even_without_a_dot() {
        assert_eq!(ddg("localhost:3000"), "https://localhost:3000");
    }

    #[test]
    fn anything_with_spaces_is_a_search() {
        assert!(ddg("what is example.com").starts_with("https://duckduckgo.com/?q="));
        assert!(ddg("rust lifetimes").contains("rust+lifetimes"));
    }

    #[test]
    fn version_numbers_are_searches_not_domains() {
        // The TLD rule is what saves these: "5" and "0" are not alphabetic.
        assert!(ddg("1.5").contains("q=1.5"));
        assert!(ddg("v2.0").contains("q=v2.0"));
    }

    #[test]
    fn javascript_urls_are_never_navigated_to() {
        // Self-XSS via a pasted javascript: URL is the reason for the scheme
        // allowlist. It must come out as a search, not a navigation.
        let out = ddg("javascript://alert(document.cookie)");
        assert!(out.starts_with("https://duckduckgo.com/"), "got {out}");
    }

    #[test]
    fn query_encoding_escapes_reserved_characters() {
        let out = ddg("a&b=c d");
        assert!(out.contains("a%26b%3Dc+d"), "got {out}");
    }

    #[test]
    fn unknown_engine_falls_back_rather_than_failing() {
        assert!(resolve("test", "not-a-real-engine").starts_with("https://duckduckgo.com/"));
    }

    #[test]
    fn duckduckgo_default_suppresses_ads_and_house_promos() {
        let url = ddg("test");

        // Third-party ads.
        assert!(url.contains("k1=-1"), "ads not disabled: {url}");

        // DuckDuckGo's own promotions are a separate family. Measured: k1=-1 on
        // its own still renders the "Download Browser" panel, so dropping these
        // would quietly bring the advert back.
        for param in ["kak=-1", "kax=-1", "kaq=-1", "kap=-1", "kao=-1"] {
            assert!(url.contains(param), "missing {param}: {url}");
        }
    }

    #[test]
    fn lite_endpoint_needs_no_suppression() {
        let url = resolve("test", "duckduckgo-lite");
        assert!(
            url.starts_with("https://lite.duckduckgo.com/lite/?q="),
            "got {url}"
        );
        // Nothing to switch off, so nothing should have been bolted on.
        assert!(!url.contains("k1="), "lite should need no ad parameter: {url}");
    }

    #[test]
    fn every_engine_template_has_a_query_placeholder() {
        for engine in ENGINES {
            assert!(
                engine.template.contains("{query}"),
                "{} is missing {{query}}",
                engine.id
            );
        }
    }
}
