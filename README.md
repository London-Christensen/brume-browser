# Brume

A lightweight web browser for Windows, built with [Tauri 2](https://v2.tauri.app/) and the
OS-native **WebView2** runtime.

Brume deliberately does *not* bundle its own copy of Chromium. It renders pages using the
WebView2 runtime that already ships with Windows, which keeps the installer small and the
memory footprint far below an Electron-based equivalent.

## Status

Shipped and self-updating. See [docs/BUILD_NOTES.md](docs/BUILD_NOTES.md) for the running log
of architectural decisions, and [docs/RELEASING.md](docs/RELEASING.md) for how a release is
cut.

## Requirements

### To run Brume

- Windows 10 (1803+) or Windows 11
- WebView2 Runtime. Pre-installed on Windows 11 and current Windows 10; the installer
  bundles a bootstrapper that fetches it if missing.

### To build Brume

- [Rust](https://rustup.rs/) (stable, MSVC toolchain)
- Visual Studio 2022 Build Tools with the **Desktop development with C++** workload
  (provides `link.exe` and the Windows SDK that the Rust MSVC target links against)
- [Node.js](https://nodejs.org/) 18+ and npm, used only to run the Tauri CLI

## Getting started

```bash
npm install
```

### Run in development

```bash
npm run tauri dev
```

Builds the Rust backend in debug mode and opens the app with live-reload on frontend
changes. The first run compiles the full dependency tree and takes a few minutes;
subsequent runs are incremental and fast.

### Produce the Windows installer

```bash
powershell tools/build-installer.ps1
```

This runs both stages and drops the shippable installer at **`dist/Brume-Setup.exe`**. That
single file is what you publish; the NSIS installer is embedded inside it.

The two stages exist because the installer is two pieces:

1. `npm run tauri build` bundles the browser and produces an NSIS installer in
   `src-tauri/target/release/bundle/nsis/`.
2. `cargo build --release` in `installer-shell/` builds the custom-UI front end, embedding
   stage 1's output.

The order matters: stage 2 cannot compile without stage 1's artifact. Running
`npm run tauri build` alone is still fine when you only want to test the browser; it just
produces the plain NSIS installer rather than the styled one.

Either installer installs a standalone Brume application that runs independently of this
development environment. See [docs/INSTALLER.md](docs/INSTALLER.md) for why it is split this
way.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl`+`T` | New tab |
| `Ctrl`+`Shift`+`N` | New private tab |
| `Ctrl`+`W` | Close tab |
| `Ctrl`+`Shift`+`T` | Reopen the last closed tab |
| `Ctrl`+`Tab` / `Ctrl`+`Shift`+`Tab` | Next / previous tab |
| `Ctrl`+`1`…`8` | Switch to that tab |
| `Ctrl`+`9` | Last tab |
| `Ctrl`+`L` | Focus the address bar |
| `Ctrl`+`F` | Find on page. `Enter` / `Shift`+`Enter` step, `Esc` closes |
| `Ctrl`+`R` / `F5` | Reload |
| `Alt`+`←` / `Alt`+`→` | Back / forward |
| `Alt`+`Home` | Homepage |
| `Ctrl`+`D` | Bookmark this page |
| `Ctrl`+`H` | History |
| `Ctrl`+`,` | Settings |
| `Ctrl`+`P` | Print |
| Right-click a tab | Duplicate, pin, close others, close to the right |
| `F11` | Fullscreen |
| `Ctrl`+scroll, `Ctrl`+`+`/`-`/`0` | Zoom the page |

Shortcuts are registered while Brume has focus and released when it loses it, so they are not
held hostage from other applications. Zoom is handled by WebView2 itself rather than by Brume.
See [docs/BUILD_NOTES.md](docs/BUILD_NOTES.md) for why menu accelerators could not be used.

`Esc` is deliberately not registered. These are global shortcuts while the window has focus, so
claiming `Esc` would take it from every page: no dismissing a site's own dialog, no leaving its
fullscreen. The find bar handles it locally instead, which works because the chrome already has
focus while you are typing in it.

## Project layout

```
src/              Frontend — the browser chrome (tabs, toolbar, address bar). Plain HTML/CSS/JS.
  assets/brand/     Runtime brand assets. Generated — see below, do not hand-edit.
src-tauri/        Rust backend — window and tab management, persistence, OS integration.
  installer/        Custom NSIS template, plus the header/sidebar bitmaps.
installer-shell/  Brume-Setup.exe — the custom-UI installer that drives NSIS underneath.
brand/            The Brume brand kit: mark, wordmark, icon set, fonts, design tokens, spec.
tools/            Build and maintenance scripts.
docs/             Build notes, release process, and extension guides.
dist/             Build output. Not committed.
```

## Brand assets

`brand/` holds the complete brand kit and is the source of truth. Because Tauri only bundles
the `src/` directory, the subset the running app needs (design tokens, the 44-icon UI set and
the logo SVGs) is copied into `src/assets/brand/` by:

```bash
powershell tools/sync-brand-assets.ps1
```

Re-run that after changing anything under `brand/assets/`. Never hand-edit files in
`src/assets/brand/`. The script wipes and recreates that directory.

All colour, type and geometry values come from `brand/assets/css/tokens.css`. A hard-coded
hex value anywhere in the UI is a bug.

## Documentation

- [docs/BUILD_NOTES.md](docs/BUILD_NOTES.md): architectural decisions and known limitations
- [docs/INSTALLER.md](docs/INSTALLER.md): the custom NSIS installer, and its maintenance cost
- [docs/RELEASING.md](docs/RELEASING.md): how to cut a new release and publish an update
- [docs/TESTING.md](docs/TESTING.md): driving the app over a socket instead of hijacking the mouse

## Licence

Code is under the **Apache License 2.0**. See [LICENSE](LICENSE).

Apache-2.0 rather than MIT for two reasons that matter for a browser: it carries an
explicit patent grant, and it explicitly grants no trademark rights. The dependency
tree is `Apache-2.0 OR MIT` throughout, so this is also the least friction.

**The brand is not covered.** Everything under `brand/` is reserved: the Brume
name, the mark, the wordmark and the brand specification. Fork the code freely,
but ship it under your own name and your own marks. The 24px UI icon set is the
one exception and may be reused with the code.

See [NOTICE](NOTICE) for the full statement, including third-party components.
