# Build notes

A running log of the architectural decisions behind Brume, and the reasoning for each one.
The point of this file is that future-me can re-read a decision instead of re-litigating it.

---

## Step 0: Repository setup

### Default branch is `main`

No strong reason beyond matching the GitHub default, which avoids a rename dance when the
remote is created.

### `Cargo.lock` is committed

Brume is a binary application rather than a library. Committing the lockfile pins the exact
dependency versions so a build today and a build in six months produce the same binary. For
libraries the convention is the opposite, which is why this is worth writing down.

### Signing keys are excluded aggressively

`.gitignore` blocks `*.key`, `*.pem`, `*.pfx`, `*.p12` and `.env*`. The updater's private key
is the single most damaging thing that could leak from this repo: anyone holding it can sign
a malicious payload that every existing Brume install will accept as a legitimate update and
run. The key is stored outside the repository entirely; the `.gitignore` entries are a
backstop, not the primary control.

The corresponding **public** key is safe to commit and is embedded in `tauri.conf.json`.
The `!*.key.pub` negation exists so that a `.key.pub` file is never accidentally swept up by
the broader `*.key` rule.

---

## Step 1: Environment

### Verified working set

These are the versions the project was first built against. Nothing here is a hard floor
except the Rust version, which Tauri 2 enforces.

| Component | Version |
|---|---|
| rustc / cargo | 1.97.1, `stable-x86_64-pc-windows-msvc` |
| Visual Studio 2022 Build Tools | 17.14.37516.0, Desktop development with C++ |
| Windows SDK | 10.0.26100.0 |
| Node / npm | 25.8.1 / 11.11.0 |
| Tauri CLI | 2.11.4 |
| WebView2 Runtime | 150.0.4078.105 |

### `cargo` may not be on PATH in an already-open terminal

Installing Rust writes `%USERPROFILE%\.cargo\bin` to the persistent user PATH, but processes
that were already running keep the environment block they started with. Any shell opened
before the install will report `cargo` as not found even though it is installed correctly.
Opening a new terminal fixes it; there is nothing to reinstall.

### Target toolchain: MSVC, not GNU

Rust on Windows offers `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu`. Brume uses MSVC:

- It is what Tauri and the `webview2-com` bindings are tested against.
- WebView2 is a COM API and the Microsoft-published bindings assume the MSVC ABI.
- NSIS bundling and code signing on Windows are both better trodden paths under MSVC.

The cost is a hard dependency on Visual Studio Build Tools for `link.exe` and the Windows SDK,
which is a multi-gigabyte install. That is a one-time developer cost and does not affect the
shipped binary or end users.

### WebView2 is a runtime dependency, not a bundled one

This is the central decision that makes Brume lightweight. Electron ships an entire Chromium
build per app, well over 100 MB before any application code. Brume renders through the
WebView2 runtime already present on the OS, so the installer carries only the Rust binary and
the frontend assets.

The tradeoff: WebView2 must exist on the target machine. It ships with Windows 11 and with
current Windows 10 installs, but is not guaranteed on older Windows 10 builds. Tauri's NSIS
bundler handles this with a **bootstrapper**, a small downloader invoked during installation
that pulls the runtime from Microsoft only when it is missing. This keeps the installer small
in the common case, at the cost of requiring an internet connection during install on the rare
machine that lacks the runtime. The alternative modes (`embedBootstrapper`, `offlineInstaller`)
trade installer size for offline capability and are noted here as a switch that can be flipped
later if offline installs ever matter.

### Rendering engine consequences

Because Brume renders with WebView2, page rendering behaviour matches Edge/Chromium of
whatever version is installed on the machine. Brume does not control the engine version, and
cannot ship engine patches independently of Windows Update. This is the accepted cost of not
bundling a browser engine.

---

## Brand assets: one source, one derived copy

The brand kit lives at `brand/` exactly as delivered, because it ships its own generators that
rewrite `brand/assets/` in place. Pointing the app at those files directly is not possible:
Tauri bundles a single directory (`frontendDist`, i.e. `src/`), so anything the running app
loads has to physically sit underneath it.

Rather than hand-copy files and let the two versions drift apart silently, the runtime subset
is derived by `tools/sync-brand-assets.ps1`, which wipes and recreates `src/assets/brand/`.
`brand/` is authoritative; `src/assets/brand/` is build output that happens to be committed.

Two details from the kit worth not rediscovering the hard way:

- **The small mark is a different drawing, not a downscale.** `mark-sm.svg` widens the cut from
  3 units to 4 so it survives antialiasing at 16 and 24px. The crossover to `mark.svg` is at
  32px, and both bundled `.ico` files already encode this internally.
- **The mark must never be rotated.** Its two halves are congruent under a 180-degree rotation,
  so a rotated Cleave is pixel-identical to an unrotated one. Rotating it communicates nothing
  and reads as a rendering bug.

Icons are drawn on a 24px grid with a 2px stroke and use `currentColor`, so they theme by
inheritance with no per-theme asset variants.

---

## A build trap worth remembering: the bundle-type marker

`tauri build` records which kind of package a binary was shipped in by **overwriting a
placeholder variable compiled into the executable**. The updater reads that marker later to
work out how to apply an update.

The placeholder is consumed the first time it is written. So bundling a binary that was already
bundled, which happens whenever only the NSIS template or `tauri.conf.json` changed and cargo
therefore saw no reason to relink, fails with:

