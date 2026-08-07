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

## 0.5.0: the chrome cannot cover the page, and that shapes everything

The address bar had no suggestions at all, and the obvious way to add them ran
straight into an architectural fact nobody had written down.

**The content webview sits ABOVE the chrome in Win32 z-order.** Measured on
2026-08-05 by walking `GetTopWindow` and `GW_HWNDNEXT` over the window's
children, with the page deliberately left visible under a full-window chrome:

```text
  0. WRY_WEBVIEW  1920x981    <- the content webview, TOPMOST
  1. WRY_WEBVIEW  1920x1057   <- the chrome, underneath
```

So the chrome cannot cover a page by being made bigger. **That is why the panel
hides every content webview** rather than drawing over them: hiding is the only
thing that works, and it was never a stylistic choice.

A dropdown cannot use that trick. Making the page vanish on every keystroke is
worse than having no suggestions, and growing the chrome the way the find bar
does reflows the page down and back on every character typed.

`overlay.rs` therefore raises the chrome's hosting HWND above the page for as
long as an overlay is open and drops it back afterwards, with one `SetWindowPos`
call. The HWND comes from `ICoreWebView2Controller::ParentWindow`, which is how
wry reaches for it too. `SWP_NOACTIVATE` is load-bearing: without it, raising the
chrome steals focus from the address bar the user is typing into.

The page is never moved and never hidden. `suggest_height` is deliberately **not**
part of `chrome_extent`, because every other bar takes space from the page and
this one does not.

Measured after the change: chrome 76 to 188 with the dropdown open, page viewport
unchanged at 981, chrome topmost while open and the page topmost again after.

### Ranking is recency with prefix promotion, not frecency

Bookmarks before history, prefix match before substring, newest first within
each, deduplicated by URL. The scheme is stripped before matching, because nobody
types `https://` to find github.

Frecency was considered and rejected for now. History is append-only JSONL with
no index, so counting visits means either a full scan per keystroke or real
storage, and this file already records that as the point where SQLite earns its
size. Recency answers most of the same question for none of the cost.

---

## The tab strip lost tabs, and the new tab button with them

Tabs are `flex: 1 1 0` with a 44px floor, inside a strip with `overflow: hidden`.
Past the floor they simply stopped being reachable.

Measured at 1920px with 50 tabs, before the fix: eight tabs clipped off the right
edge, and the new tab button sitting at x=2259 against a strip ending at 1920. On
a 1200px window that begins at about 26 tabs, which is ordinary use. Ctrl+Tab and
Ctrl+9 could still reach the lost tabs; nothing else could, including closing
them.

The strip is now two parts: a scrolling run of tabs, and the new tab button
outside it. The button is reachable at any tab count by construction rather than
by luck. The active tab is scrolled into view after every render, or switching
with Ctrl+Tab past the visible run would leave the strip disagreeing with which
page is showing. Scroll position is preserved across renders, because state is
republished on every load tick of every tab and the strip would otherwise jump
back to the left constantly.

---

## The loading indicator had never once rendered

`.progress` was `flex: 0 0 2px` in the body's flex column, after the toolbar. The
chrome webview is exactly tab strip plus toolbar tall, so the row began at the
first pixel below the viewport and `body { overflow: hidden }` clipped all of it.
Measured 2026-08-04: `top: 76` in a 76px viewport.

It is absolutely positioned now, pinned to the bottom edge of the toolbar at
`top: calc(tab-strip + toolbar - 2px)`. Pinned to the toolbar rather than to the
bottom of the chrome because the chrome's height moves: the find bar, the
bookmarks bar and the address dropdown all change it, and an indicator that
wandered with them would be worse than one that never appeared.

Costs the page nothing, which is the reason it is an overlay rather than another
2px term in `chrome_extent`.

Verified: `top: 74, height: 2` in a 76px viewport, fully visible, with the sweep
animation running on a 614px fill, and `loading` confirmed reaching the chrome as
`true` over `brume://state` during a real navigation.

---

## Finishing things that were already half-built

Three small ones, grouped because each was a case of the hard part already
existing and only the last step being missing.

**Find already knew about case sensitivity.** `find_start` took a
`case_sensitive` argument and set it on the options object, and the chrome passed
`false` unconditionally. WebView2 also offers `SetShouldMatchWord`, so both are
exposed now as `Aa` and `ab` in the find bar. Whole word is the runtime's own
word boundary rather than something Brume defines, which matters for every
language that does not put spaces between words.

Flipping a toggle re-runs the current search immediately rather than waiting for
the next keystroke, because a modifier that only takes effect once you type
again reads as broken.

Verified against a running page: "domain" found 2 matches, 1 with match case on.
"omain" found 2 as a substring and **No results** with whole word on, which is
the assertion that actually proves the flag reaches the runtime.

**Ctrl+J and Ctrl+Shift+Delete were never bound.** Both are standard everywhere
else. Ctrl+Shift+Delete opens Settings rather than a dedicated dialog: the clear
control already lives there, and a second surface for the same switch is a second
thing to keep in step. The panel event now carries any of the four view names
rather than only history and settings, falling back to history for anything
unrecognised instead of rendering an empty panel.

**History could only be cleared entirely.** Removing one visit rewrites the file,
which is exactly what an append-only format exists to avoid, and that is fine:
it runs when someone deletes a row by hand, not on every page load, so recording
a visit stays O(1) and the delete pays O(n) once.

Two properties there are easy to get wrong and both are tested: the match is on
URL **and** timestamp, so removing one visit to a page does not take every other
visit with it, and a line that will not parse is kept rather than dropped,
because a torn line is not the line anyone asked to remove and discarding it
would turn a delete into a repair.

`match_rank` and `without_visit` are free functions rather than closures for the
same reason `extent_for` is: the ranking and the line handling are the decisions
worth testing, and neither needs a Store or a filesystem to exercise. Writing the
ranking test immediately caught a wrong expectation of my own, which is the
argument for having written it.

