//! Content blocking: adverts, trackers, and the frames they leave behind.
//!
//! # The engine is Brave's, and that is the point
//!
//! `adblock`, from `brave/adblock-rust`. Full Adblock Plus syntax, the same
//! lists uBlock Origin and Brave consume, and a matcher built for the job.
//!
//! This project normally hand-writes rather than depends: the Netscape exporter,
//! the date formatter and the reader's extraction are all a few dozen lines with
//! an obvious correct answer. A filter matcher is not that. One that is
//! genuinely good is years of other people's work, and a worse one is a
//! substring search wearing a filter list as a hat.
//!
//! MPL-2.0, recorded in NOTICE.
//!
//! # Lists are fetched, never bundled
//!
//! Two reasons, and the second is the one that settles it.
//!
//! Lists change daily and Brume releases every few weeks, so anything bundled is
//! behind before it is installed.
//!
//! And EasyList, with everything derived from it, is CC BY-SA 3.0. Bundling one
//! would drag that licence and its share-alike terms into a repository that is
//! otherwise Apache-2.0. Fetching on the user's machine redistributes nothing:
//! what Brume ships is the engine, not the rules.
//!
//! # Parsed once, then cached as a blob
//!
//! Parsing several megabytes of filter text takes long enough to be felt at
//! launch, which would undo the parked-tab startup work of 0.6.0 in one feature.
//! `Engine::serialize` writes the built engine to disk and `deserialize` reads it
//! back, which is how Brave does it too.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
};
use webview2_com::{take_pwstr, WebResourceRequestedEventHandler};
use windows_core::Interface;

/// A list Brume knows how to fetch.
///
/// Held as a table rather than free-typed URLs so the default set is one place,
/// and so an id in settings survives a URL moving.
pub struct ListSource {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    /// What it is for, shown in Settings so the choice is not a guess.
    pub about: &'static str,
    /// Whether it is on before anyone changes anything.
    pub default_on: bool,
}

/// The lists offered out of the box.
///
/// EasyList and EasyPrivacy are the pair every blocker starts from: adverts and
/// trackers. The others are opt-in because they are matters of taste rather than
/// of privacy, and a blocker that hides things nobody asked it to hide is how
/// people end up turning the whole feature off.
pub const LISTS: &[ListSource] = &[
    ListSource {
        id: "easylist",
        name: "EasyList",
        url: "https://easylist.to/easylist/easylist.txt",
        about: "Adverts. The list nearly every blocker is built on.",
        default_on: true,
    },
    ListSource {
        id: "easyprivacy",
        name: "EasyPrivacy",
        url: "https://easylist.to/easylist/easyprivacy.txt",
        about: "Trackers and analytics, separate from adverts.",
        default_on: true,
    },
    ListSource {
        id: "annoyances",
        name: "Annoyances",
        url: "https://easylist.to/easylist/fanboy-annoyance.txt",
        about: "Cookie banners, newsletter overlays, social widgets.",
        default_on: false,
    },
    ListSource {
        id: "badware",
        name: "Badware and scams",
        url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/badware.txt",
        about: "Known malicious and deceptive hosts.",
        default_on: false,
    },
];

pub fn list_by_id(id: &str) -> Option<&'static ListSource> {
    LISTS.iter().find(|l| l.id == id)
}

/// The engine, and what it was built from.
///
/// `RwLock` rather than `Mutex`: matching is a read and happens on every
/// subresource of every page, while rebuilding happens when someone changes a
/// setting or a list is refreshed. Readers must not queue behind each other.
pub struct Blocker {
    engine: RwLock<Option<Engine>>,
    /// Where the cached blob and the raw lists live. Per profile, because which
    /// lists are on is a setting and settings are per profile.
    dir: PathBuf,
    /// Counts since launch, for the dashboard.
    stats: Mutex<Stats>,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Blocked since Brume started.
    pub session: u64,
    /// Blocked ever, carried across launches.
    pub total: u64,
    /// Per registrable domain of the page, this session only.
    #[serde(default)]
    pub by_site: std::collections::HashMap<String, u64>,
}

