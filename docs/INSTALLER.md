# The Brume installer

Brume's installer is two pieces:

| Piece | What it is | Who sees it |
|---|---|---|
| **`Brume-Setup.exe`** | A Tauri app in `installer-shell/` with a fully custom HTML/CSS UI | Humans, on a first install |
| **`Brume_<ver>_x64-setup.exe`** | A custom NSIS template at `src-tauri/installer/installer.nsi` | Nobody, normally; it runs silently underneath |

**`dist/Brume-Setup.exe` is the file to publish.** It is self-contained: the NSIS installer is
embedded inside it.

## Why a shell over NSIS, rather than replacing it

MUI2 is a thin skin over Win32 dialog resources. Every widget is a native control that paints
itself, which is why the reskin below hits a hard ceiling: buttons and checkbox captions are
drawn by the Windows theme engine and ignore the colours they are handed.

The shell sidesteps that entirely by drawing its own UI in a webview, so it gets real
typography, real animation, a rounded borderless window and the Brume palette exactly.

What it does *not* do is reimplement installation. The NSIS installer still places files,
registers with Add/Remove Programs, writes the uninstaller and creates shortcuts. The shell
collects two decisions and then runs it:

```
Brume_setup.exe /S [/NOAUTOUPDATE] /D=<install dir>
```

**The reason this split matters is updates.** Tauri's updater applies an update by running the
NSIS installer with `/P /UPDATE` and no UI at all. The pretty shell is never involved. Had NSIS
been replaced outright, the uninstaller, the Add/Remove Programs entry and the entire
update-apply path would all have had to be rebuilt, and the updater would need a custom
artifact type it does not support.

### Three things about that command line

- **`/D` must be last, must not be quoted, and swallows the rest of the line verbatim.** That
  last rule is what lets an unquoted path containing spaces work. Rust's normal argument
  quoting corrupts it, so the shell uses `raw_arg` throughout.
- **`/NOAUTOUPDATE` is a Brume addition**, checked in `.onInit` after the stored registry value
  so it takes precedence. Silent installs have no Options page, so this flag is the only way a
  caller can express the user's choice, and it keeps NSIS as the single writer of the setting
  rather than having the shell race it.
- **A zero exit code is not proof of success.** A bad `/D` path fails quietly, so the shell
  checks that `brume.exe` actually exists before reporting success.

### The WebView2 chicken-and-egg

The shell renders its UI in WebView2, but bootstrapping WebView2 is the NSIS installer's job.
If the runtime is missing, `tauri::Builder::run` cannot even draw an error message.

So `main()` probes the registry for the WebView2 client key **before Tauri starts**, and hands
straight off to the NSIS installer with its own UI if the runtime is absent. The user gets the
plain installer instead of the styled one, which is a fair trade for it working at all.

### Build order is not optional

The shell embeds the NSIS installer with `include_bytes!`, so the NSIS artifact must exist
before the shell compiles. `build.rs` discovers it by pattern rather than by fixed name,
because the filename carries the version; a hardcoded name would silently embed a stale
payload after a version bump.

```bash
pwsh tools/build-installer.ps1
```

runs both stages in order and drops the result in `dist/`. Running `cargo build` in
`installer-shell/` on its own fails with an explicit message rather than a confusing one.

---

## The NSIS layer

Everything below concerns `src-tauri/installer/installer.nsi`, the installer that runs
underneath the shell, and that runs *alone* during an auto-update.

## Why a custom template at all

Two reasons, one of which is unavoidable:

1. **The "Enable Automatic Updates" checkbox.** Tauri's bundler config exposes no way to add a
   page or a control. A custom page is only expressible in NSIS itself.
2. **Dark theming.** Background colours, text colours and widget styling are not reachable
   through `tauri.conf.json`. See "Theming" below.

## The maintenance cost, stated plainly