---

## Site permissions, and an API that remembers when you ask it not to

Brume never handled `PermissionRequested`, so every decision about a camera, a
microphone or a location was WebView2's default: its own prompt, in its own
styling, with no record Brume could show and no way to change your mind
afterwards. For a browser that leads with privacy, having no opinion about the
camera was a strange place to have none.

### The runtime does the remembering, deliberately

`ICoreWebView2Profile4::SetPermissionState` persists a decision against an
origin, and once set the runtime stops raising the event for it. So Brume stores
nothing of its own: no permissions.json, and `GetNonDefaultPermissionSettings`
reads back exactly what the engine is enforcing.

That is worth more than it looks. A list Brume kept itself could say "blocked"
while the engine allowed it, and nothing on screen would tell you which one was
lying.

### `remember` works backwards

**Setting `State` to ALLOW or DENY on the event args already persists it.**
Measured rather than assumed: answering with `remember: false` and then reading
`GetNonDefaultPermissionSettings` came back with the decision sitting in the
profile. The documentation advertises `SetPermissionState` as "a persistent
version of the State property", which reads as though the event-args version is
not persistent. It is.

So remembering an answer takes no work, and *forgetting* one takes an extra call:
after the deferral completes, the setting is put back to DEFAULT. That is what
makes a dismissed prompt mean "no, this time" instead of "no, forever".

DEFAULT is deliberately not used to answer the request itself, because it means
"do whatever the browser would have done", and what WebView2 would have done is
show its own prompt.

### The deferral never leaves the main thread

A prompt is answered by a human, so the event is deferred rather than answered
inline. `ICoreWebView2Deferral` and the event args are COM objects and are **not
`Send`**, so they cannot be parked in the `Mutex<HashMap<..>>` the rest of
Brume's state lives in.

The usual workaround is `unsafe impl Send` and a promise to behave. This does
not do that. Pending requests sit in a `thread_local!` on the main thread keyed
by a plain integer, and `answer_permission` hops back with `run_on_main_thread`
carrying only that integer and two bools. Nothing crossing a thread boundary is a
COM pointer, so there is no promise to break.

**This is also the answer to the download-cancel problem**, which was deferred
in 0.4.0 for exactly this reason: holding `ICoreWebView2DownloadOperation` to
cancel it later has the same shape and the same fix.

### Denied first, then overwritten

The handler sets DENY before doing anything else and only then takes the
deferral. If any step afterwards fails, or the window closes with the prompt
still up, the request resolves as denied rather than as whatever the runtime
would otherwise have decided.

Prompts queue rather than stack: a site asking for the camera and the microphone
raises two events, and two prompts at once is a question nobody reads properly.

Verified end to end against a real page calling `getCurrentPosition`: the page
sat at "waiting" while the prompt was up, came back `denied:1` the instant Block
was pressed, the decision survived a full restart with no second prompt, Reset
in Settings brought the asking back, and dismissing with Escape answered the page
while leaving nothing behind.

---

## Cancelling a download, and the pattern that unblocked it

Deferred in 0.4.0 because `ICoreWebView2DownloadOperation` is not `Send`, so it
could not be parked in the `Mutex<HashMap<..>>` the rest of Brume's state uses.
permissions.rs settled the pattern and it applies unchanged: the operation stays
in a `thread_local!` on the main thread, the key is a plain `String`, and
`cancel_download` hops back with `run_on_main_thread` carrying only that key. No
`unsafe impl Send`, so there is no promise to break.

Keyed by URL, which is what `finish_download` already matches on. Two downloads
of one URL at once are ambiguous here exactly as they are there.

**Nothing is written to the store when cancelling.** The runtime finishes the
transfer as interrupted, which fires wry's own `Finished` with `success: false`,
and the existing bookkeeping moves the row to the finished list. One path for a
download ending, however it ended.

A `StateChanged` handler removes the operation from the map once the transfer
leaves IN_PROGRESS, so the map holds exactly the downloads a cancel button is
showing for.

Verified: a 100 MB transfer cancelled at 30.9% emptied the active list, recorded
`success: false` with no path, and left no partial file behind.

---

## Tab audio: half of it works, and the half that does not is documented

`ICoreWebView2_8` carries both `IsMuted`/`SetIsMuted` and
`IsDocumentPlayingAudio`, with change events for each.

**Muting works.** `IsMutedChanged` fires, the value reaches the tab strip through
the same mirror-from-the-runtime pattern as `can_back` and `zoom`, and the
indicator switches to the struck-through speaker.

**`IsDocumentPlayingAudio` has never been observed to report true.** Tried on
2026-08-05 against WebView2 150.0.4078.105, tab both muted and unmuted: a Web
Audio oscillator, an `<audio>` element on a `data:` WAV, and an `<audio>` element
on a remote `.ogg`, at volumes 0.02, 0.1 and 0.15. `audible` stayed false
throughout.

The read is not the problem: the same `publish` call reports `muted` correctly
from the same interface in the same breath. Either the change event never fires
or the runtime does not count any of that as playing audio.

That mattered for the design rather than just being a curiosity. The obvious
control is an indicator in the tab strip, but it can only appear once a tab is
*already* audible or muted, so with the audible half silent it could never appear
in the first place: a control that does not exist. **The tab context menu is the
real entry point**, and the strip indicator is what shows a muted tab and offers
the way back.

The watcher code is kept rather than removed. It is correct against the
documented API, costs nothing while the event stays quiet, and starts working the
day the runtime reports it.

---

## Print was never a system modal

Carried as "unverified" since 0.3.0 on the grounds that it "opens a system modal
that has to be dismissed by hand, which would hang an unattended test and put a
dialog on the screen". **All three parts of that were wrong**, and it was never
checked.

