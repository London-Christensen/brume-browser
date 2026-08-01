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
    /// Where a new tab lands when no explicit homepage is set.
    ///
    /// Per-engine rather than one global constant, so choosing Mojeek does not
    /// leave every new tab sitting on a DuckDuckGo page. Carries the same
    /// suppression and theming parameters as `template`, since the landing page
    /// should be as free of house advertising as the results page.
    pub home: &'static str,

    /// Light-theme variants, where the engine exposes theming at all.
    ///
    /// `None` means the engine has nothing to switch - either it follows
    /// `prefers-color-scheme` on its own (Brave), or it ships no stylesheets to
    /// theme (DuckDuckGo Lite). Without these, choosing a light browser theme
    /// would leave a dark results page bolted underneath a light toolbar.
    pub template_light: Option<&'static str>,
    pub home_light: Option<&'static str>,
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
///
/// `k1=-1` alone still leaves the browser advert on the page - that was measured,
/// not guessed, so please do not "simplify" this back to a single parameter.
///
/// # Custom theming
///
/// `kae=-1` selects a *custom* theme rather than one of the presets, which
/// unlocks explicit colours. Brume uses this to paint results in its own palette
/// instead of settling for a generic dark mode:
///
/// | Parameter | Colours                    | Brume token        |
/// |-----------|----------------------------|--------------------|
/// | `k7`      | Page background            | Ink `#101418`      |
/// | `kj`      | Header background          | Ink `#101418`      |
/// | `k21`     | Card / module background   | Surface `#171D24`  |
/// | `k9`      | Result titles              | Paper `#F3F4F5`    |
/// | `k8`      | Result body text           | Haar `#9DB2C0`     |
/// | `kaa`     | Result URLs                | Haar `#9DB2C0`     |
/// | `kx`      | Visited result titles      | Dimmed Haar        |
///
/// Titles get Paper and body text gets Haar, which is the same primary/secondary
/// split the chrome uses. Lamplight is deliberately absent: the brand allows it
/// once per screen, and a results page has too many links for that to hold.
///
/// This only works on the full site. `lite.duckduckgo.com` ships no stylesheets
/// at all, so it has nothing to colour - see that entry below.
pub const ENGINES: &[SearchEngine] = &[
    SearchEngine {
        id: "duckduckgo",
        name: "DuckDuckGo",
        // Painted in Brume's own palette, not merely "dark". See the
        // custom-theme table above for what each parameter colours.
        //
        // The hex values are the brand tokens, and a test asserts they still
        // match brand/assets/css/tokens.css - so retheming Brume and forgetting
        // the search page is a build failure rather than a visual surprise.
        template: "https://duckduckgo.com/?q={query}\
                   &kae=-1&k7=101418&k8=9db2c0&k9=f3f4f5&kx=6e7f8c&kaa=9db2c0&k21=171d24&kj=101418\
                   &k1=-1&kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1",
        // Same parameters as the template. A test asserts they stay in step, so
        // a themed results page cannot end up paired with an unthemed homepage.
        home: "https://duckduckgo.com/\
               ?kae=-1&k7=101418&k8=9db2c0&k9=f3f4f5&kx=6e7f8c&kaa=9db2c0&k21=171d24&kj=101418\
               &k1=-1&kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1",
        // Light: Paper page, Ink titles, Haar body. The same roles as the dark
        // set, with the two neutrals swapped.
        template_light: Some(
            "https://duckduckgo.com/?q={query}\
             &kae=-1&k7=f3f4f5&k8=4a5c6b&k9=101418&kx=6e7f8c&kaa=4a5c6b&k21=ffffff&kj=f3f4f5\
             &k1=-1&kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1",
        ),
        home_light: Some(
            "https://duckduckgo.com/\
             ?kae=-1&k7=f3f4f5&k8=4a5c6b&k9=101418&kx=6e7f8c&kaa=4a5c6b&k21=ffffff&kj=f3f4f5\
             &k1=-1&kak=-1&kax=-1&kaq=-1&kap=-1&kao=-1",
        ),
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
        home: "https://lite.duckduckgo.com/lite/",
        // Nothing to theme: the page ships zero stylesheets, so it renders in
        // the engine's default black-on-white whatever Brume is set to.
        template_light: None,
        home_light: None,
    },
    SearchEngine {
        id: "mojeek",
        name: "Mojeek",
        // Independent crawler, no ads, no house promotions, and roughly half the
        // page weight of a DuckDuckGo result page. `theme=dark` is honoured, so
        // unlike Lite this one matches Brume's chrome.
        template: "https://www.mojeek.com/search?q={query}&theme=dark",
        home: "https://www.mojeek.com/?theme=dark",
        template_light: Some("https://www.mojeek.com/search?q={query}&theme=light"),
        home_light: Some("https://www.mojeek.com/?theme=light"),
    },
    SearchEngine {
        id: "brave",
        name: "Brave Search",
        // No theme parameter needed: Brave follows prefers-color-scheme, and the
        // window is created with a dark theme, so it renders dark already.
        template: "https://search.brave.com/search?q={query}",
        home: "https://search.brave.com/",
        // Brave follows prefers-color-scheme, which the webview reports from the
        // window theme, so it changes with Brume without being told.
        template_light: None,
        home_light: None,
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

impl SearchEngine {
    /// The search template for the current theme.
    pub fn template_for(&self, dark: bool) -> &'static str {
        if dark {
            self.template
        } else {
            self.template_light.unwrap_or(self.template)
        }
    }

    /// The landing page for the current theme.
    pub fn home_for(&self, dark: bool) -> &'static str {
        if dark {
            self.home
        } else {
            self.home_light.unwrap_or(self.home)
        }
    }
}