```
Warn Failed to add bundler type to the binary: __TAURI_BUNDLE_TYPE variable not found in binary.
     Updater plugin may not be able to update this package.
```

**It is only a warning.** The build succeeds, the installer is produced, everything looks fine,
and the resulting package may simply refuse to update itself later. That combination of silent,
non-fatal, and only observable much later during an update is what makes it worth writing
down.

`tools/build-installer.ps1` runs `cargo clean -p brume --release` before bundling, which forces
a genuine relink and a fresh placeholder while leaving the dependency tree cached.

**Deleting `target/release/brume.exe` is not enough**, which is worth knowing because it is the
obvious thing to try and it silently does not work. Cargo links into
`target/release/deps/brume-<hash>.exe` and hardlinks that to `brume.exe`. The two names are one
file, so patching one patches both, and deleting the copy merely restores the already-patched
original. The giveaway is `Finished \`release\` profile in 1.12s`: no relink happened.

If you ever run `npm run tauri build` by hand and intend to ship the result, check the output
for that warning first.

---

## Browser chrome: two webviews, not one

Brume's window holds **two sibling webviews**: the chrome strip on top, the page beneath it.

The tempting alternative is one webview showing our HTML, with pages in an `<iframe>`. It does
not work. `X-Frame-Options` and CSP `frame-ancestors` let any site refuse to be embedded, and
most large ones do, including Google and DuckDuckGo. A browser that cannot open a search
engine is not a browser.

This requires Tauri's **`unstable` feature**, which is what gates multiple webviews per window.
That is a real dependency on an API Tauri may change in a minor release, so all of it is
confined to `browser.rs`, so a breaking change upstream has exactly one file to be repaired in.

Child webviews take no part in any layout system. They are rectangles positioned by hand, in
*logical* pixels, which is why `relayout` runs on every resize and scale-factor change: on a
150% display the chrome must still be 48 CSS pixels, or the toolbar and the space reserved for
it disagree.

### Tabs are one webview each, and the commands must be `async`

Every tab gets its own content webview. Reusing a single webview would mean
switching tabs reloads the page, loses scroll position and discards form input.
Inactive tabs are **hidden, not destroyed**, so a background tab keeps running
and still finishes loading, which is what every real browser does.

**Any command that creates, closes or reparents a webview must be declared
`async`.** This is not a style preference, and getting it wrong produces one of
the nastiest failure modes in the codebase.

Tauri runs a *synchronous* command on the main thread. `Window::add_child` is
implemented as dispatch-and-block:

```rust
self.run_on_main_thread(move || { ...build...; tx.send(res) })?;
rx.recv().unwrap()          // blocks the calling thread
```

Call that from the main thread and it queues work onto the very thread it then
blocks. The closure can never run. The result is not an error: **the entire app
deadlocks**, with the half-built webview stranded at `about:blank` and every
subsequent command timing out, including ones that touch no webview at all. There
is nothing in the logs.

Declaring the command `async` moves it onto the async runtime, so `add_child`
dispatches to a main thread that is still free to run it.

The obvious "fix", wrapping the work in `run_on_main_thread` yourself, makes it
*structurally identical* and deadlocks just the same. The
requirement is to be **off** the main thread, not on it.

### Brume owns the session history

`browser.rs` keeps its own `Vec<String>` of visited URLs plus an index, rather than deferring
to the webview's own history.

The reason is that **nothing exposes whether a webview can go back**. There is no DOM API for
it, and Tauri does not surface WebView2's `CanGoBack`. Without that, the back button would
either always be enabled or need guessing, and a back button that lies is worse than none.

The cost is real and worth stating: going back **re-navigates** rather than restoring from the
back-forward cache, so scroll position is lost and the page is refetched. A mainstream browser
does not behave this way. The obvious improvement is reaching into WebView2 through
`webview2-com` for the real `CanGoBack`/`GoBack`, at which point the local stack becomes
redundant.

A second known wrinkle: redirects create history entries, because `on_navigation` cannot
distinguish a redirect from a click. Going back from a redirected page can therefore land on
the URL that redirected you, and bounce forward again.

### The content webview has no permissions

`capabilities/default.json` is scoped to `webviews: ["chrome"]`. This is the single most
important line in that file. The content webview renders arbitrary websites, and any capability
granted to it is granted to every page the user visits. Tauri 2 denies by default, so the
protection is simply never listing it. But it would be easy to "fix" a permissions error by
widening the scope, and that would hand every site on the internet an IPC bridge.

### The address bar guesses, and one guess is a security decision

`search.rs` decides whether typed text is an address or a search. Most of it is heuristics with
unit tests. One rule is not a heuristic: **schemes are an allowlist**. `javascript:` URLs are
never navigated to, because pasting one into the address bar of a page you are logged into is a
classic self-XSS delivery mechanism, which is why every mainstream browser strips them. Unknown
schemes fall through to a search rather than being handed to the webview.

---

## Persistence: JSON, and specifically two different shapes

```text
%APPDATA%/com.londonchristensen.brume/
  settings.json     small, mutable        -> atomic whole-file rewrite
  bookmarks.json    small, mutable        -> atomic whole-file rewrite
  history.jsonl     large, append-only    -> one JSON object per line
```

**Not SQLite**, despite it being the obvious answer. `rusqlite` bundles the
SQLite C library, roughly 1 to 1.5 MB on a binary under 5 MB. A ~25% size increase
to store a list of URLs is hard to justify in a project whose entire premise is
not being large. `serde_json` was already present for settings, so this cost
nothing new.