impl Blocker {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let stats = read_stats(&dir.join("blocking-stats.json"));
        Self {
            engine: RwLock::new(None),
            dir,
            stats: Mutex::new(stats),
        }
    }

    fn cache_path(&self) -> PathBuf {
        self.dir.join("blocker.bin")
    }

    fn list_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("list-{id}.txt"))
    }

    /// Whether an engine is loaded and able to answer.
    pub fn ready(&self) -> bool {
        self.engine.read().expect("engine lock poisoned").is_some()
    }

    /// Loads the cached engine, if one was built by an earlier run.
    ///
    /// Failure is not an error worth surfacing: a missing or unreadable cache
    /// means the engine gets rebuilt from the lists on disk, and a missing list
    /// means it gets fetched. Every path ends somewhere usable.
    pub fn load_cached(&self) -> bool {
        let Ok(bytes) = std::fs::read(self.cache_path()) else {
            return false;
        };
        let mut engine = Engine::default();
        if engine.deserialize(&bytes).is_err() {
            // A blob from an incompatible engine version. Dropped rather than
            // repaired: it is a cache, and the lists it came from are still
            // beside it.
            let _ = std::fs::remove_file(self.cache_path());
            return false;
        }
        *self.engine.write().expect("engine lock poisoned") = Some(engine);
        true
    }

    /// Rebuilds from whichever lists are enabled and on disk, then caches it.
    pub fn rebuild(&self, enabled: &[String]) -> Result<usize, String> {
        let mut set = FilterSet::new(false);
        let mut rules = 0usize;

        for id in enabled {
            let Ok(text) = std::fs::read_to_string(self.list_path(id)) else {
                continue; // not fetched yet; a later refresh will bring it
            };
            rules += text.lines().filter(|l| !l.trim().is_empty()).count();
            set.add_filter_list(text, ParseOptions::default());
        }

        let engine = Engine::new_with_filter_set(set);
        // Written before the lock is taken, so matching is never blocked on a
        // disk write.
        let blob = engine.serialize();
        *self.engine.write().expect("engine lock poisoned") = Some(engine);
        if let Err(e) = std::fs::write(self.cache_path(), blob) {
            // Not fatal. The engine is live; only the next launch pays for it.
            eprintln!("[blocker] could not cache the engine: {e}");
        }
        Ok(rules)
    }

    /// Stores a freshly downloaded list.
    pub fn store_list(&self, id: &str, text: &str) -> Result<(), String> {
        crate::store::write_atomic(&self.list_path(id), text)
    }

    /// Whether a request should be blocked.
    ///
    /// Called on the UI thread for every subresource of every page, so this is
    /// the hottest path in Brume. It takes a read lock, asks the engine, and
    /// does nothing else: no allocation beyond what the engine needs, no I/O,
    /// no logging.
    pub fn should_block(&self, url: &str, source: &str, request_type: &str) -> bool {
        let guard = self.engine.read().expect("engine lock poisoned");
        let Some(engine) = guard.as_ref() else {
            return false;
        };
        // `method` is always GET here. WebResourceRequested reports the real
        // method, but a filter that discriminates on it is vanishingly rare and
        // reading it costs a COM call on the hot path.
        let Ok(request) = Request::new(url, source, request_type, "GET") else {
            // An unparseable URL is not something to block on a guess.
            return false;
        };
        // `should_block` rather than checking `filter`, because a matched rule
        // is not the whole answer: an exception rule can have matched too, and
        // `@@` exceptions are how lists un-break sites they would otherwise
        // catch. Reading `filter.is_some()` would ignore every one of them.
        engine.check_network_request(&request).should_block()
    }

    /// Records a block, for the dashboard.
    ///
    /// Deliberately separate from `should_block`, which must stay free of locks
    /// it does not need. This one is only reached when something is actually
    /// blocked, which is a fraction of requests.
    pub fn record(&self, page_host: &str) {
        let due = {
            let mut stats = self.stats.lock().expect("stats lock poisoned");
            stats.session += 1;
            stats.total += 1;
            if !page_host.is_empty() {
                *stats.by_site.entry(page_host.to_string()).or_insert(0) += 1;
            }
            // Every so often rather than only at close.
            //
            // Persisting on CloseRequested alone loses the count to a crash or
            // a kill, which is the same mistake session saving already made and
            // documents. Measured: killing the process after 109 blocks brought
            // back a total of 0.
            //
            // 50 is chosen so a heavy page writes this once or twice, not per
            // request. The file is a few dozen bytes.
            stats.session % 50 == 0
        };
        if due {
            self.persist_stats();
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().expect("stats lock poisoned").clone()
    }

    /// Writes the running total out.
    ///
    /// Only the total survives a restart. The session count and the per-site
    /// breakdown are about what is happening now, and a per-site tally that
    /// accumulated forever would be a record of every site visited, which is
    /// precisely what a blocker exists to avoid keeping.
    pub fn persist_stats(&self) {
        let total = self.stats.lock().expect("stats lock poisoned").total;
        let slim = Stats {
            session: 0,
            total,
            by_site: std::collections::HashMap::new(),
        };
        if let Ok(json) = serde_json::to_string(&slim) {
            let _ = crate::store::write_atomic(&self.dir.join("blocking-stats.json"), &json);
        }
    }
}

