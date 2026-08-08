//! Two rules applied per site: keep it on https, and whether it may run script.
//!
//! # Both are per origin, like everything else that is per site
//!
//! Permissions, remembered zoom and the blocking exception are all keyed on
//! scheme and host. These are too. A decision about a site is a decision about
//! the site, not about the one page that prompted it.
//!
//! # The upgrade happens at the navigation, not in the address bar
//!
//! `search::resolve` sees only what somebody typed. Most navigation is links,
//! redirects and scripts, so upgrading there would cover the smallest part of
//! the problem. `on_navigation` sees all of it.
//!
//! # What this does not do
//!
//! It does not detect that the upgraded page failed and offer to fall back.
//! WebView2 reports navigation failures through `NavigationCompleted`, which wry
//! does not surface, and adding a second handler races the one it owns - the
//! same wall popup blocking hit.
//!
//! So a site with no https at all shows the runtime's error page, and the way
//! out is the padlock: turn the upgrade off for that site and reload. That is a
//! real limitation, stated here rather than implied away.

use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings;

/// Scheme and host, with `www.` removed. The key every per-site decision uses.
///
/// One function, because there were five and they did not agree. Four kept the
/// host as it came; `permissions.rs` stripped `www.`, and so does the chrome's
/// own `originOf`. The result was that a control clicked in the window wrote
/// `https://theguardian.com` while the code reading it looked up
/// `https://www.theguardian.com`, and the setting silently did nothing.
///
/// That was found on the shield: turning blocking off for a site left it on,
/// and the count carried on rising. It only survived earlier testing because
/// `example.com` has no `www.`, so both spellings happened to agree.
///
/// Stripping is the right side to land on. `www.example.com` and `example.com`
/// are one site to the person deciding about them, and a decision that applied
/// to only one of the two spellings would look broken either way round.
pub fn origin_of(url: &str) -> String {
    let Ok(parsed) = tauri::Url::parse(url) else {
        return String::new();
    };
    let Some(host) = parsed.host_str() else {
        return String::new();
    };
    // The port belongs to the key when there is one.
    //
    // `host_str` drops it and the chrome's `URL.host` keeps it, which is the
    // same disagreement `www.` had: a decision made on `localhost:3000` would
    // have been stored under one spelling and looked up under another. Both
    // sides omit a default port, since `Url::port` returns `None` for those and
    // so does `URL.host`.
    let host = host.trim_start_matches("www.");
    match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    }
}

/// The https version of an http URL, if it should be upgraded.
///
/// Returns `None` when there is nothing to do, which is the common case and
/// therefore the cheap one: the setting is off, the URL is already secure, or
/// the site is excused.
///
/// Split from the navigation handler so the decision can be tested without a
/// running webview, the way `without_visit` and `read_bookmarks` are.
pub fn upgrade(url: &str, https_only: bool, excepted: bool) -> Option<String> {
    if !https_only || excepted {
        return None;
    }
    let mut parsed = tauri::Url::parse(url).ok()?;
    if parsed.scheme() != "http" {
        return None;
    }
    // localhost is exempt and always will be. It is not reachable by anyone
    // else on the network, it almost never has a certificate, and upgrading it
    // breaks local development for no gain in safety.
    if parsed
        .host_str()
        .is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]")
    {
        return None;
    }
    parsed.set_scheme("https").ok()?;
    Some(parsed.to_string())
}

