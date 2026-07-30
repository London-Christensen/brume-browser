# Copies the runtime subset of the brand kit into the frontend.
#
# Why this exists: Tauri bundles exactly one directory (`frontendDist`, which is
# `src/`), so anything the running app references must physically live under it.
# The full brand kit lives at `brand/` and stays untouched, because it ships its
# own Python generators that regenerate in place - pointing the app at `brand/`
# directly is not an option, and hand-copying invites silent drift.
#
# So: `brand/` is authoritative, `src/assets/brand/` is derived. Re-run this after
# regenerating the kit. Never hand-edit anything under src/assets/brand/.
#
#   pwsh tools/sync-brand-assets.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$from = Join-Path $repo 'brand\assets'
$to   = Join-Path $repo 'src\assets\brand'

if (-not (Test-Path $from)) {
    throw "Brand kit not found at $from"
}

# Start clean so assets deleted upstream do not linger here.
if (Test-Path $to) { Remove-Item $to -Recurse -Force }
New-Item -ItemType Directory -Force -Path $to | Out-Null

# --- design tokens -----------------------------------------------------------
# The single source of truth for colour, type and geometry. Everything else in
# the frontend reads from these custom properties.
New-Item -ItemType Directory -Force -Path (Join-Path $to 'css') | Out-Null
Copy-Item (Join-Path $from 'css\tokens.css') (Join-Path $to 'css\tokens.css') -Force

# --- UI icon set -------------------------------------------------------------
# 24px grid, 2px stroke, drawn with currentColor so they theme for free.
Copy-Item (Join-Path $from 'icons') (Join-Path $to 'icons') -Recurse -Force

# --- logo sources ------------------------------------------------------------
# mark-sm is a genuine redraw for 16/24px, not a downscale of mark. Both are
# needed; the crossover is at 32px.
New-Item -ItemType Directory -Force -Path (Join-Path $to 'svg') | Out-Null
foreach ($f in 'mark.svg', 'mark-sm.svg', 'wordmark.svg', 'lockup-h.svg', 'lockup-v.svg') {
    Copy-Item (Join-Path $from "svg\$f") (Join-Path $to "svg\$f") -Force
}

$count = (Get-ChildItem $to -Recurse -File).Count
Write-Output "Synced $count brand asset(s) into src/assets/brand/"