`webview.print()` opens WebView2's own browser preview. It appears as a CDP
target at `edge://print/` inside the app window, with the paginated document in
a `chrome-untrusted://print/...print.pdf` iframe. Screenshotted on 2026-08-05: a
real printer enumerated, "Total: 1 sheet of paper", layout, colour and duplex
controls, and the page rendered with a running head and date.

Nothing blocked: CDP kept answering throughout, and enumerating the process's
visible top-level windows showed no new one. The system dialog exists, but it is
a separate opt-in behind "Print using system dialog... (Ctrl+Shift+P)" *inside*
that preview, which is presumably where the original claim came from.

Two things to know when testing it: `edge://print/` exposes no debugger
websocket, so it cannot be driven or dismissed over CDP; and while it is open it
is the first non-`tauri.localhost` page target, so `Resolve-BrumeTarget -Target
content` picks it rather than the page.

---

## Tab search, and a naming collision the tests caught

Ctrl+Shift+A, filtering the tab list already in `brume://state` so it searches
exactly what the strip drew. Worth having now that the strip scrolls: past about
26 tabs on a 1200px window most of them are off screen.

**The action was first called `tab_search`, and that was a real bug.**
`shortcuts.rs` dispatches Ctrl+1..8 with a guard arm on `action.starts_with("tab_")`
that parses the rest as a digit, and that arm sits above where the new handler
was added. So the shortcut would have been swallowed, tried to parse "search" as
a number, logged a line nobody reads, and done nothing.

`numbered_tab_actions_carry_a_usable_index` failed the moment it was added,
because it counts `tab_`-prefixed actions and expects exactly eight. That test
was written for precisely this - its comment says a typo like `tab_one` "would
compile, register, and then do nothing at all" - and it earned its keep. The
action is `search_tabs` now.

---

## The context menu is curated, not replaced

`ContextMenuRequested` offers two ways out. `SetHandled(true)` suppresses
WebView2's menu and leaves Brume to draw its own, which means reimplementing
positioning, keyboard navigation, submenus, and every item the engine already
gives away: copy, paste, spellcheck suggestions, image and media commands,
inspect. The other way is to modify the item collection the event hands over and
let the runtime draw it. That is what contextmenu.rs does.

**Nothing here was a repair.** The note claiming WebView2's "open link in new
window" did nothing was wrong, and had never been checked. The item is replaced
because its *label* lies: it says window and delivers a tab. Brume swaps in two
items that say what they do, and adds opening a link in a private tab, which the
engine cannot offer because privacy is Brume's idea and not the runtime's.

The stock item is matched on `Name`, not `Label`. The label is localised and
would stop matching the moment Brume ran in another language; the name is the
engine's own identifier.

### Verifying a menu you cannot read

A Chromium-drawn menu is not a Win32 menu, so there is nothing to enumerate, and
driving it would need the desktop-level input this project does not use.

What worked: right-click a link over CDP with `Input.dispatchMouseEvent`, which
is contained to the webview and never touches the desktop cursor, with the
process started under `Start-Process -RedirectStandardError` so the handler could
report what it did. Measured on 2026-08-05: the stock item was found and removed,
both Brume items were inserted at 0 and 1, and the collection went from 8 to 10.

The logging came back out afterwards, since a line per right-click is noise. Put
it back the same way if this ever needs checking again.

---

## Site info on the padlock

The padlock has always implied something it never said. Clicking it now answers
two questions: is this connection encrypted, and what has this site been allowed.

Both answers come from somewhere authoritative rather than from anything the
chrome remembers. The scheme is read off the live URL; the permissions come from
the runtime through `list_permissions`, so the popover cannot claim a site is
blocked while the engine allows it. Resetting one from here is the same call the
Settings list makes.

The certain parts are painted before the permissions are fetched, so the popover
appears at once rather than waiting on a round trip to say anything at all. It
checks the origin has not changed before appending, so a slow answer for a page
you have navigated away from is discarded rather than shown against the wrong
site.

An unencrypted page is stated plainly and coloured Lamplight rather than red. An
http page is not an attack, it is simply not private, and styling it as a warning
would cry wolf on every plain site on the internet.

---

## A new tab stopped being a request to a search engine

Opening a tab used to navigate to the active engine's landing page, because an
empty `homepage` meant "follow the search engine". That was a genuinely nice
property - choosing Mojeek moved the new-tab destination too - and the wrong
default for this browser. It meant a request to a third party every time a tab
was opened, before anything had been typed. Park ten tabs and a search engine is
told ten times that you are there.

`src/newtab.html` makes no network requests at all: the font, the mark and every
colour are local. It is also deliberately almost empty. There is no search box,
because the address bar is two centimetres away, already has history and
bookmark suggestions, and a second field doing the same job is a thing that ends
up disagreeing with the first.

The old behaviour is one click away in Settings rather than removed. That is what
`engine_homepage` exists for, and it is why `search.rs::home_for` and its
theme-matching tests are still live rather than orphaned by the change.

### It needed a hole in the navigation guard

`on_navigation` refuses anything on `tauri.localhost`, as a second lock behind
the capability scoping. A page served from there is exactly what a new tab page
has to be, so the guard now allows one exact path and nothing else.

The exception is narrow on purpose and costs little: a page that navigates itself
there gains nothing, because it holds no capabilities either way and stops being
able to render anything of its own the moment it goes. The address bar keeps
showing the real URL, so there is nothing to impersonate.

The page is kept out of history - it would otherwise be far and away the most
visited entry - and the address bar shows nothing for it rather than a
chrome-internal URL.

**This broke the test harness, quietly.** `Resolve-BrumeTarget` picked the chrome
by matching `*tauri.localhost*` and taking the first hit, and the new tab page
matches that too. It came first in the list, so `-Target chrome` started
returning a page with no toolbar in it. The chrome is matched on the root path
exactly now, and the new tab page counts as content, which is what it is.

---