/// Applies the script setting for the origin a tab is about to show.
///
/// WebView2's `IsScriptEnabled` belongs to the *webview*, not the document, so
/// it has to be set on every navigation. This is the same shape per-site zoom
/// needed and for the same reason: leaving it set would carry one site's rule
/// on to the next site the tab visits.
pub fn apply_script_rule(app: &AppHandle, label: &str, url: &str) {
    let blocked = app
        .state::<crate::settings::SettingsState>()
        .script_blocked(&origin_of(url));

    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let settings: ICoreWebView2Settings =
                platform.controller().CoreWebView2()?.Settings()?;
            settings.SetIsScriptEnabled(!blocked)?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_https_only(
    settings: tauri::State<'_, crate::settings::SettingsState>,
    enabled: bool,
) -> Result<(), String> {
    settings.set_https_only(enabled)
}

#[tauri::command]
pub fn set_https_exception(
    settings: tauri::State<'_, crate::settings::SettingsState>,
    origin: String,
    excepted: bool,
) -> Result<(), String> {
    settings.set_https_exception(&origin, excepted)
}

/// Turns script on or off for a site, and reloads so it takes effect.
///
/// Async because it reloads, which shows a page, which closes the panel.
#[tauri::command]
pub async fn set_script_blocked(
    app: AppHandle,
    window: tauri::Window,
    origin: String,
    blocked: bool,
) -> Result<(), String> {
    app.state::<crate::settings::SettingsState>()
        .set_script_blocked(&origin, blocked)?;
    // The setting is read on navigation, so the page in front of the user keeps
    // whatever it was built with until it loads again. Reloading here is what
    // makes the switch mean something immediately.
    crate::browser::reload(app, window).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_key_matches_what_the_chrome_writes() {
        // The chrome's originOf is `${protocol}//${host.replace(/^www\./,"")}`.
        // These have to agree exactly: a control clicked in the window writes
        // the key, and the code enforcing it does the lookup. When they
        // disagreed, turning blocking off for a www site silently did nothing
        // and the block count carried on rising.
        assert_eq!(
            origin_of("https://www.theguardian.com/international"),
            "https://theguardian.com"
        );
        // Without www, both spellings already agreed, which is why this went
        // unnoticed: example.com was the test site.
        assert_eq!(origin_of("https://example.com/a"), "https://example.com");
        // The scheme is part of the key, so http and https are separate
        // decisions. That matters for the https exception, which only means
        // anything on an http origin.
        assert_eq!(origin_of("http://example.com/"), "http://example.com");
        // Only a leading www., not any host that happens to contain it.
        assert_eq!(origin_of("https://wwwx.example/"), "https://wwwx.example");
        assert_eq!(origin_of("https://a.www.example/"), "https://a.www.example");
        // A port is part of the host and stays.
        assert_eq!(
            origin_of("http://localhost:3000/x"),
            "http://localhost:3000"
        );
        assert_eq!(origin_of("not a url"), "");
    }

    #[test]
    fn only_http_is_upgraded_and_only_when_asked() {
        // Off is the default, so this is the path nearly every navigation takes.
        assert_eq!(upgrade("http://example.com/a", false, false), None);
        // On, and the obvious case.
        assert_eq!(
            upgrade("http://example.com/a", true, false).as_deref(),
            Some("https://example.com/a")
        );
        // Already secure, so nothing to do rather than a pointless rewrite.
        assert_eq!(upgrade("https://example.com/a", true, false), None);
        // Excused, because the site genuinely has no https.
        assert_eq!(upgrade("http://old.example/", true, true), None);
    }

    #[test]
    fn local_addresses_are_never_upgraded() {
        // Not reachable by anyone else, almost never have a certificate, and
        // upgrading them breaks local development for no gain in safety.
        for url in [
            "http://localhost:3000/",
            "http://127.0.0.1:8080/x",
            "http://[::1]:9000/",
        ] {
            assert_eq!(
                upgrade(url, true, false),
                None,
                "{url} should be left alone"
            );
        }
    }

    #[test]
    fn the_path_and_query_survive_the_upgrade() {
        // An upgrade that dropped these would send people to a site's front
        // page instead of the thing they clicked, which reads as a broken link.
        assert_eq!(
            upgrade("http://example.com/a/b?c=d&e=f#g", true, false).as_deref(),
            Some("https://example.com/a/b?c=d&e=f#g")
        );
    }

    #[test]
    fn a_scheme_that_is_not_web_is_left_alone() {
        // file:// and about: reach the navigation guard too, and neither has an
        // https equivalent to be upgraded to.
        assert_eq!(upgrade("file:///c:/x.html", true, false), None);
        assert_eq!(upgrade("about:blank", true, false), None);
    }
}