/// Subscribes a tab's webview to request interception.
///
/// Called once per content webview, next to `history::watch`. The token is
/// dropped for the same reason the others are: the subscription lives as long as
/// the webview.
///
/// # This is the hot path, and the reason the engine was not hand-written
///
/// The handler runs **on the UI thread for every subresource**. A heavy page
/// makes hundreds, so anything done here is done hundreds of times before the
/// page is readable. It reads two strings from the runtime, asks the engine, and
/// for the overwhelming majority of requests returns without touching anything
/// else.
///
/// A blocked request is answered with a synthesised empty 403 rather than
/// cancelled: `WebResourceRequested` has no refuse, only a response. That is
/// what "blocked" means here, and it is what the page sees.
pub fn watch(app: &AppHandle, label: &str) {
    let Some(webview) = app.get_webview(label) else {
        return;
    };
    let handle = app.clone();

    let _ = webview.with_webview(move |platform| {
        let _ = (|| unsafe {
            let core = platform.controller().CoreWebView2()?;

            // Everything, because a tracker can arrive as any resource type.
            // Narrowing the filter would be an optimisation that decides in
            // advance which requests are allowed to be adverts.
            core.AddWebResourceRequestedFilter(
                &windows_core::HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            )?;

            let environment = core.cast::<ICoreWebView2_2>()?.Environment()?;

            let mut token = 0i64;
            core.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };

                    let blocker = handle.state::<Blocker>();
                    // Cheapest possible bails, in order of cost. Before the
                    // first list arrives there is nothing to ask, and every
                    // page on first run would otherwise pay for a lock and two
                    // string reads.
                    if !blocker.ready() {
                        return Ok(());
                    }
                    let settings = handle.state::<crate::settings::SettingsState>();
                    if !settings.blocking_enabled() {
                        return Ok(());
                    }

                    let request = args.Request()?;
                    let mut raw = windows_core::PWSTR::null();
                    request.Uri(&mut raw)?;
                    if raw.is_null() {
                        return Ok(());
                    }
                    let url = take_pwstr(raw);

                    let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
                    args.ResourceContext(&mut context)?;

                    // The page the request belongs to. Filters are mostly about
                    // this relationship: `$third-party` and every domain option
                    // are meaningless without it.
                    let source = current_source(&handle);

                    // The per-site escape hatch, checked before the engine
                    // rather than after: on an allowed site there is no point
                    // asking, and the answer would be thrown away.
                    if settings.blocking_allowed(&origin_of(&source)) {
                        return Ok(());
                    }

                    if !blocker.should_block(&url, &source, context_name(context)) {
                        return Ok(());
                    }

                    let response = environment.CreateWebResourceResponse(
                        None,
                        403,
                        &windows_core::HSTRING::from("Blocked by Brume"),
                        &windows_core::HSTRING::from(""),
                    )?;
                    args.SetResponse(&response)?;
                    blocker.record(&host_of(&source));
                    Ok(())
                })),
                &mut token,
            )?;
            Ok::<_, windows_core::Error>(())
        })();
    });
}