## Importing bookmarks, and where it stops

Nobody moves browser without their bookmarks. Chrome, Edge, Brave, Vivaldi and
Opera all keep theirs in the same **plain JSON** file at a predictable path, so
reading them needs serde and nothing else.

Firefox keeps them in `places.sqlite`, which would mean bundling a SQLite driver -
the ~1.5 MB that store.rs explicitly refused for Brume's own storage - or parsing
the format by hand. Taking the easy 90% now beats taking none of it while waiting
to do all of it.

Three decisions worth keeping:

- **Add-only, never toggle.** `toggle_bookmark` is what the star needs and
  exactly the wrong call here: it would have *removed* every bookmark the two
  browsers had in common. Importing twice adds 0 the second time.
- **Dates come from the source.** Otherwise every imported bookmark claims to
  have been made today and the bar reorders itself wholesale.
- **`http` and `https` only.** A Chromium file can carry `javascript:`
  bookmarklets, and importing one would put a script behind a one-click button
  on the bookmarks bar. That filter is a security decision, and it is tested.

Folders are flattened, because Brume's list is flat. Flattening loses the
grouping and keeps every bookmark, which is the right way round: a bookmark in a
long list still works, one silently dropped does not. The Settings row says so
rather than letting it be discovered.

Renaming arrived with it, in place rather than in a dialog: a page title is often
not what you want on a 172px strip, and that is most obvious right after
importing forty of them.

Verified against real profiles: Chrome and Brave detected with counts, Edge
correctly absent, 46 imported, second run added 0, and every imported date landed
in the past rather than today.

---

## The lightweight claim, measured at last

`README.md` said Brume's "memory footprint far below an Electron-based
equivalent", and `BUILD_NOTES` called not bundling Chromium "the central decision
that makes Brume lightweight". Neither had a number anywhere in the repo.

Measured 2026-08-05, attributing WebView2 processes by walking parent PIDs so
other apps' runtime processes were not counted:

| | Brume |
|---|---|
| Installer | 5.1 MB |
| Installed on disk | 5.2 MB |
| `brume.exe` resident | 41 MB |
| One tab | ~590 MB, 9 processes |
| Five tabs | ~2.8 GB, 42 processes |

**The disk claim is emphatically true. The memory claim was not.** Once a page is
open, Brume costs what Chromium costs, because it is Chromium. Only the shell
around it is small, and 41 MB of shell does not offset an engine.

The saving that is real and worth stating is disk and distribution: 5 MB against
an Electron app's 100 MB before any application code, and no second copy of a
browser engine to ship with every update.

Also checked, because 42 processes for five tabs looked like a leak: closing back
to one tab drops to 9 processes and ~590 MB and stays there. Nothing is leaking;
it scales with what is open.

---

## Where a tab's cost actually goes, and what moved it

Measured 2026-08-05, attributing WebView2 processes by walking parent PIDs so
other applications' runtime processes are not counted.

**The shared processes are shared.** One browser process, one GPU, one crashpad
and two utility processes serve every tab. A tab adds exactly one renderer:

```text
  1 tab    7 processes  522 MB
  +1       8 processes  586 MB    +1 renderer,  +64 MB
  +1       9 processes  640 MB    +1 renderer,  +54 MB
  +1      10 processes  740 MB    +1 renderer, +100 MB
```

An earlier reading of "42 processes for five tabs" was wrong in a way worth
recording: it counted six processes belonging to Windows SearchHost, and it was
taken while tabs closed moments earlier were still being reaped. Attributing by
parent PID and waiting for the count to settle is the only way to get an honest
number here.

### TrySuspend freezes, it does not reclaim

`ICoreWebView2_3::TrySuspend` on background tabs looked like the obvious answer
and mostly is not. With four tabs and three of them parked, total resident memory
went from 1099 MB to 1038 MB: about 5%. Checking the processes afterwards showed
why - eight renderers still resident at 57 to 137 MB each. Suspension freezes
execution and keeps the heap.

It is kept, because freezing a background renderer is a real saving in CPU and
battery and costs nothing, but **it is not a memory fix** and the module says so.

### Not loading the tab at all is the memory fix

Session restore used to call `open_tab_inner` for every saved tab, so a session
of twenty meant twenty webviews and twenty page loads before the window was
usable, for pages nobody had asked to see.

Restored tabs are parked now: a row in the strip with the saved URL and a title
guessed from the host, and no webview until the tab is first activated. Measured
on the same six-tab session:

| | processes | memory | ready |
|---|---|---|---|
| Eager, all six loaded | 29 | 1982 MB | - |
| Parked | 8 | **501 MB** | **1.3s** |

That is 75% less memory at launch and 21 fewer processes, and the window is
usable in a second and a bit.

### Why this is not extended to a timer

Parking a tab that has already loaded would reclaim the same renderer, and it is
deliberately not done. Destroying a live webview also destroys scroll position
and anything typed into a form, and a tab you glanced away from for a minute is
not worth that. Chrome only discards under real memory pressure for the same
reason.

A restored tab has nothing to lose, which is exactly why the trick is safe there
and nowhere else.

### File size was already the cheap part

`Brume-Setup.exe` is 5.12 MB and the installed application is 5.2 MB, of which
`brume.exe` is 5.07 MB. The release profile already carries `opt-level = "s"`,
`lto`, `codegen-units = 1`, `panic = "abort"` and `strip`. There is no
meaningful fat left to trim, and the disk figure was never the problem: it is
the one number that was always comfortably better than an Electron equivalent.

---

## 0.6.0 plan: Library

Planned, not built. Written down because the 0.X releases so far were assembled
from whatever happened to be next on a list, and they read as a pile of unrelated
work rather than as a release.

### Themes are a planning device, and they do not ship

Each `0.X.0` now gets a theme, used here and in planning only. The names stay out
of release notes and tags: a name on a release page means a full `X.0.0`, and
spending that signal on a minor release would cost more than it buys.

