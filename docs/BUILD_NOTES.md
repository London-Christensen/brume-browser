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
bundled — which happens whenever only the NSIS template or `tauri.conf.json` changed and cargo
therefore saw no reason to relink — fails with:

```
Warn Failed to add bundler type to the binary: __TAURI_BUNDLE_TYPE variable not found in binary.
     Updater plugin may not be able to update this package.
```

**It is only a warning.** The build succeeds, the installer is produced, everything looks fine,
and the resulting package may simply refuse to update itself later. That combination — silent,
non-fatal, and only observable much later during an update — is what makes it worth writing
down.

`tools/build-installer.ps1` runs `cargo clean -p brume --release` before bundling, which forces
a genuine relink and a fresh placeholder while leaving the dependency tree cached.

**Deleting `target/release/brume.exe` is not enough**, which is worth knowing because it is the
obvious thing to try and it silently does not work. Cargo links into
`target/release/deps/brume-<hash>.exe` and hardlinks that to `brume.exe`. The two names are one
file, so patching one patches both — and deleting the copy merely restores the already-patched
original. The giveaway is `Finished \`release\` profile in 1.12s`: no relink happened.

If you ever run `npm run tauri build` by hand and intend to ship the result, check the output
for that warning first.

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