**History is append-only because it is written on every page load.** Rewriting a
growing file that often is exactly the waste that makes people reach for a
database. Appending one line is O(1) and touches only the tail. JSONL also
degrades well: a torn write from a crash costs the last line rather than the
whole file, and the reader skips lines it cannot parse. There is a test for
precisely that.

Bookmarks are small, edited rarely, and need whole-list operations, so a plain
array is the better fit. Both writes go through a temp-file-and-rename, because
serialising directly over the destination leaves a truncated file if the process
dies mid-write, which for bookmarks means losing all of them.

Compaction to `HISTORY_CAP` runs **once at startup**, never during a navigation.
Trimming on write would turn the O(1) append back into an O(n) rewrite, which is
the entire thing this format exists to avoid.

### When to revisit

Every history query loads and parses the file. At the cap that is a few megabytes
and milliseconds, fine for a personal browser. Full-text search, or ranking by
visit frequency over a much larger corpus, is where SQLite starts earning its
size. All reads and writes are confined to `store.rs` so that swap stays
contained.

---

## The panel lives in the chrome webview

History, bookmarks and (next) settings render inside the **chrome** webview,
which grows to the full window height while open, hiding the content webviews.

The alternatives were both worse. A privileged internal page in a *tab* would sit
in the same list as arbitrary websites and need IPC permissions, putting a
trusted surface one bug away from an untrusted one. A separate window is heavier
and loses the browser's context.

Expanding the chrome keeps every privileged surface inside the one webview that
already holds capabilities, and costs one boolean plus a branch in `relayout`.

---

## Settings: homepage follows the search engine

`homepage` defaults to **empty**, not to a URL. Empty means "use the active
search engine's own landing page", so choosing Mojeek moves the new-tab page with
it rather than leaving every new tab on a DuckDuckGo page. Setting an explicit
homepage is a statement of intent and always wins.

That is why `SearchEngine` carries a `home` field alongside `template`, and why a
test asserts DuckDuckGo's homepage carries the same theme parameters as its
results template. A themed results page paired with an unthemed homepage would
be a visible seam every time a tab opens.

The engine is stored by **id**, never by URL template. Storing the template would
pin every user to whatever string was current the day they chose it, so a later
fix to a query string would never reach them.

### A CSS trap worth remembering

`[hidden]` is only a **user-agent default** of `display: none`. Any element given
an explicit display wins over it. `.btn { display: grid }` meant the clear-history
button stayed visible on the Settings tab while `element.hidden` reported `true`,
which is a confusing thing to debug from either side.

The fix is one rule that has to outrank the components:

```css
[hidden] { display: none !important; }
```

Worth having in any stylesheet that hides things by attribute.

---

## Keyboard shortcuts: why not a menu accelerator table

The obvious approach for Ctrl+T is a native menu with accelerators, hidden so no
menu bar shows. It is the platform's own mechanism and needs no dependency.

**It does not work in this stack.** muda builds an accelerator table and exposes
`haccel()`, but translating it requires someone to call `TranslateAcceleratorW`
in the message loop, and nothing does. Verified two ways: grepping tao, wry and
tauri-runtime-wry for a non-doc call site (none), and then attaching a menu,
focusing the window, pressing Ctrl+T and watching nothing happen.

A `keydown` listener in the chrome is not an alternative either. Keystrokes go to
whichever webview has focus, and while the user is reading a page that is the
*content* webview, which is deliberately outside every capability and cannot
call a command. The listener would be silent during most of the browser's use.

So shortcuts are **global shortcuts, registered on window focus and released on
blur**. "Global" is doing less work than it sounds: while Brume is focused no
other application receives keystrokes anyway, so the practical effect is an
application shortcut. The focus gating is what stops Brume holding Ctrl+T hostage
across the machine, and it is tested by minimising the window and confirming the
shortcut no longer fires.

If a future Tauri starts calling `TranslateAcceleratorW`, the menu approach
becomes viable and would drop a dependency.

---

## Theme: one switch, three surfaces

Choosing light has to move three things, and missing any one leaves a visible
seam:

1. **The chrome.** Every surface colour is a CSS variable, so the light theme is
   a value swap rather than a second stylesheet.
2. **The window frame.** Drawn by Windows, and only changes if the window is
   told. Otherwise a light UI keeps a dark title bar bolted to the top.
3. **The search results page.** `search.rs` carries `template_light` and
   `home_light`. Without them, choosing light left a dark DuckDuckGo page under a
   light toolbar, which looked broken.

`None` for those light variants means the engine needs nothing: Brave follows
`prefers-color-scheme` on its own, and DuckDuckGo Lite ships no stylesheets to
theme in either direction.

"System" is resolved by asking the **window** what the OS gave it, rather than
guessing from a media query on the Rust side.

---

## 0.3.0: two defaults that were not defaults

Two of the bugs fixed in 0.3.0 shared a shape worth recognising, because the next one will
look the same. Both were **library defaults that read as "platform behaviour" but were
actually "feature off"**, and neither produced an error, a warning, or a log line.

### New windows were being cancelled, not opened

`target="_blank"` and `window.open()` did nothing whatsoever. Not "opened in the wrong place".
Nothing.

wry registers a `NewWindowRequested` handler unconditionally. When no callback is supplied,
its else-branch is:

```rust
} else {
  args.SetHandled(true)?;
}
```