| Release | Theme | What it covers |
|---|---|---|
| 0.6.0 | Library | Bookmarks, history and downloads: organising them, and getting them back out |
| 0.7.0 | Windows | A second window, moving a tab into it, split view, a tab sidebar |
| 0.8.0 | Power tools | DevTools, view source, custom search engines, per-site zoom |
| 1.0.0 | Shield | Content blocking, unchanged from the decision of 2026-08-03 |

Library goes first because every item in it finishes something already half
built. `import.rs` walks Chrome's folder tree and throws the structure away.
`store.history` has taken a query parameter since 0.3.0 and no caller passes one.
Import exists and export does not. None of that is new construction, and the one
piece that is, folders, has been deferred twice because it changes a file format.
That is not the sort of work that ever happens as an item on a list.

### bookmarks.json discards everything it cannot parse

Found while planning this, and it is why the release is ordered the way it is.
`store.rs:202` loads bookmarks like this:

```rust
let loaded = fs::read_to_string(&store.bookmarks_path)
    .ok()
    .and_then(|raw| serde_json::from_str::<Vec<Bookmark>>(strip_bom(&raw)).ok())
    .unwrap_or_default();
```

A parse failure becomes an empty `Vec`, and the next edit writes that empty list
back over the file. There is no `.bak`, which `settings.rs:202` does have, so the
bytes are gone rather than merely unreadable. A missing file and a corrupt file
are also indistinguishable here, and only one of those is normal.

It is latent today only because `Bookmark` has not changed since it was written.
Adding a field is precisely what makes it fire. So the fix lands first, before
the struct is touched: a missing file stays silent, and a file that is present
but does not parse is renamed to `bookmarks.json.bak` before anything starts
empty. The net has to exist before the change that needs it, or the first person
to find this loses their bookmarks.

**Built and verified on 2026-08-06**, against a running debug build, with the
real profile copied aside first and restored byte for byte afterwards. A file
that is present and will not parse is renamed to `bookmarks.json.bak` byte for
byte, confirmed by SHA-256 on both sides, and no replacement is written in its
place.

The control is the part that proves anything: a valid file loads, produces no
`.bak`, is left untouched at its original hash, and the running app reported the
bookmark back through the `bookmarks` command. Without that, a fix which simply
backed up every file unconditionally would have passed the corrupt case just as
well. A BOM-prefixed file, which is what Notepad leaves behind, also loads rather
than being taken for damage, which is the mistake that already cost a real bug in
`settings.rs`.

Startup writes no bookmarks file at all. With the file deleted outright, nothing
is created until something is bookmarked, so the loss this fixes happened on the
first edit rather than the moment the app opened. A slower fuse, the same
outcome. One reading during the first corrupt run showed a 2-byte `[]` file
appearing with an older timestamp than the file it replaced; it did not reproduce
from a clean profile and is not explained. Recorded rather than guessed at.

### The folder model is one array, not a tree

`Bookmark` gains two fields, both `#[serde(default)]`:

```rust
#[serde(default)] pub parent: Option<u64>,   // None is root
#[serde(default)] pub is_folder: bool,       // url unused when true
```

The top level stays a JSON array, and that is the entire point: an existing
`bookmarks.json` is already a valid new one, so no migration step exists to be
got wrong, and the corrupt-file path above is never entered by the upgrade
itself. Ids keep coming from one space, so the `max + 1` allocation at
`store.rs:353` and `:383` keeps working and a folder can never collide with a
bookmark.

Two alternatives were considered and rejected. A separate `folders.json` types
better, with no unused `url` hanging off a folder, but two files cannot be
written atomically together, so a crash between the writes leaves bookmarks
pointing at folders that do not exist, and `parent` would refer into a second id
space. A nested `enum` mirrors Chromium's own format and makes cycles and orphans
impossible by construction, and it can even stay backward compatible through
`#[serde(untagged)]`, because an old object with no children still matches the
link variant. That last part is the objection: untagged variants fail silently
and pick the nearest match, and the one code path already caught destroying data
without a trace is the wrong place to put something clever.

Invariants live in one place rather than at every call site. On load, a `parent`
that does not resolve to an entry with `is_folder` set is reset to root, which
covers dangling ids, references to a bookmark rather than a folder, and files
edited by hand. On move, walking up from the destination must not reach the
folder being moved, or a folder can be made its own ancestor and vanish from the
tree while still taking up space in the file.

**The model landed on 2026-08-06** as `parent`, `is_folder`, `repair_tree`,
`would_cycle`, `create_folder`, `move_bookmark` and `reorder_bookmark`, with
`remove_bookmark` changed to promote. Driven against a running build, with the
real profile restored byte for byte afterwards: a pre-folder file loaded and
defaulted to root, a folder was created and a bookmark filed into it, and both
reached disk. Moving a folder into its own descendant and into itself were each
refused, while an unrelated move still succeeded, which is the control that
stops "refuse everything" passing. Deleting the outer of two nested folders
brought the inner one up and left the grandchild attached to it; deleting that
one returned the bookmark to root. Nothing was lost at any step.

`repair_tree` was checked the same way, on a hand-damaged file carrying a
dangling parent, a link used as a parent, a two-folder cycle and one correctly
filed bookmark as the control. All four faults went to root, the control kept its
parent, all six entries survived, and no `.bak` was written, because the file
parsed perfectly well and was merely inconsistent. Those are different failures
and they get different treatment.

`reorder_bookmark` was wrong on the first attempt, which is the argument for
splitting it out. Moving an entry that was already in position relocated it past
unrelated entries sitting between it and its next sibling. Sibling order came out
correct, so the manager would have looked right while the file churned on every
no-op move. It now leaves an entry alone when it is already at the requested
index, and takes its insertion point from the preceding sibling rather than the
end of the array.