/// The URL of the page the request belongs to.
///
/// Read from Brume's own state rather than from the request's Referer header,
/// which is absent on plenty of requests and forgeable on the rest. The active
/// tab is the right answer in practice: a background tab is not making requests
/// a user is looking at, and being wrong costs a `$third-party` decision rather
/// than correctness.
fn current_source(app: &AppHandle) -> String {
    crate::browser::active_tab_url(app).unwrap_or_default()
}

fn host_of(url: &str) -> String {
    tauri::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

/// Scheme and host, matching how permissions and per-site zoom are keyed.
fn origin_of(url: &str) -> String {
    tauri::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| format!("{}://{}", u.scheme(), h)))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// What the Settings panel and the dashboard render.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockingView {
    pub enabled: bool,
    pub ready: bool,
    pub updated: i64,
    pub lists: Vec<ListView>,
    pub stats: Stats,
    /// Origins the user has excused, so the list can be managed in one place.
    pub allowed: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListView {
    pub id: String,
    pub name: String,
    pub about: String,
    pub on: bool,
}

#[tauri::command]
pub fn blocking_state(
    app: AppHandle,
    settings: tauri::State<'_, crate::settings::SettingsState>,
) -> BlockingView {
    let on = settings.blocking_lists();
    BlockingView {
        enabled: settings.blocking_enabled(),
        ready: app.state::<Blocker>().ready(),
        updated: settings.blocking_updated(),
        lists: LISTS
            .iter()
            .map(|l| ListView {
                id: l.id.to_string(),
                name: l.name.to_string(),
                about: l.about.to_string(),
                on: on.iter().any(|id| id == l.id),
            })
            .collect(),
        stats: app.state::<Blocker>().stats(),
        allowed: settings.get().blocking_allowed,
    }
}

#[tauri::command]
pub fn set_blocking_enabled(
    settings: tauri::State<'_, crate::settings::SettingsState>,
    enabled: bool,
) -> Result<(), String> {
    settings.set_blocking_enabled(enabled)
}

/// Turns a list on or off and rebuilds, without re-downloading.
///
/// Async because the rebuild parses every enabled list, which is seconds of work
/// on a large set and must not run on the main thread.
#[tauri::command]
pub async fn set_blocking_list(app: AppHandle, id: String, on: bool) -> Result<(), String> {
    let settings = app.state::<crate::settings::SettingsState>();
    let mut ids = settings.blocking_lists();
    ids.retain(|x| x != &id);
    if on {
        ids.push(id.clone());
    }
    settings.set_blocking_lists(ids.clone())?;

    // A list switched on for the first time has never been fetched, so this
    // does the download too. Switching one off only rebuilds.
    if on
        && !app
            .state::<Blocker>()
            .dir
            .join(format!("list-{id}.txt"))
            .exists()
    {
        refresh(app.clone(), ids).await?;
        return Ok(());
    }
    app.state::<Blocker>().rebuild(&ids)?;
    Ok(())
}

/// Excuses an origin from blocking, or stops excusing it.
#[tauri::command]
pub fn set_blocking_allowed(
    settings: tauri::State<'_, crate::settings::SettingsState>,
    origin: String,
    allowed: bool,
) -> Result<(), String> {
    settings.set_blocking_allowed(&origin, allowed)
}

/// Fetches every enabled list and rebuilds.
#[tauri::command]
pub async fn refresh_blocking(app: AppHandle) -> Result<usize, String> {
    let ids = app
        .state::<crate::settings::SettingsState>()
        .blocking_lists();
    let rules = refresh(app.clone(), ids).await?;
    let _ = app
        .state::<crate::settings::SettingsState>()
        .set_blocking_updated(crate::store::now_unix());
    Ok(rules)
}

/// The runtime's resource context as the string the engine expects.
///
/// These names are Adblock Plus's `$type` options, which is what filter lists
/// are written against. Getting one wrong does not error, it silently stops
/// `$script` or `$image` rules matching, so the mapping is spelled out rather
/// than derived.
fn context_name(context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> &'static str {
    match context {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => "document",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "stylesheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST => "xmlhttprequest",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => "fetch",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => "websocket",
        _ => "other",
    }
}

