# Copies the runtime subset of the brand kit into every frontend that ships one.
#
# Why this exists: Tauri bundles exactly one directory per app (`frontendDist`),
# so anything the running app references must physically live under it. The full
# brand kit lives at `brand/` and stays untouched, because it ships its own
# Python generators that regenerate in place - pointing an app at `brand/`
# directly is not an option, and hand-copying invites silent drift.
#
# So: `brand/` is authoritative, and every destination below is derived output
# that happens to be committed. Re-run this after regenerating the kit. Never
# hand-edit anything under a destination directory - they are wiped and rebuilt.
#
#   powershell tools/sync-brand-assets.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$from = Join-Path $repo 'brand\assets'

if (-not (Test-Path $from)) {
    throw "Brand kit not found at $from"
}

# Both frontends get the same set. The installer does not strictly need all 44
# icons, but they total a few kilobytes and keeping one asset set means one
# thing to reason about rather than two that can diverge.
$destinations = @(
    (Join-Path $repo 'src\assets\brand')
    (Join-Path $repo 'installer-shell\ui\assets\brand')
)

foreach ($to in $destinations) {
    # Start clean so assets deleted upstream do not linger here.
    if (Test-Path $to) { Remove-Item $to -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $to | Out-Null

    # --- design tokens -------------------------------------------------------
    # The single source of truth for colour, type and geometry. Everything else
    # reads from these custom properties.
    New-Item -ItemType Directory -Force -Path (Join-Path $to 'css') | Out-Null
    Copy-Item (Join-Path $from 'css\tokens.css') (Join-Path $to 'css\tokens.css') -Force

    # --- Archivo -------------------------------------------------------------
    # Vendored rather than fetched: both the installer and the browser chrome
    # need to render before a CDN request would return, and an installer that
    # phones out to draw its own UI is slow and leaky.
    Copy-Item (Join-Path $from 'fonts') (Join-Path $to 'fonts') -Recurse -Force

    # --- UI icon set ---------------------------------------------------------
    # 24px grid, 2px stroke, drawn with currentColor so they theme for free.
    Copy-Item (Join-Path $from 'icons') (Join-Path $to 'icons') -Recurse -Force

    # --- logo sources --------------------------------------------------------
    # mark-sm is a genuine redraw for 16/24px, not a downscale of mark. Both are
    # needed; the crossover is at 32px.
    New-Item -ItemType Directory -Force -Path (Join-Path $to 'svg') | Out-Null
    foreach ($f in 'mark.svg', 'mark-sm.svg', 'wordmark.svg', 'lockup-h.svg', 'lockup-v.svg') {
        Copy-Item (Join-Path $from "svg\$f") (Join-Path $to "svg\$f") -Force
    }

    $count = (Get-ChildItem $to -Recurse -File).Count
    $rel = $to.Substring($repo.Length + 1)
    Write-Output "Synced $count asset(s) -> $rel"
}