### What that model does not solve, stated rather than glossed

Ids come from `max + 1`, so deleting the highest-numbered entry frees that id for
the next thing created. Today that is harmless. With parent pointers it means a
stale reference can resolve to a different folder made later, and a bookmark
turns up somewhere nobody put it. Validating parents on load does not catch this,
because the id does resolve, just to the wrong entry.

It is left open deliberately. Closing it properly needs a persisted counter,
which means a top-level `next_id`, which turns the array into an object, which is
the shape change this whole design exists to avoid. Timestamp ids would also work
and are the fallback if it ever bites in practice.

### Deleting a folder promotes its children

Chrome and Edge delete the subtree. Brume does not, and the reason is two
sections up: this is a file with no recovery path, and its load routine has
already been caught discarding the lot. Deleting a folder moves its contents up
one level, so no single click can lose a collection. The cost is a genuine
surprise, because a folder full of bookmarks scatters into the level above
instead of tidying away.

### The bar reuses overlay.rs rather than growing a second mechanism

A folder menu hanging off the bookmarks bar has to paint over the page, and the
page sits above the chrome in Win32 z-order. That is the 0.5.0 finding that
produced `overlay.rs`, and the address bar dropdown already does the whole dance:
raise the chrome HWND, report the overlay bottom edge through `syncOverlay()`,
and keep that height out of `chrome_extent()` so no content webview is moved.
Folder menus take the same path.

Overflow needs less than it looks. The bar measures rather than counts, so a
folder is one more chip of variable width. What is new is nesting, which makes
the overflow menu renderer recursive.

### Reordering needs a path that is not dragging

Drag and drop is the obvious way to reorder a bookmark. It is also untestable
here: driving it needs synthetic input, which is barred, so a drag-only feature
could never be verified. Move up, move down and move to folder go in the
manager's context menu and on the keyboard, which makes the behaviour reachable
over CDP and is better for anyone not using a mouse. Dragging then becomes a
second path to commands that already exist rather than the only one.

### History search was already there, and it was already costing something

**The plan above was wrong about this and the correction is the useful part.** It
claimed the panel had never sent a query and that the search box was yet to be
built. It has sent one since 0.3.0: `renderPanel` passes `query` straight to
`store.history`, and the box has been in the panel header all along.

That makes the caching worth more, not less. `store.rs:142` said history was not
cached "because it is large and read only when the user actually opens the
panel", which would have been true had there been no search. There was, so every
debounced render re-read and re-parsed up to 20,000 lines. Debounced at 120ms, so
not the per-keystroke version of the 0.3.0 bug, but the same shape and live in
shipped code rather than hypothetical.

The parse is now cached while the panel is open and dropped by `release_history`
when it closes. `record_visit` appends to the cache instead of dropping it, so
browsing with the panel open does not re-parse per page load. That comment has
been rewritten, along with the one on the debounce that described the old cost.

What history genuinely lacked was day headings and a way to clear part of it
rather than all of it. Both are new.

Date grouping is front end work over `visited_at`. Clearing by range extends
`clear_history` with a cutoff, and `compact_history` already writes a filtered
file back, so the shape is there.

### Import stops flattening, and export is the missing half

`collect()` at `import.rs:129` already recurses. It takes a parent id and emits
folders as it descends instead of discarding them, and the test
`nested_folders_are_flattened_not_dropped` inverts into one asserting the tree
survives. The comment at `import.rs:20`, explaining that a tree cannot survive
the trip, goes with it.

Each import lands in its own dated folder, so a second import of the same source
never collides and is one deletion to undo. That keeps the add-only rule, which
was chosen because a two-way sync would remove bookmarks the source and Brume
both hold.

Export writes the whole tree as one Netscape bookmark HTML file, nested
`<DL><DT>` with `ADD_DATE`, which Chrome, Edge and Firefox all still read. It is
string building, so it costs no dependency, and it doubles as the manual backup
this file has never had. The save dialog makes the command `async`, for the
reason `blocking_pick_folder` already documents.

### Downloads

A failed or cancelled download offers a retry, re-issuing the original URL from
the start. It is not a resume: range requests need the partial file tracked and
server support detected, with a fallback to a full retry anyway, and that is its
own feature rather than part of this one.

### Non-goals for 0.6.0

Content blocking, which stays at 1.0.0. Multiple windows and split view, which
are 0.7.0 and which fight assumptions running through `chrome_extent()`,
`overlay.rs`, session restore and `browser_state`. DevTools and user agent
switching, which are 0.8.0. Sync, Firefox import, and any move to a database.

### Two bugs found by building on top of the menus

Neither was the feature being built, and both had shipped.

**Brume's own menus were cut off at the toolbar.** `openMenu` placed itself with
`Math.min(at.clientY, window.innerHeight - rect.height - 4)`, and in the chrome
webview `innerHeight` is the height of the bars, not of the window: 76px. So the
clamp went negative, every menu was pinned to y=4, and anything past 76px was
outside the webview and simply not drawn. Measured on 2026-08-06: a tab context
menu ran to 179px inside a 76px viewport, so 103px of it did not exist. A menu of
six items showed two.

The cause is the 0.5.0 finding again. `syncOverlay` raises and grows the chrome
for the address bar dropdown, the permission prompt, tab search and site info,
and `ctx-menu` was never added to that list even though it is HTML in the same
webview. It is now, `openMenu` calls `syncOverlay` after positioning, `closeMenu`
calls it again to shrink back, and the vertical clamp is against the same 720 the
Rust side clamps the overlay to rather than against `innerHeight`. Verified: the
chrome grows from 76 to 195 and the menu is fully inside it.

**The rename button was a solid square.** `.ico` fills with `currentColor` and
relies on a mask to cut the shape out, so a class with no `mask-image` paints a
filled block. `ico-edit` was used by the bookmark rename button from 0.5.0 and
never defined. `ico-upload` and `ico-chevron-down` were missing too, which the
export button and the folder chevron would have hit. All three are defined now.