/// Resolves address bar input to a URL to navigate to.
pub fn resolve(input: &str, engine_id: &str, dark: bool) -> String {
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
        .template_for(dark)
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
        resolve(input, DEFAULT_ENGINE_ID, true)
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
        assert!(resolve("test", "not-a-real-engine", true).starts_with("https://duckduckgo.com/"));
    }

    /// The brand kit itself, read at compile time.
    ///
    /// This is what makes the theme test meaningful rather than a restatement of
    /// the same hex codes in two places.
    const TOKENS_CSS: &str = include_str!("../../brand/assets/css/tokens.css");

    /// Pulls a `--brume-*` hex value out of tokens.css, lowercased and without
    /// the leading `#`, ready to compare against a URL parameter.
    fn token_hex(name: &str) -> String {
        TOKENS_CSS
            .lines()
            .find_map(|line| {
                let rest = line.trim().strip_prefix(name)?;
                // Guard against `--brume-haar` matching `--brume-haar-dark`:
                // the very next character has to be the colon.
                let rest = rest.strip_prefix(':')?;
                // Values are often followed by a comment, so stop at the
                // semicolon before doing anything else.
                let value = rest.split(';').next()?.trim();
                value.strip_prefix('#').map(str::to_ascii_lowercase)
            })
            .unwrap_or_else(|| panic!("{name} not found in tokens.css"))
    }

    #[test]
    fn duckduckgo_theme_tracks_the_brand_tokens() {
        let url = ddg("test");

        // Parameter -> the token it is supposed to be painted with.
        let bindings = [
            ("k7", "--brume-ink"),     // page background
            ("kj", "--brume-ink"),     // header background
            ("k9", "--brume-paper"),   // result titles
            ("k8", "--brume-haar-dark"), // result body text
            ("kaa", "--brume-haar-dark"), // result URLs
        ];

        for (param, token) in bindings {
            let expected = token_hex(token);
            assert!(
                url.to_ascii_lowercase().contains(&format!("{param}={expected}")),
                "search theme has drifted from the brand: expected {param}={expected} \
                 (from {token} in tokens.css)\nURL: {url}"
            );
        }

        // Custom theme mode, without which none of the colours above apply.
        assert!(url.contains("kae=-1"), "custom theme not selected: {url}");
    }

    #[test]
    fn lamplight_is_not_used_for_search_results() {
        // The brand allows the accent once per screen at most. A results page
        // has far too many links for that to survive contact with reality.
        let lamplight = token_hex("--brume-lamplight");
        let url = ddg("test").to_ascii_lowercase();
        assert!(
            !url.contains(&lamplight),
            "Lamplight should not colour search results: {url}"
        );
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
        let url = resolve("test", "duckduckgo-lite", true);
        assert!(
            url.starts_with("https://lite.duckduckgo.com/lite/?q="),
            "got {url}"
        );
        // Nothing to switch off, so nothing should have been bolted on.
        assert!(!url.contains("k1="), "lite should need no ad parameter: {url}");
    }

    #[test]
    fn duckduckgo_homepage_carries_the_same_theme_as_results() {
        let home = engine_by_id("duckduckgo").home;
        // A themed results page paired with an unthemed homepage would be an
        // obvious visual seam every time a tab opens.
        for param in [
            "kae=-1", "k7=101418", "k8=9db2c0", "k9=f3f4f5", "kj=101418", "k1=-1", "kak=-1",
        ] {
            assert!(home.contains(param), "homepage missing {param}: {home}");
        }
    }

    #[test]
    fn light_theme_swaps_the_search_palette() {
        let light = resolve("test", "duckduckgo", false);
        // Paper page, Ink titles - the dark values must not survive.
        assert!(light.contains("k7=f3f4f5"), "page not Paper: {light}");
        assert!(light.contains("k9=101418"), "titles not Ink: {light}");
        assert!(!light.contains("k7=101418"), "dark page colour leaked: {light}");

        // Ad and promo suppression is not a theme concern and must persist.
        for param in ["k1=-1", "kak=-1", "kax=-1"] {
            assert!(light.contains(param), "light variant lost {param}: {light}");
        }
    }

    #[test]
    fn engines_without_a_light_variant_fall_back_rather_than_break() {
        // Brave follows prefers-color-scheme; Lite has no stylesheets at all.
        for id in ["brave", "duckduckgo-lite"] {
            let engine = engine_by_id(id);
            assert_eq!(
                engine.template_for(false),
                engine.template,
                "{id} should reuse its only template in light mode"
            );
        }
    }

    #[test]
    fn every_engine_has_a_usable_homepage() {
        for engine in ENGINES {
            assert!(
                engine.home.starts_with("https://"),
                "{} has no https homepage",
                engine.id
            );
            assert!(
                !engine.home.contains("{query}"),
                "{} homepage still has a query placeholder",
                engine.id
            );
        }
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
