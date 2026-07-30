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
| — | Installing | Progress bar. NSIS cannot install without this page. |
| — | Finished | Completion, with a "run Brume now" button. |

### What was removed

- **Welcome page.** No information, no decision.
- **Language selector.** `displayLanguageSelector: false`, English only.
- **License page.** Never configured, so it was already absent.
- **Install-mode page.** Avoided by pinning `installMode: "currentUser"`.
- **Start menu folder page.** Skipped automatically because no `startMenuFolder` is set.

### What was kept, and why

- **Installing / Finished.** NSIS has no install step without `INSTFILES`, and removing the
  finish page makes the installer appear to vanish mid-action rather than complete.
- **The reinstall/maintenance page.** This one only appears when a previous Brume install is
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

Filled in when the dark reskin lands. Anything that could not be fully themed will be listed
here explicitly rather than left to be discovered.

## Verification performed

Against `Brume_0.1.0_x64-setup.exe` (1.02 MB), from a clean state — no install directory, no
registry key:

| Check | Result |
|---|---|
| Silent install (`/S`) exit code | 0 |
| Installed payload | `brume.exe` 2.92 MB + `uninstall.exe` 0.08 MB |
| Registry after default install | `AutoUpdate = 1` |
| Registry after unticking the box in the GUI | `AutoUpdate = 0` |
| Installed app runs standalone | Yes — window opens, WebView2 renders a live page |
| Uninstall | Removes install directory and registry key |

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