### How this gets verified

Against a running browser, over CDP, asserting on something independent of the
UI that claims success. Bookmarks are read from `bookmarks.json` on disk rather
than from the panel. The upgrade path gets a real 0.5.0-shaped file dropped in
and loaded. The corrupt path gets deliberately malformed JSON, asserting the
`.bak` appears and the original bytes survive inside it. An export is re-imported
by Chrome. Folder depth, an attempted cycle, and deleting a folder with children
are pure store logic and get unit tests.

**Done on 2026-08-06.** 53 unit tests, clippy clean, formatting clean. The
profile was copied aside first and `bookmarks.json` came back byte for byte on
the same SHA-256 it started with. Driven against a running build:

| | |
|---|---|
| Bar shows the root level only | A folder and a loose bookmark; the two filed pages were not on it |
| Folder chip opens its contents | Nested folder as a submenu, link, and Open all |
| Submenu drills in | Replaced the menu with the inner folder's contents |
| Manager renders a tree | Indents of 10, 26 and 42px, in tree order, with item counts |
| Move to, on a folder | Offered nothing: the only other folder was its own descendant |
| Move up | Root order swapped, and the bar followed through `brume://bookmarks` |
| Move down at the end | Disabled rather than offered and refused |
| History day headings | Today, Yesterday, Tuesday |
| Clear the last hour | 90 visits to 82; all 82 older ones intact, the 8 recent ones gone |
| Bookmark search | Showed each hit's folder path, "in Reading / Later" |

**The export dialog is the one thing not driven.** `blocking_save_file` opens a
native modal, and putting one on the desktop is exactly what the testing rule
forbids, so it would also hang an unattended run. The HTML it writes is unit
tested, including nesting and escaping; the dialog itself is the installer
shell's already-proven pattern. Worth saying plainly rather than implying the
whole path was exercised.

---

## 0.7.0 plan: Windows

Planned, and being built. 0.6.0 shipped; this is the next theme, and it is the
one that argues with the architecture rather than extending it.

### What the release is, and what it is not

A second window, a private window, moving a tab between windows, and session
restore that remembers more than one of them.

Split view and a vertical tab sidebar were both in the original sketch and are
**deliberately out**. Neither needs the work below, because both live inside a
single window, so bundling them would mean two hard problems in one release with
nothing shared between them. They move to 0.8.0.

### The capability file is the part to get right

`capabilities/default.json` reads `"windows": ["main"]` and
`"webviews": ["chrome"]`, and carries a standing warning never to widen it,
because the content webview renders arbitrary websites and anything granted to it
is granted to every page the user visits.

A second window needs both to match more than one thing. The only safe way is a
glob that cannot ever match a content webview:

```text
  chrome webview   chrome-main, chrome-win-2, ...   capability matches chrome-*
  content webview  tab-1, tab-2, ...                matched by nothing
```

The two namespaces staying disjoint **is** the protection. That was previously a
naming convention and is now a security boundary, so it is stated on
`WindowState::chrome` where the label is built rather than only here.

### One global state becomes one per window

`Browser` held `tabs`, `closed`, `panel_open`, `find_open` and `overlay_bottom`.
Every one of those is a property of a single window's chrome. They move to a
`WindowState`, and `Browser` becomes a map of them keyed by window label, plus
the two counters that must stay global.

Tab ids move from `Tabs` to `Browser`. They have to be unique across the whole
app, because a webview label is `tab-{id}` and labels are app-wide, and because a
tab keeps its id when it moves between windows: reallocating on the way would
make it a different tab as far as every handler watching it is concerned.

The map holds `Arc<WindowState>` so a caller takes its window's state and drops
the map lock immediately. Holding the map while laying a window out would
serialise every window against every other, and a command that opened a window
while holding it would deadlock outright.

### Three ways to find the window you are acting on

Not one, because the callers genuinely differ.

A **command** takes `tauri::Window` and reads its label. That is the common case
and it is exact: the chrome that invoked it belongs to the window it is for.

An **event handler on a content webview** knows a tab and nothing else, so
`window_of_tab` scans the windows for the one holding it. A scan rather than a
`window` field on `Tab`, so that moving a tab between windows is only moving it
between two lists. A second record of where a tab lives is a second thing that
can be left pointing at the old window.

A **global shortcut** has no originating webview at all, so `focused_state` asks
Tauri which window is focused and falls back to any window. The fallback matters:
a keystroke arriving in the gap between one window losing focus and the next
gaining it should still do something rather than nothing.

### Sequenced so the refactor is not debugged alongside the feature

Everything above is threading a window label through code that never had one, and
it is about 70 call sites in `browser.rs` before the other modules are touched.
It lands **first, with one window still**, and has to behave exactly as 0.6.0 did
before a second window is added at all. Adding both at once would mean every bug
having two possible causes.

The same reasoning put the bookmarks recovery fix before the folder model in
0.6.0, and that ordering was right.

### Session restore needed a third key, not a changed one

`session_tabs` and `session_active` describe one window. The obvious move is to
turn the first into a list of lists, and it is the wrong one for exactly the
reason those keys already document: serde rejects a field of the wrong type
outright, and an unparseable settings file is moved aside wholesale, so every
install upgrading from 0.6.0 would have lost its entire settings file to get a
feature it had not asked for.

So `session_windows` is a new key with `#[serde(default)]`. It is read when it
has anything in it, and the two old keys are the fallback when it does not, which
is precisely the upgrade case: a 0.6.0 session becomes one window. The old keys
are still written to disk untouched rather than cleared, so a downgrade restores
the session it last understood instead of opening to nothing.

Geometry is deliberately **not** per window. A window is identified by its label,
labels are not stable across runs, and remembering where `win-3` sat tells you
nothing about which window that is next launch. The first window takes the saved
geometry and the rest are offset from wherever they open.