`SetHandled(true)` tells WebView2 the host has dealt with the request. Since nothing then
created a window, the request was simply dropped. The fix is `WebviewBuilder::on_new_window`,
returning `NewWindowResponse::Deny` and opening a Brume tab instead. `Deny` rather than
`Allow`, because allowing it produces a bare OS window with no chrome, no address bar and no
tab strip.

The handler runs on the main thread, so opening the tab **must** be spawned. Calling
`add_child` there deadlocks exactly as it does from a synchronous command.

### Zoom was switched off

`Ctrl` + scroll and `Ctrl` + `+`/`-` did nothing either. Both wry and tauri-runtime default
`zoom_hotkeys_enabled` to `false`, and Brume never set it, so what looked like an unset
option was WebView2's `IsZoomControlEnabled` being actively disabled. Setting it to `true` is
the entire fix; WebView2 handles both the wheel and the keyboard itself, so there is no
accelerator to register and nothing for Brume to keep in step.

**The lesson for next time:** when a browser affordance is missing and nothing is logging an
error, check the dependency's `Default` impl before looking at Brume's own code. Two of these
were sitting in the same file.

---

## Reload uses the runtime, not injected script

`reload` was `eval("location.reload()")`. That only works on a document that hosts script:
a PDF in the built-in viewer, a bare image, or one of WebView2's own error pages has no
`location` worth calling, so reload failed silently on exactly the pages a user is most likely
to want it on. It is now `Webview::reload()`.

`stop_loading` is still `eval("window.stop()")`, because neither Tauri nor wry surfaces
WebView2's `Stop()`. There is no `Webview::stop`, so reaching it means going through
`with_webview` to the `ICoreWebView2` directly, which is worth doing alongside the other
interop work rather than on its own.

---

## The navigation boundary has its own allowlist

`search.rs` refuses `javascript:` and other schemes for **address bar** input, which is a
self-XSS defence and documented there. That guard never covered page-initiated navigation,
because `on_navigation` returned `true` unconditionally.

It now refuses navigation to `tauri.localhost` and `asset.localhost`, Brume's own UI origin
and its asset protocol. The content webview holds no capabilities, so a page that got there
could not invoke a command today; the point is that the capability scoping should not be the
*only* thing standing between a website and Brume's chrome.

**The full address-bar allowlist was deliberately not applied here.** `href="javascript:void(0)"`
is ubiquitous, and a page that can navigate itself to a `javascript:` URL can already run the
same script directly, so blocking it buys no security and would break real sites. The hosts
are matched exactly rather than by suffix, because `ends_with("tauri.localhost")` would also
accept `nottauri.localhost`.

---

## Window geometry lives in settings.json

Brume opened at a fixed 1200x800, centred, every launch. The geometry now persists.

Three decisions worth recording:

1. **In `settings.json`, not its own file.** It is not a preference and the Settings panel does
   not show it, but keeping it here inherits that module's BOM-stripping and its refusal to
   overwrite a file it cannot parse. A second file would need both again.
2. **Logical pixels, not physical.** A window saved at 150% scaling would reopen at two thirds
   the size on a 100% display otherwise.
3. **Saved on close, not on move.** `Moved` and `Resized` fire continuously while a window is
   dragged, and each one would be a full rewrite of `settings.json`: hundreds of writes to
   record one final position. The cost is that a hard kill loses the last move.

Restoring a position is guarded by `position_is_on_screen`, which checks the saved top-left
against every available monitor. Without it, unplugging a second display leaves Brume running
somewhere the desktop no longer covers, with no way to reach the window. While maximised, the
previously saved size is kept and only the flag is updated, so un-maximising does not restore
to full screen forever.

---

## 0.3.0, second pass: five more

### settings.json was written non-atomically

`settings.rs` called `fs::write` straight over the destination while `store.rs`,
in the same crate, had a `write_atomic` helper and a comment explaining exactly
why that is wrong. The two had simply drifted. A crash mid-write truncates the
file and every setting goes at once.

It got worse when window geometry started being saved on close, because that
write happens precisely when the process is about to exit. Both modules now share
`store::write_atomic`.

### A failed webview left a tab that broke the toolbar

`open_tab_inner` registers the tab before creating its webview, which it has to:
the webview's handlers fire during construction and look the tab up by id. But on
failure the `?` returned with the tab still in the list, and still marked active.

Every later `navigate`, `reload` and `go_back` then resolved to a webview that
did not exist and failed. The chrome raises a dialog per failed command, so the
browser became unusable until that tab was closed. It now rolls the tab back out
and restores the previously active one.

`spawn_tab_webview` also returned `Ok(())` when the window was missing, producing
the same broken state while reporting success. That is an error now.

### Searching history re-read the whole file per keystroke

The panel's search box was wired straight to `input`. History is append-only
JSONL with no index, so every query reads the file and parses every line, up to
the 20,000-entry cap. Typing a six-letter word did that six times.

Debounced now, and each render takes a sequence number and drops its results if a
newer one has started. Without that, a slow earlier query could resolve last and
paint results for a query already typed past.

### The installer shell blocked its own event loop

Both of its commands were synchronous, and Tauri runs a synchronous command on
the main thread:

- `install` waits on the NSIS run, which takes seconds. The UI froze on the
  "Installing" screen, mid-animation, and Windows greyed the window out.
- `pick_folder` calls `blocking_pick_folder`, which the dialog plugin documents
  as "should *NOT* be used when running on the main thread". Its own example puts
  it in an `async fn`.

This is the `add_child` trap from the other direction. There the symptom is a
deadlock, here a frozen window, but it is one rule: do not block the main thread.
The browser learned it and the shell never had it applied. Both are `async` now.