**Tauri's future improvements to the default `installer.nsi` will not reach us automatically.**
When Tauri fixes an installer bug, whether an upgrade edge case, a WebView2 bootstrapper change, a
Windows compatibility fix, that fix lands in *their* template, and ours carries on unaware.

This is an accepted tradeoff for the checkbox and the dark theme, but it is a real cost and it
compounds quietly.

To keep it manageable, `src-tauri/installer/installer.upstream-2.11.4.nsi` is a **pristine copy
of the upstream template** at the version ours was forked from. When upgrading Tauri:

```bash
# 1. What changed upstream between our fork point and the new version?
diff installer.upstream-2.11.4.nsi <new upstream installer.nsi>

# 2. What did we change?
diff installer.upstream-2.11.4.nsi installer.nsi
```

Two small diffs beat one archaeological dig. After merging, replace the pristine copy with the
new upstream version and rename it to match.

Every Brume modification is marked with a `BRUME ADDITION` or `REMOVED FOR BRUME` comment, so
the second diff should never be surprising.

## The flow

Deliberately minimal: two decisions, then install:

| # | Page | Notes |
|---|---|---|
| 1 | **Choose Install Location** | Defaults to `%LOCALAPPDATA%\Brume` |
| 2 | **Options** | One checkbox: *Enable Automatic Updates*, ticked by default |
| n/a | Installing | Progress, ending in a **Close** button. NSIS cannot install without this page. |

### What was removed

- **Welcome page.** No information, no decision.
- **Language selector.** `displayLanguageSelector: false`, English only.
- **License page.** Never configured, so it was already absent.
- **Install-mode page.** Avoided by pinning `installMode: "currentUser"`.
- **Start menu folder page.** Skipped automatically because no `startMenuFolder` is set.
- **Finish page.** Removed for two reasons: it was a third screen asking nothing new, and its
  two checkboxes could not be themed (see Theming). What it offered is preserved: the desktop
  shortcut is now created unconditionally in `Section Install`, and the app appears in the
  Start menu, so it is one click away the moment the installer closes.

### What was kept, and why

- **Installing.** NSIS has no install step without `INSTFILES`. With the finish page gone this
  is the last page, and its button becomes *Close*, which ends the installer cleanly.
- **The reinstall/maintenance page.** This only appears when a previous Brume install is
  detected, so a first install never sees it. It is what makes upgrades and repairs work, and
  removing it to satisfy a literal two-screen count would break the update path.

### Per-user, not per-machine

`installMode: "currentUser"` installs to `%LOCALAPPDATA%\Brume` with **no UAC prompt**. This
matters more than it looks: the auto-updater relaunches this installer non-interactively, and a
per-machine install would need elevation every time an update lands: either a UAC prompt the
user did not ask for, or a silent failure.

The cost is that Brume installs per Windows account rather than once for the whole machine.
For a personal browser that is the right trade.

## The installer-to-app handoff

The checkbox has to reach a program that is not running yet. It travels through the registry:

```
HKCU\Software\London Christensen\Brume
  (default)   REG_SZ     install directory
  AutoUpdate  REG_DWORD  1 = check on launch, 0 = do not
```

The key path derives from `bundle.publisher` and `productName`. **`publisher` is pinned
explicitly in `tauri.conf.json` for exactly this reason.** Left unset, Tauri derives the
manufacturer from the bundle identifier, and the path the app reads would silently drift from
the path the installer writes.

The app reads this value once, on first launch, to seed its own setting. From then on the
setting belongs to the app and is changed in Settings, not by reinstalling.

### The update-must-not-reset-it problem

The updater relaunches this same installer with `/P /UPDATE`. In passive mode the Options page
never renders, so a naive implementation that wrote "checked" unconditionally would **silently
re-enable update checks that the user had deliberately turned off**, on every update.

The fix is in `.onInit`, which runs before any page:

```nsis
ClearErrors
ReadRegDWORD $AutoUpdateEnabled SHCTX "${MANUPRODUCTKEY}" "AutoUpdate"
${If} ${Errors}
  StrCpy $AutoUpdateEnabled 1
${EndIf}
```

