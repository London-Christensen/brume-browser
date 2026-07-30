# Build notes

A running log of the architectural decisions behind Brume, and the reasoning for each one.
The point of this file is that future-me can re-read a decision instead of re-litigating it.

---

## Step 0 — Repository setup

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
run. The key is stored outside the repository entirely — the `.gitignore` entries are a
backstop, not the primary control.

The corresponding **public** key is safe to commit and is embedded in `tauri.conf.json`.
The `!*.key.pub` negation exists so that a `.key.pub` file is never accidentally swept up by
the broader `*.key` rule.

---

## Step 1 — Environment

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
build per app — well over 100 MB before any application code. Brume renders through the
WebView2 runtime already present on the OS, so the installer carries only the Rust binary and
the frontend assets.

The tradeoff: WebView2 must exist on the target machine. It ships with Windows 11 and with
current Windows 10 installs, but is not guaranteed on older Windows 10 builds. Tauri's NSIS
bundler handles this with a **bootstrapper** — a small downloader invoked during installation
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