### The release script could pair an installer with the wrong signature

`new-release.ps1` took the newest `*-setup.exe` and, separately, the newest
`*.sig`, and never checked the two belonged together. That directory is never
cleaned, so it holds every build ever made.

If signing failed for the current build, the newest `.exe` was the new version
while the newest `.sig` was still the previous release's, and they were paired
without complaint. The `if (-not $sig)` guard passes, because a `.sig` does
exist. The manifest would advertise the new version, link the new installer, and
carry the old signature, and every client would download it and reject it
silently. That is the "Clients see it but installation fails" row in
RELEASING.md, reached by a route the script created itself.

The artifact is now pinned to the version being released, the signature is
derived from the artifact rather than chosen, and a `.sig` older than the `.exe`
it signs is refused.

The publish step also checked no exit codes. `$ErrorActionPreference = 'Stop'`
does not apply to native commands, so a rejected `git push` fell through to
`gh release create`, which creates the missing tag itself from the default
branch. The release would then ship built from a different commit than the tag
names. Each step is checked now.

---

## 0.3.0, third pass: the installer shell could ship the wrong browser

### `rerun-if-changed` on a directory misses in-place rebuilds

`installer-shell/build.rs` embeds the NSIS installer with `include_bytes!` and
told cargo to watch the bundle **directory**. A directory's mtime moves when an
entry is added or removed, but not when an existing file is overwritten. Verified
on this filesystem rather than assumed.

So rebuilding without bumping the version overwrote `Brume_<ver>_x64-setup.exe`
in place, the directory looked untouched, `build.rs` did not re-run, and the
stale `payload.exe` already sitting in `OUT_DIR` was the one embedded. The
shipped `Brume-Setup.exe` then quietly installed the previous build, which is
maddening to debug because the browser binary itself is correct and only the
installer is behind.

`build.rs` now also emits `rerun-if-changed` for the payload file itself.

### The payload was chosen by timestamp, not by version

Same shape as the release script's signature bug, in a second place. The bundle
directory is never cleaned, so "newest wins" could reach across releases. The
payload is now pinned to `CARGO_PKG_VERSION`, which `new-release.ps1` keeps in
step with the browser, and a missing match lists what it did find instead.

---

## Two smaller ones worth recording

**A double-encoded string reached the UI.** `status.textContent` read
`"Checkingâ€¦"`: U+2026 had been through UTF-8, then CP1252, then
UTF-8 again, which is why the middle byte came back as a euro sign. Users saw
`Checkingâ€¦` every time they pressed "Check for updates". It is plain dots now.
This file has been through two encoding accidents already, so the one string a
user reads mid-action is not worth a non-ASCII character.

**Overlapping settings renders stacked two forms.** `renderSettings` empties the
list before three IPC round trips and appends after them, so two renders in
flight both cleared and then both appended. Clicking a theme or engine button
triggers a re-render, so two quick clicks did it. It now takes the same sequence
guard the history and bookmark paths use.

---

## 0.4.0: reaching WebView2 directly

Two modules now hold `unsafe` COM: `find.rs` and `history.rs`. Both exist for
the same reason, that Tauri surfaces only part of WebView2, and both confine
their interop so a breaking change upstream has a small number of places to be
repaired in. The route in either case is `Webview::with_webview`, which hands
back the raw `ICoreWebView2Controller`.

### The threading rule here is the opposite of browser.rs

browser.rs insists that webview-creating commands are `async`, because
`add_child` deadlocks a synchronous command. The find and history commands are
deliberately **synchronous**, and that is not an inconsistency.

`with_webview` posts through `send_user_message`, which checks whether it is
already on the main thread. On the main thread it runs the closure *inline* and
returns. Off it, it posts to the event loop and returns immediately without
waiting. A sync command already runs on the main thread, so the closure has
finished by the time the call returns and the result can be read straight back.
Nothing in either module creates a webview, so the `add_child` reasoning does
not apply.

The channel those modules pass results through waits with `recv_timeout` rather
than `try_recv`, so the same helper works when called from a background thread,
where the closure genuinely has not run yet.

### Two async reads that cost three test cycles

Both of these compiled, ran, and returned wrong answers. Neither was findable
without driving a real browser.

**`Find.Start` is asynchronous.** Reading `MatchCount` on the following line
returns 0 every time. On a page with two matches it reported none, which looks
exactly like a search that does not work. The count comes back through the
completion handler instead, and reaches the bar as an event because it does not
exist when the command returns.

**`FindNext` is asynchronous too, and the obvious fix does not work.**
`ActiveMatchIndexChanged` fires *before* the index property is updated, so
reading inside that handler is stale as well. The bar sat one press behind:
Next on "1 of 2" still read "1 of 2". What works is letting it settle briefly
and reading after, which is what `SETTLE` in find.rs is.

The pattern worth remembering: a WebView2 method that starts something does not
mean the property describing it has changed yet, and its change event is not a
promise that it has either.

### The chrome height is no longer constant

The find bar grows the chrome by exactly its own height and shrinks the page to
match, rather than floating over it. A bar drawn on top of the page can cover
the match it just scrolled to.

`CHROME_HEIGHT` is therefore a floor rather than the answer, and everything that
positions a content webview reads `chrome_extent()`. The 36px in the CSS and the
`FIND_BAR_HEIGHT` constant have the same contract the tab strip and toolbar
already had: disagree and the page is overlapped by the difference.

