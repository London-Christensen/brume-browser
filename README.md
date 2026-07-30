# Brume

A lightweight web browser for Windows, built with [Tauri 2](https://v2.tauri.app/) and the
OS-native **WebView2** runtime.

Brume deliberately does *not* bundle its own copy of Chromium. It renders pages using the
WebView2 runtime that already ships with Windows, which keeps the installer small and the
memory footprint far below an Electron-based equivalent.

## Status

Early build, in active development. See [docs/BUILD_NOTES.md](docs/BUILD_NOTES.md) for the
running log of architectural decisions.

## Requirements

### To run Brume

- Windows 10 (1803+) or Windows 11
- WebView2 Runtime — pre-installed on Windows 11 and current Windows 10. The installer
  bundles a bootstrapper that fetches it automatically if missing.

### To build Brume

- [Rust](https://rustup.rs/) (stable, MSVC toolchain)
- Visual Studio 2022 Build Tools with the **Desktop development with C++** workload
  (provides `link.exe` and the Windows SDK that the Rust MSVC target links against)
- [Node.js](https://nodejs.org/) 18+ and npm — used only to run the Tauri CLI

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
npm run tauri build
```

Outputs an NSIS installer to:

```
src-tauri/target/release/bundle/nsis/
```

Running that `.exe` installs a standalone Brume application that runs independently of
this development environment.

## Project layout

```
src/          Frontend — the browser chrome (tabs, toolbar, address bar). Plain HTML/CSS/JS.
src-tauri/    Rust backend — window and tab management, persistence, OS integration.
docs/         Build notes, release process, and extension guides.
```

## Documentation

- [docs/BUILD_NOTES.md](docs/BUILD_NOTES.md) — architectural decisions and known limitations
- [docs/RELEASING.md](docs/RELEASING.md) — how to cut a new release and publish an update

## Licence

Not yet chosen.