An existing value always wins. Only a genuinely fresh install falls through to the default of
enabled. Rewriting the value during an update is then a no-op rather than a reset.

`.onInit` seeding must stay **after** `!insertmacro SetContext`, because `SHCTX` does not resolve to
the correct hive until the install mode is known.

## Theming

The installer is dark to match the browser. Colours come from
`brand/assets/css/tokens.css` (Ink `#101418`, Paper `#F3F4F5`, Haar `#9DB2C0`) and are
declared near the top of `installer.nsi` as `MUI_BGCOLOR` / `MUI_TEXTCOLOR`.

The header strip and finish sidebar are BMP files, because NSIS accepts nothing else and
fixes their dimensions (150x57 and 164x314). They are generated from the real brand rasters by
`tools/make-installer-art.ps1`, composited onto the same Ink field as `MUI_BGCOLOR` so they sit
flush against the page instead of floating on a pale rectangle. **If you change `MUI_BGCOLOR`,
re-run that script** or the images will show a visible seam.

### Two things that make this harder than it looks

**`MUI_BGCOLOR` only reaches the header.** The body of a standard MUI page is a stock Windows
dialog that paints itself with system colours. Darkening it means walking its child controls by
ID in a `MUI_PAGE_CUSTOMFUNCTION_SHOW` callback and calling `SetCtlColors` on each, which is
what `DirectoryPageShow` and `InstfilesPageShow` do. The IDs are fixed by NSIS's own dialog
resources.

**Themed `BUTTON` controls ignore `SetCtlColors` for their caption.** Windows paints group
boxes, checkboxes and radio buttons through the theme engine, which never asks for the colour
returned by `WM_CTLCOLORSTATIC`. The caption is therefore drawn in the system text colour,
near-black on Ink, i.e. invisible. Two different workarounds are used:

- `BrumeUnthemeAndColor` calls `uxtheme::SetWindowTheme(hwnd, "", "")` to strip the visual
  style from one control, after which `SetCtlColors` is honoured. Used for the "Destination
  Folder" group box. The cost is that the widget renders in the classic style.
- The Options page checkbox keeps its modern themed tick box and instead carries an **empty
  caption**, with the text drawn beside it as a separate static that we colour freely. The
  static is created with `SS_NOTIFY` so clicking the words still toggles the box.

### What could not be themed

Listed explicitly rather than left to be discovered:

| Element | Why |
|---|---|
| **Back / Next / Close / Cancel buttons** | Windows common controls painted by the theme engine. Recolouring needs owner-draw or a subclassing plugin: real complexity, and unthemed buttons are what every Windows installer looks like anyway. |
| **Browse… and Show details buttons** | Same. |
| **Progress bar** | A native `msctls_progress32`. It renders green regardless; `MUI_INSTFILESPAGE_PROGRESSBAR "smooth"` at least avoids the segmented Classic look. |
| **Window title bar** | Drawn by the OS, not the application. NSIS does not opt into the Windows 11 dark title bar. |
| **Details-log scrollbars** | Native scrollbars follow the system theme. |
| **The finish page's checkboxes** | Not themable *and* not reachable. `$MUI_HWND` is not declared for that page, `ioSpecial.ini` carries no `HWND` entries, and probing IDs 1200–1214 against the inner dialog returns nothing. This is why the page was removed rather than left with invisible text. |

Everything else (page backgrounds, all body text, the install-path field, the group box, the
Options checkbox and its caption, the install log) is on the Brume palette.

## Verification performed

### The shell: `dist/Brume-Setup.exe` (4.27 MB)