The bookmarks bar later became a second one of these, so `chrome_extent()` sums
its terms rather than branching on which bar is open. See below.

---

## Back and forward stopped re-navigating

browser.rs used to keep a `Vec<String>` per tab and go back by navigating to the
previous URL. That was the only option at the time, because nothing exposed
whether a webview could go back and a back button that is always enabled is
worse than none.

The cost was paid on every press: re-navigating refetches the page, loses scroll
position, discards form state and re-runs whatever the page does on load.
`ICoreWebView2` has had `GoBack` and `HistoryChanged` since its first release,
and they were reachable all along once find.rs proved the route.

`NavState` is now the current URL plus two flags mirrored from the runtime.
Nothing computes `can_go_back` here, because the runtime is the only thing that
knows.

**Why `HistoryChanged` rather than reading CanGoBack in `publish`:** publish runs
from inside WebView2's own navigation events, and calling back into the same
object from one of its handlers is the reentrancy its documentation warns about.
Letting the runtime say when history changed avoids the question.

Measured after the change: scroll to 3000, navigate away, back, still 3000.

**The JS heap is not preserved**, so bfcache proper appears to be off in
WebView2. Scroll and session history are restored either way, which is the part
a person notices, but do not promise form state.

---

## Session restore saves on change, not on quit

Tabs are persisted next to the window geometry in settings.json and rebuilt at
startup.

The first version saved only on `CloseRequested`, which is where the geometry is
saved and looked consistent. It was wrong: a crash or a kill lost the session,
and surviving an unexpected exit is the main thing session restore is for. It is
now written whenever the tab set changes. The write is small and atomic and
happens far less often than the history append that already runs on every page
load.

Only URLs are kept. Per-tab back history died with the stack when traversal moved
to WebView2, and the runtime's own history goes with the webview.

Closing every tab still leaves nothing to restore. That is deliberate, and it is
what every mainstream browser does: you closed them.

---

## Downloads are recorded, not intercepted

WebView2 performs the transfer and shows its own dialog either way. Brume simply
never heard about it, so there was nothing to list. `on_download` gives
`Requested` and `Finished` and nothing between, so there is no byte count and no
progress bar. Reaching one means going through
`ICoreWebView2DownloadOperation` directly, which is not done here.

The destination is deliberately left alone. The runtime already puts files where
Windows says downloads go, and overriding that to somewhere Brume invented would
be worse than doing nothing.

`reveal_download` checks the path against the recorded list before handing it to
Explorer. It arrives over IPC, and launching a process with an unchecked
caller-supplied string is worse than it looks.

---

## Private tabs are two separate jobs

`WebviewBuilder::incognito(true)` hands cookies, storage and cache to a
throwaway partition the runtime discards with the webview. That is half of it,
and the easy half.

The other half is everything **Brume** would otherwise write to disk, and each
one is a separate decision:

- no history entry on page load
- no entry in the saved session, or the file would name pages the tab existed to
  keep off disk
- no place in the reopen-closed-tab list, since that list outlives the tab

A link opened from a private tab inherits privacy. Anything else would leak a
private context into a recorded one on the first `target="_blank"`.

The tab strip shows the mask rather than a favicon, deliberately: fetching the
icon means a request to that site from a tab whose point is not making them.

## Clearing site data is a third interop module

`profile.rs` joins find.rs and history.rs. Cookies, local storage, IndexedDB and
the HTTP cache live in the WebView2 profile, reached through
`ICoreWebView2_13::Profile` and cleared with `ICoreWebView2Profile2::ClearBrowsingDataAll`.

`ClearBrowsingDataAll` rather than a time range, because a control labelled
"clear everything" that quietly kept last week would be worse than not offering
one. It is asynchronous and reports through a completion handler, and the
command waits for it: returning before the work is done invites a second press.

**History and bookmarks are deliberately not included.** Those are Brume's own
records with their own controls. "What the sites left behind" and "what I chose
to keep" are different decisions and should not share a button.

---

## Zoom: watched, not driven

WebView2 owns zooming. `zoom_hotkeys_enabled` lets it handle Ctrl+scroll and
Ctrl+plus/minus internally, so Brume never sets a level during normal use and
only needs to know what it became. `history.rs` subscribes to
`ZoomFactorChanged` on the **controller** (zoom is a property of the host's
presentation, not of the document) and mirrors the factor per tab, because
WebView2 keeps zoom per webview.

The indicator is hidden at 100%. A readout that is always on screen is noise,
and its presence is most of the signal: if you can see it, you are not at normal
size.

**ZoomFactorChanged does not fire for a programmatic `SetZoomFactor`.** Measured,
not assumed: setting 1.5 resized the page correctly, the watcher stayed silent
and the indicator kept reading 100%. So `set_zoom` reads the factor back and
updates state itself. The event still covers user zoom, which is the common path,
but it cannot be relied on for a set Brume made.

`NavState` implements `Default` by hand for one reason: `zoom` must start at
`1.0`. Deriving it gives `0.0`, and a new tab would report itself at 0%.

---

## Pinned tabs reorder rather than just flagging

A pinned tab in the middle of the strip is still a pinned tab you have to hunt
for, so pinning moves it to the end of the pinned run and unpinning moves it to
the front of the unpinned one. Both land at the same index, the boundary between
the two groups.

**A pinned tab refuses to close.** `close_tab_inner` returns early for one, and
"close others" and "close to the right" skip them. Ctrl+W is easy to hit by
accident, and pinning is the user saying this tab should survive it.

