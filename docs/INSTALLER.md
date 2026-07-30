# The Brume installer

Brume ships a **custom NSIS template** at `src-tauri/installer/installer.nsi`, wired up via
`bundle.windows.nsis.template` in `tauri.conf.json`.

## Why a custom template at all

Two reasons, one of which is unavoidable:

1. **The "Enable Automatic Updates" checkbox.** Tauri's bundler config exposes no way to add a
   page or a control. A custom page is only expressible in NSIS itself.
2. **Dark theming.** Background colours, text colours and widget styling are not reachable
   through `tauri.conf.json`. See "Theming" below.

## The maintenance cost, stated plainly

**Tauri's future improvements to the default `installer.nsi` will not reach us automatically.**
When Tauri fixes an installer bug — an upgrade edge case, a WebView2 bootstrapper change, a
Windows compatibility fix — that fix lands in *their* template, and ours carries on unaware.

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

Deliberately minimal — two decisions, then install:

| # | Page | Notes |
|---|---|---|
| 1 | **Choose Install Location** | Defaults to `%LOCALAPPDATA%\Brume` |
| 2 | **Options** | One checkbox: *Enable Automatic Updates*, ticked by default |
| — | Installing | Progress, ending in a **Close** button. NSIS cannot install without this page. |

### What was removed

- **Welcome page.** No information, no decision.
- **Language selector.** `displayLanguageSelector: false`, English only.
- **License page.** Never configured, so it was already absent.
- **Install-mode page.** Avoided by pinning `installMode: "currentUser"`.
- **Start menu folder page.** Skipped automatically because no `startMenuFolder` is set.
- **Finish page.** Removed for two reasons — it was a third screen asking nothing new, and its
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
per-machine install would need elevation every time an update lands — either a UAC prompt the
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
explicitly in `tauri.conf.json` for exactly this reason** — left unset, Tauri derives the
manufacturer from the bundle identifier, and the path the app reads would silently drift from
the path the installer writes.

The app reads this value once, on first launch, to seed its own setting. From then on the
setting belongs to the app and is changed in Settings, not by reinstalling.

### The update-must-not-reset-it problem

The updater relaunches this same installer with `/P /UPDATE`. In passive mode the Options page
never renders — so a naive implementation that wrote "checked" unconditionally would **silently
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

`.onInit` seeding must stay **after** `!insertmacro SetContext` — `SHCTX` does not resolve to
the correct hive until the install mode is known.

## Theming

The installer is dark to match the browser. Colours come from
`brand/assets/css/tokens.css` — Ink `#101418`, Paper `#F3F4F5`, Haar `#9DB2C0` — and are
declared near the top of `installer.nsi` as `MUI_BGCOLOR` / `MUI_TEXTCOLOR`.

The header strip and finish sidebar are BMP files, because NSIS accepts nothing else and
fixes their dimensions (150x57 and 164x314). They are generated from the real brand rasters by
`tools/make-installer-art.ps1`, composited onto the same Ink field as `MUI_BGCOLOR` so they sit
flush against the page instead of floating on a pale rectangle. **If you change `MUI_BGCOLOR`,
re-run that script** or the images will show a visible seam.

### Two things that make this harder than it looks

**`MUI_BGCOLOR` only reaches the header.** The body of a standard MUI page is a stock Windows
dialog that paints itself with system colours. Darkening it means walking its child controls by
ID in a `MUI_PAGE_CUSTOMFUNCTION_SHOW` callback and calling `SetCtlColors` on each — which is
what `DirectoryPageShow` and `InstfilesPageShow` do. The IDs are fixed by NSIS's own dialog
resources.

**Themed `BUTTON` controls ignore `SetCtlColors` for their caption.** Windows paints group
boxes, checkboxes and radio buttons through the theme engine, which never asks for the colour
returned by `WM_CTLCOLORSTATIC`. The caption is therefore drawn in the system text colour —
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
| **Back / Next / Close / Cancel buttons** | Windows common controls painted by the theme engine. Recolouring needs owner-draw or a subclassing plugin — real complexity, and unthemed buttons are what every Windows installer looks like anyway. |
| **Browse… and Show details buttons** | Same. |
| **Progress bar** | A native `msctls_progress32`. It renders green regardless; `MUI_INSTFILESPAGE_PROGRESSBAR "smooth"` at least avoids the segmented Classic look. |
| **Window title bar** | Drawn by the OS, not the application. NSIS does not opt into the Windows 11 dark title bar. |
| **Details-log scrollbars** | Native scrollbars follow the system theme. |
| **The finish page's checkboxes** | Not themable *and* not reachable — `$MUI_HWND` is not declared for that page, `ioSpecial.ini` carries no `HWND` entries, and probing IDs 1200–1214 against the inner dialog returns nothing. This is why the page was removed rather than left with invisible text. |

Everything else — page backgrounds, all body text, the install-path field, the group box, the
Options checkbox and its caption, the install log — is on the Brume palette.

## Verification performed

Against `Brume_0.1.0_x64-setup.exe` (1.02 MB), from a clean state — no install directory, no
registry key:

| Check | Result |
|---|---|
| Silent install (`/S`) exit code | 0 |
| Installed payload | `brume.exe` 2.92 MB + `uninstall.exe` 0.08 MB |
| Registry after default install | `AutoUpdate = 1` |
| Registry after unticking the box in the GUI | `AutoUpdate = 0` |
| Desktop + Start menu shortcuts | Created on every install |
| Installed app runs standalone | Yes — window opens, WebView2 renders a live page |
| Uninstall | Removes install directory, registry key and shortcuts |

One trap worth recording, since it cost a wrong conclusion once already: on a machine with
OneDrive folder redirection, `$DESKTOP` resolves to `%USERPROFILE%\OneDrive\Desktop`, not
`%USERPROFILE%\Desktop`. Checking the latter reports the shortcut as missing when it was
created correctly. Use `[Environment]::GetFolderPath('Desktop')` when verifying.

## Testing it yourself

```bash
npm run tauri build
```

Then run `src-tauri/target/release/bundle/nsis/Brume_0.1.0_x64-setup.exe`.

To inspect the handoff value after installing:

```bash
reg query "HKCU\Software\London Christensen\Brume" /v AutoUpdate
```

To uninstall, use Windows "Apps & features", or run `%LOCALAPPDATA%\Brume\uninstall.exe`.
Append `/S` to either the installer or the uninstaller to run it without a UI.