### Verified against a running build

2026-08-07, 56 unit tests, clippy clean, formatting clean. The profile was copied
aside and `bookmarks.json` came back on its original SHA-256.

| | |
|---|---|
| Single window after the refactor | Still builds, loads, and answers commands |
| The `chrome-*` capability | Chrome can still invoke; nothing else was widened |
| Two windows | Independent tab lists, ids globally unique across both |
| Opening tabs in one window | The other is untouched |
| Move tab to new window | Third window with that page; source lost the tab |
| Closing one window's last tab | That window closed, the other two kept running |
| Session across a hard kill | Both windows restored, with the right tabs in each |
| A 0.6.0-shaped settings.json | Restored as one window, right active tab, no `.bak` |

**One claim in this plan was wrong and the test caught it.** The note on
`next_tab_id` said ids must be globally unique partly "because a tab keeps its id
when it moves between windows". It does not: `move_tab_to_new_window` rebuilds
the tab, so it arrives with a fresh id, measured as 4 becoming 5. Global
uniqueness is still required, but for the other reason on its own, which is that
two windows would otherwise both allocate `tab-1` and the second `add_child`
would collide with the first. The comment now says that and says what the move
actually does.

WebView2 exposes no way to reparent a webview between windows, which is also why
the move loses scroll position and anything typed into a form. That is stated on
the command rather than left to be discovered.

### The test harness only knew one window

`Resolve-BrumeTarget` returned the first match and had no way to say which
window. It takes an `-Index` now, defaulting to 0, so every existing caller is
unchanged and a second window is reachable.

There is no way to ask CDP which window a target belongs to: targets are webviews
and a window is a Win32 concept above them. Order is what the runtime reports
consistently, so index is the honest handle rather than something dressed up as a
window id.

---

## 0.8.0: Power tools

In progress. The theme is the things a browser owes someone who wants to look
underneath it, plus the two layout features cut from 0.7.0.

### DevTools costs nothing, and the reason is the whole premise

`Webview::open_devtools` is gated behind `any(debug_assertions, feature =
"devtools")`, so without the feature a release build has no such method and the
browser ships unable to inspect anything. Adding the feature to a project whose
entire pitch is size is the sort of thing worth measuring before doing.

Measured on 2026-08-07: the release binary went from 5,544,960 bytes to
5,527,040. That is 17,920 *smaller*, which is codegen noise across an LTO build
that also gained the new code, not a saving. The honest reading is **no
measurable cost**.

The reason there is nothing to pay is the point of the project. DevTools is part
of the WebView2 runtime, not part of Brume: the feature flag only gates a method
that forwards to `OpenDevToolsWindow`. An Electron application ships its own copy
of the inspector; Brume borrows the one already on the machine, exactly as it
borrows the renderer.

Bound to F12 and Ctrl+Shift+I. Both, because F12 is what people reach for and
Ctrl+Shift+I is what the Chromium menus advertise. Verified on 2026-08-07: a
`devtools://devtools/bundled/devtools_app.html` target appears after the call.

### View source is the runtime's too

`view-source:` is Chromium's own scheme, so there is nothing to fetch and nothing
to render. It also means the markup shown is the document that was actually
loaded, rather than a second request that could come back different: a page that
varies by cookie or by time would otherwise show source that never existed.

The URL is built in Rust rather than typed, so it never passes through
`search::resolve`. `view-source:` is deliberately **not** added to
ALLOWED_SCHEMES: that list guards what a person can put in the address bar, and
widening it to serve a menu item would be paying in the wrong place.

Verified on 2026-08-07 against a running build: `view-source:https://example.com/`
opened as its own target showing `<!doctype html><html lang="en">...` with
Chromium's own line-wrap control. Note that `location.href` inside that document
reports the original URL, not the `view-source:` one, which is Chromium's
behaviour and worth knowing before writing a test that asserts on it.

Refused on Brume's own new tab page and on a document already being viewed as
source, since stacking the prefix gives a page about a page.

### Per-site zoom has to be reapplied on every navigation

WebView2's zoom belongs to the **webview**, not to the document. Following a link
from a site zoomed to 150% therefore leaves the next site at 150% too, which is
not per-site zoom, it is a sticky webview. So the stored factor is applied on
every load rather than once.

Applied on load *finished*, not on navigation start: applying it earlier means
the runtime resets the factor as the new document commits and the zoom silently
does not stick. And spawned rather than called from inside the handler, because
reentering a webview from its own event is what WebView2 warns against and what
history.rs already avoids.

Two paths, deliberately separate. `update_zoom` records what the user did and
writes it to settings; `update_zoom_display` only moves the indicator. Applying a
remembered zoom uses the second, or reading a value would immediately write it
back and settings.json would be rewritten on every navigation to a site that has
one.

Keyed on scheme and host, the same shape `permissions.rs` uses: zooming one
article is asking for the site to be bigger, not that URL. Only entries that
differ from 100% are kept, so resetting a site removes its row rather than
storing a 1.0, and a browser used for years does not accumulate one row per site
ever visited. The comparison uses a tolerance rather than `==`, because the
factor comes back from WebView2 as a float it stepped.

**A private tab records nothing.** Writing its zoom would put a site it visited
into settings.json, which is the one thing private browsing exists to avoid.

Verified on 2026-08-07 against a running build, with the page's own reported
width as the independent check rather than Brume's indicator:

| | |
|---|---|
| example.com at 150% | `innerWidth` 1280, and `{"https://example.com":1.5}` on disk |
| Navigating to example.org | Zoom 1, `innerWidth` 1920: it did not follow |
| Returning to example.com | Back to 1.5 and 1280 on its own |
| Zooming in a private tab | 200% in the session, nothing written to disk |

1920 / 1.5 is exactly 1280, which is what makes the width an assertion rather
than a coincidence.

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