Pinned state is persisted, which is why the session field changed shape.

### Why the settings key was renamed rather than reused

`session` held bare URL strings; tabs now need a URL and a pinned flag. serde
rejects a field of the wrong type outright, and `SettingsState::load` moves an
unparseable file aside wholesale, so reusing the key would have cost the user
every other setting as well. Unknown keys are ignored, so `session_tabs` is a new
name and an old file simply starts with no session.

Worth remembering for the next schema change: `#[serde(default)]` on the
container rescues a *missing* field, not a mistyped one.

---

## The bookmarks bar is the second consumer of `chrome_extent()`

The find bar was the first thing to grow the chrome, and while it was the only
one, `chrome_extent()` could get away with a floor plus one conditional. Two rows
that can be open at the same time is where that shape stops working, so the
function sums its terms:

```rust
CHROME_HEIGHT
    + if find_open { FIND_BAR_HEIGHT } else { 0.0 }
    + if bookmarks_bar { BOOKMARKS_BAR_HEIGHT } else { 0.0 }
```

A third row later is one more term rather than a rewrite. The arithmetic is split
into `extent_for(find_open, bookmarks_bar)` so it can be tested without a running
app, and the test spells out all four heights as literals: 76, 108, 112, 144. It
deliberately does not recompute them from the constants, because a test that says
`CHROME_HEIGHT + FIND_BAR_HEIGHT` passes just as happily after someone changes a
constant and forgets index.html.

Measured against a running browser: 76 to 108 when the bar opens, with the page
going 724 to 692, and 144 with both bars up and the find bar sitting at y=108
directly under the bookmarks bar.

### Where the "is it showing" flag lives

In settings, not in `Browser`. It is a preference that has to survive a restart,
so it was going to be written there regardless, and a mirrored copy in the
process is the thing that eventually disagrees with the file after a failed
write. `chrome_extent()` reads it through a field accessor rather than
`SettingsState::get()`, which clones the whole struct including the session tab
list, and this is on the path taken every time a webview is positioned.

### Why the list is not in `BrowserState`

`BrowserState` is published on every navigation, title change and load-progress
tick. The bookmark list changes when someone presses Ctrl+D. Carrying every
bookmark in every one of those would re-serialise the list dozens of times per
page load to catch an event that happens by hand.

`brume://bookmarks` follows what `brume://downloads` already does: no payload,
only "this changed", and the two listeners fetch the list themselves. It is
emitted from all three places that mutate the list, including the two store
commands, so a removal from the panel reaches the bar without either knowing
about the other.

### Overflow is measured, not counted

How many bookmarks fit depends on the font, the titles and the window width,
so there is no count that works. Everything is rendered, then walked left to
right against the available width minus the overflow button, and what does not
fit is hidden and put in a menu.

Once one item overflows, so does everything after it, even if a later one would
have fit in the gap. Packing would reorder the bar on every resize, and a
bookmark bar is useful precisely because a thing stays where it was last seen.

The re-measure hangs off a `ResizeObserver` on the bar rather than a window
resize listener: this webview is positioned and sized by the Rust side, so what
changes is the element.

Checked at three widths: 7 items visible at 1200px, 4 at 700px, 10 at 1600px,
and back to exactly 7 on return. The overflow menu's contents matched the hidden
items exactly, in bar order.

### One context menu, two callers

The tab strip's menu already had the placement and clamping logic, and the
bookmarks bar needs the same thing, so `openMenu(at, build)` now holds it and
both callers pass only their own items. `at` needs nothing but `clientX` and
`clientY`, which lets the overflow button open a menu under itself with a plain
point instead of a pointer event.

The one trap: the overflow button opens on a left click, and the document-level
listener that dismisses menus sees the same click. It needs
`stopPropagation`, or the menu closes in the tick it opened. The tab menu never
hit this because it opens on `contextmenu`.

---

## The icon set is Lucide now, and that changed the licensing

The set used to be drawn by hand in `brand/tools/icons.py`, to a house rule of 90
degrees with 45 permitted only where an object carries direction and arcs only
where an object genuinely is round. It was internally consistent and it did not
look good enough. Squaring off things that are conventionally round reads as
deliberate on one icon and as a limitation across forty.

Lucide is the same 24 canvas, the same 2px stroke and the same `currentColor`, so
**no application code changed at all**. The chrome paints icons as CSS masks,
which care about the alpha channel and not about how it was drawn. The filenames
are unchanged too, because `build-icons.ps1` maps Brume's names onto Lucide's
rather than adopting Lucide's naming.

### UXWing was the first choice and could not be used

Its terms permit commercial use with no attribution, which covers using the icons
in the app. They also say redistribution of the files is prohibited. Brume's
repository is public and Apache-2.0, so committing them would both republish the
files and use Apache-2.0 to grant every person who clones the repo a
redistribution right UXWing withholds. Lucide is ISC, which permits
redistribution provided the notice travels along, so every generated file carries
it in a comment and NOTICE carries it at repository level.

**NOTICE said something that is no longer true and had to change.** It used to
carve the whole of `brand/` out of the Apache License and then grant the icon set
back under it. Neither half survives: the set is not Brume's to reserve, and it
is not Brume's to relicense. `cleave.svg` is the one file in that directory that
is still the mark and still reserved.

Some Lucide icons derive from Feather and are MIT rather than ISC. NOTICE lists
which of the bundled ones are.

### The generator had to stop being able to revert it

