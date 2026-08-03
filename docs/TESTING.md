# Testing Brume without taking over the machine

The default way to test a desktop app is to synthesise mouse and keyboard input.
It works, and it makes the computer unusable while it runs: `SetForegroundWindow`
steals focus mid-keystroke, `SetCursorPos` moves the real pointer, and `SendKeys`
types into whatever is focused if the timing slips by 200ms.

Brume does not need any of that. WebView2 exposes the same DevTools Protocol
Chrome does, so the app can be driven over a local socket instead. Precisely,
with no focus, and with the window parked off-screen.

## Quick start

```powershell
. tools/cdp.ps1

Start-BrumeDebug -OffScreen          # launch with the debug port, out of sight
Invoke-BrumeJs 'document.title'      # evaluate in the chrome webview
Invoke-BrumeJs -Target content 'location.href'
Get-BrumeShot -Target content -Out page.png
Stop-BrumeDebug
```

`-OffScreen` moves the window to `(-4000, -4000)`. It still renders and still
screenshots; it simply never appears on your desktop.

## Driving the UI

Anything the user can do, the chrome's own JavaScript can do, so a test presses
buttons by dispatching events rather than by clicking pixels:

```powershell
# Type an address and press Enter
Invoke-BrumeJs @'
const u = document.getElementById("url");
u.value = "rust ownership rules";
u.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
'@

# Press Back
Invoke-BrumeJs 'document.getElementById("back").click()'

# Read state
Invoke-BrumeJs 'JSON.stringify({ back: !back.disabled, forward: !forward.disabled })'
```

This is more reliable than synthetic input as well as less rude: no focus races,
no coordinates to recalculate when the toolbar layout changes, and it works while
the window is behind something else.

## Screenshots

`Get-BrumeShot` uses CDP's `Page.captureScreenshot`, which renders the webview
directly. It needs neither focus nor visibility.

The one thing it cannot capture is the composite OS window: title bar plus both
webviews together. For that, capture the two webviews separately, or use
`PrintWindow` with `PW_RENDERFULLCONTENT`, which reads a window's pixels without
raising it. `CopyFromScreen` is the function to avoid: it captures whatever is
physically on the display, so it requires the window to be visible and on top.

## Security note

The debug port is **opt-in and never compiled in**. It is enabled by setting
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` at launch,
which `Start-BrumeDebug` does and `Stop-BrumeDebug` clears. A shipped Brume has
no debug port. Do not add that variable to a build script or a release profile:
anything able to reach the port can execute JavaScript in every open page.

## What still needs a real install

Most work does not. `Start-BrumeDebug` runs the built binary straight out of
`target/`, so nothing is installed and nothing is written to the registry.

Genuinely installing is only required for:

- the installer flow itself (the two screens and the auto-update checkbox)
- the installer-to-app registry handoff
- the end-to-end update test

When those are needed, install into a scratch directory rather than the real one:

```powershell
# NSIS: /D must be last, unquoted, and consumes the rest of the line
& .\Brume_0.1.0_x64-setup.exe /S /D=C:\Temp\brume-test
```

That leaves `%LOCALAPPDATA%\Brume` untouched. The registry handoff still writes to
`HKCU\Software\London Christensen\Brume`, which is the point of the test; remove
the key afterwards to reset.

The installer shell is itself a Tauri app, so it can be driven the same way by
launching `Brume-Setup.exe` with the same environment variable, with no clicking
through its dialogs by hand.

## Rust tests

Logic that does not need a GUI should not be tested through one. `search.rs` is
the model: the address-bar heuristics, the ad-suppression parameters and the
brand-token binding are all covered by unit tests that run in milliseconds.

```bash
cargo test --bin brume
```

Prefer moving a behaviour down to this level over automating it at the UI level.