/// Downloads the enabled lists and rebuilds the engine.
///
/// `async` and spawned by the caller: this is a network round trip per list, and
/// a browser must never wait on one to open.
///
/// Uses reqwest, which is already in the tree under `tauri-plugin-updater`.
/// Naming it in Cargo.toml makes an existing dependency honest rather than
/// adding one, the same move `webview2-com` and `windows` already made.
///
/// A list that fails to download is skipped, not fatal. The engine is rebuilt
/// from whatever did arrive plus whatever was already cached on disk, so one
/// unreachable publisher does not leave the user with no blocking at all.
pub async fn refresh(app: AppHandle, enabled: Vec<String>) -> Result<usize, String> {
    let client = reqwest::Client::builder()
        // A filter list is a few megabytes of text from a public endpoint. If
        // it has not arrived in thirty seconds something is wrong, and hanging
        // the refresh forever helps nobody.
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("Brume/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let mut fetched = 0usize;
    for id in &enabled {
        let Some(source) = list_by_id(id) else {
            continue;
        };
        match client.get(source.url).send().await {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(text) if !text.trim().is_empty() => {
                    if let Err(e) = app.state::<Blocker>().store_list(id, &text) {
                        eprintln!("[blocker] could not save {id}: {e}");
                    } else {
                        fetched += 1;
                    }
                }
                Ok(_) => eprintln!("[blocker] {id} came back empty"),
                Err(e) => eprintln!("[blocker] could not read {id}: {e}"),
            },
            Ok(response) => eprintln!("[blocker] {id} returned {}", response.status()),
            Err(e) => eprintln!("[blocker] could not fetch {id}: {e}"),
        }
    }

    // Rebuilt regardless of how many arrived, so a run where everything failed
    // still ends with the engine reflecting what is on disk.
    let rules = app.state::<Blocker>().rebuild(&enabled)?;
    let _ = fetched;
    Ok(rules)
}

fn read_stats(path: &std::path::Path) -> Stats {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<Stats>(raw.strip_prefix('\u{feff}').unwrap_or(&raw)).ok()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_lists_are_the_two_everyone_starts_from() {
        // Adverts and trackers on; taste-based lists off. A blocker that hides
        // things nobody asked it to hide is how people end up switching the
        // whole feature off rather than one list.
        let on: Vec<&str> = LISTS
            .iter()
            .filter(|l| l.default_on)
            .map(|l| l.id)
            .collect();
        assert_eq!(on, vec!["easylist", "easyprivacy"]);
    }

    #[test]
    fn every_list_has_a_distinct_id_and_an_https_url() {
        let mut seen = std::collections::HashSet::new();
        for l in LISTS {
            assert!(seen.insert(l.id), "{} is listed twice", l.id);
            // Fetched over the network on the user's machine, so plaintext
            // would let anyone on the path decide what Brume does not block.
            assert!(
                l.url.starts_with("https://"),
                "{} must be fetched over https",
                l.id
            );
            assert!(!l.about.is_empty(), "{} needs a description", l.id);
        }
    }

    #[test]
    fn an_engine_with_no_lists_blocks_nothing() {
        // The state before the first fetch finishes. It has to be permissive:
        // refusing requests because the lists have not arrived would break every
        // page on first run.
        let dir = std::env::temp_dir().join("brume-blocker-empty");
        let b = Blocker::new(dir);
        assert!(!b.ready());
        assert!(!b.should_block(
            "https://ads.example/banner.gif",
            "https://news.example/",
            "image"
        ));
    }

    #[test]
    fn a_built_engine_blocks_what_the_rule_names_and_leaves_the_rest() {
        let dir = std::env::temp_dir().join("brume-blocker-rules");
        let _ = std::fs::create_dir_all(&dir);
        let b = Blocker::new(dir);
        b.store_list("easylist", "||ads.example^\n").unwrap();
        b.rebuild(&["easylist".to_string()]).unwrap();

        assert!(b.ready());
        assert!(
            b.should_block(
                "https://ads.example/banner.gif",
                "https://news.example/",
                "image"
            ),
            "the named host should be blocked"
        );
        assert!(
            !b.should_block(
                "https://news.example/logo.png",
                "https://news.example/",
                "image"
            ),
            "an unrelated host should not be"
        );
    }
}