`icons.py` held the path data and `kit.py` wrote all 44 icons from it on every
run. Left alone, `python3 tools/kit.py` to regenerate the *mark* would have
silently overwritten the whole icon set with the old drawings.

`icons.py` now holds no geometry and reads the generated files back, and
`kit.py` writes only `cleave.svg`. That makes it impossible for the kit to
produce an icon that differs from the one being shipped, which is the property
that actually matters.

`brand/tools/audit.py` was deleted rather than repaired. Its entire purpose was
checking the hand-drawn geometry against the angle rule, and it parses only
absolute path commands, so against Lucide's relative commands and curves it would
have mis-parsed rather than honestly failed.

### What is still stale

`brand/BRAND-KIT.md` is generated by `docs.py` and still describes the angle rule
and inlines two of the old icons. `docs.py` itself has been corrected, so
`python3 tools/docs.py` fixes it in one run. That was not done here because
Python is not installed on this machine, which is also why `build-icons.ps1` is
PowerShell while the rest of `brand/tools/` is Python.

`brand/preview.html` **was** brought in line, by reproducing exactly what
`preview.py`'s `ic()` does, and checked by rendering it: 44 icons in the sheet,
43 round-capped, one butt-capped, and that one is `cleave`.

---

## Download progress is a fourth interop module

`on_download` reports a download starting and a download finishing, and nothing
between the two, so the panel could say "Downloading..." and never anything more.
The byte counts are on `ICoreWebView2DownloadOperation`, which needs the raw
interface, so downloads.rs joins find.rs, history.rs and profile.rs.

**It adds a second DownloadStarting handler rather than replacing wry's.** That
event takes any number of subscribers, and wry's is what `on_download` is built
on. Adding to it means the existing start and finish recording carries on
untouched and this module only supplies the part that was missing.

The cost is that nothing specifies which handler runs first, so the first
progress tick can arrive before the store has a row to put it in. That is not
theoretical: it is visible in the very first sample of every download, which
reads `received 0, total 0` and renders as "Starting...".
`update_download_progress` ignores a miss rather than inventing a row, and the
event fires often enough that the next one lands.

**Progress is throttled in Rust, not in the chrome.** BytesReceivedChanged fires
once per network read, and every event that reaches the chrome makes the panel
rebuild its whole list. 200ms, with the completing update always allowed through
so the bar lands on full rather than stopping wherever the throttle last let
something past.

`TotalBytesToReceive` is read on every tick rather than once at the start,
because a server that sent no Content-Length reports 0 and some only report it
once the response is properly under way. Zero means there is no fraction to
draw, and the bar falls back to the same indeterminate sweep the page-load
indicator uses.

Measured against a running browser: `total` came back as 10485760 for a file
whose Content-Length was independently fetched as 10485760, `received` climbed
monotonically, and the DOM went from an indeterminate "Starting..." bar to
`10.0 MB of 10.0 MB` at width 100%. A 58 MB download through a redirecting host
also matched, which is what confirms the URL key stays consistent between wry's
handler and this one across a redirect.

---

## The panel hid every page behind it, and nothing said so

Reported as: open Settings, press new tab, and the tab opens but the window
carries on showing Settings. Searching from the address bar did the same thing.

The cause is in the panel's own design. It lives in the chrome webview so that
every privileged surface stays in the one webview holding capabilities, and the
way it gets the space is that `relayout` grows the chrome to the full window and
**hides every content webview**. Nothing closed it again. So a new tab was
created, its webview built, its page loaded, the strip grew a tab and the address
bar updated, all correctly, and all behind an opaque panel.

Everything about it looked like it worked, which is why it survived this long:
`browser_state` reported two tabs and the right URL, and the only thing wrong was
that the window was still showing something else.

`dismiss_panel` now runs from everything that means "show me a page": opening a
tab, activating one, navigating, back, forward, reload and home. Bookmarking
deliberately does not, because it acts on the active tab without needing to show
it.

**Five of those commands had to become `async`.** They only ever talked to an
existing webview before, so they were sync; closing the panel means calling
`relayout`, which moves and resizes webviews, which is the thing a sync command
must not do from the main thread. `shortcuts.rs` dispatches them through `spawn`
now for the same reason.

The chrome takes `panel_open` from the published state rather than only from its
own click handlers, so the document cannot go on believing the panel is up after
the Rust side has already laid the page back out.

---

## Known hard problems, deliberately deferred

These are flagged early so they do not come as a surprise later. None are attempted in this
build; each is noted with where a future extension point would go.

### Chrome extension compatibility

WebView2 has limited extension support compared to full Chromium, and nothing resembling the
full Chrome Web Store API surface. Supporting real Chrome extensions would mean either
implementing a meaningful subset of the `chrome.*` APIs against WebView2's more limited
capabilities, or abandoning WebView2 for an embedded Chromium (CEF), which discards the entire
lightweight premise of this project. Not attempted.

### Deep sandboxing / process isolation

Real browsers run each site in its own OS process with a tightly restricted security token, so
that a renderer compromise does not become a machine compromise. Brume's tabs share the
WebView2 process model, which provides Chromium's own sandboxing but not per-tab isolation
that Brume itself controls. Building process-per-tab isolation on top of WebView2 is a
substantial project. Not attempted.

### Ad blocking

Content filtering needs request interception. WebView2 exposes `WebResourceRequested` for
this, which is the natural hook point. Deferred, but the request pipeline is worth keeping
clean so a filter can be inserted without restructuring.