| Check | Result |
|---|---|
| Window renders | Rounded, borderless, genuinely transparent corners |
| Install path pre-filled | `%LOCALAPPDATA%\Brume` from `default_install_dir` |
| **Install path containing spaces** | Installed correctly to `…\Brume Space Test` |
| Auto-update toggle off → registry | `AutoUpdate = 0` |
| Done state | Shows the real install path |
| Launch Brume | Started `brume.exe` from the spaced path, closed the installer |
| **WebView2 missing** | No shell window; handed straight to the NSIS installer's own UI |

The spaced-path case is the one that matters most, because it is where the obvious
implementation breaks. Rust's `Command::arg` quotes any argument containing a space, and a
quoted `/D="C:\Some Path"` makes NSIS install to the wrong place, usually the default,
without erroring. Hence `raw_arg` throughout, and hence the post-install check that
`brume.exe` really landed where it was asked to.

The WebView2 case was tested by temporarily pointing the registry probe at an all-zero GUID so
the runtime appeared absent, then reverting. That is worth repeating after any change to
`webview2_available`, since the failure is invisible on a machine that has the runtime.

### The NSIS layer: `Brume_0.1.0_x64-setup.exe` (1.04 MB)

From a clean state, with no install directory and no registry key:

| Check | Result |
|---|---|
| Silent install (`/S`) exit code | 0 |
| Installed payload | `brume.exe` 2.92 MB + `uninstall.exe` 0.08 MB |
| Registry after default install | `AutoUpdate = 1` |
| Registry after unticking the box in the GUI | `AutoUpdate = 0` |
| Desktop + Start menu shortcuts | Created on every install |
| Installed app runs standalone | Yes; window opens, WebView2 renders a live page |
| Uninstall | Removes install directory, registry key and shortcuts |

One trap worth recording, since it cost a wrong conclusion once already: on a machine with
OneDrive folder redirection, `$DESKTOP` resolves to `%USERPROFILE%\OneDrive\Desktop`, not
`%USERPROFILE%\Desktop`. Checking the latter reports the shortcut as missing when it was
created correctly. Use `[Environment]::GetFolderPath('Desktop')` when verifying.

## Testing it yourself

```bash
pwsh tools/build-installer.ps1
```

Then run `dist/Brume-Setup.exe`. To exercise the NSIS layer on its own, which is what an
auto-update actually runs, use `src-tauri/target/release/bundle/nsis/Brume_0.1.0_x64-setup.exe`
directly.

To inspect the handoff value after installing:

```bash
reg query "HKCU\Software\London Christensen\Brume" /v AutoUpdate
```

## Uninstalling

The supported path is Windows **Settings → Installed apps**, or running
`%LOCALAPPDATA%\Brume\uninstall.exe` directly. Append `/S` to either the installer or the
uninstaller to run it without a UI. The confirm page carries a **Delete application data**
checkbox; leave it unticked and settings, history, bookmarks and the WebView2 profile all
survive the uninstall, which is what you want when reinstalling.

That path assumes a healthy install. When it is not available (`uninstall.exe` missing, no
registry entry so Settings never lists Brume, a scratch install from testing, or an install
whose shortcut and registry entry disagree about where the binary lives), use:

```bash
pwsh tools/uninstall-brume.ps1
```

It reports what it found and changes nothing. Add `-Run` to remove it, and `-IncludeData` to
take the browsing profile with it:

```bash
pwsh tools/uninstall-brume.ps1 -Run -IncludeData
```

Reporting is the default deliberately, because `-IncludeData` deletes bookmarks and history.

### The `.setup` profile

`Brume-Setup.exe` is itself a Tauri app, with its own bundle identifier of
`<bundle id>.setup`, so WebView2 gives it a **second** profile directory next to the
browser's. Upstream's uninstaller removes `$APPDATA\${BUNDLEID}` and
`$LOCALAPPDATA\${BUNDLEID}` as exact paths, so it never named this one and left roughly
10 MB of cache behind on every uninstall, for a window the user saw once. The uninstaller
now removes it alongside the other two, and `tools/uninstall-brume.ps1` knows about it for
installs predating that fix.
