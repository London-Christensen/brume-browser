# Builds the UI icon set in assets/icons/ from Lucide.
#
# Why this is PowerShell when the rest of tools/ is Python: it has to run on the
# machine that builds Brume, and Python is not installed there. The other
# generators produce the mark from geometry constants and genuinely need Python's
# maths; this one copies files and rewrites a tag.
#
# Why Lucide rather than the hand-drawn set that used to live in icons.py: the
# set was drawn to a house rule of 90 degrees, 45 only for direction, and arcs
# only where a thing is genuinely round. It was consistent, and it did not look
# good enough. Lucide is 24 grid, 2px stroke, currentColor, which is the same
# geometry the mask pipeline in src/index.html already expects, so nothing in the
# app had to change.
#
# Licence: Lucide is ISC, which permits redistribution provided the copyright
# notice travels with the files. Every generated file therefore carries the
# notice in a comment, and NOTICE records it at the repository level.
#
#   powershell brand/tools/build-icons.ps1
#   powershell tools/sync-brand-assets.ps1     # then push it to both frontends

$ErrorActionPreference = 'Stop'

$brand = Split-Path -Parent $PSScriptRoot
$repo = Split-Path -Parent $brand
$src = Join-Path $repo 'node_modules\lucide-static\icons'
$out = Join-Path $brand 'assets\icons'

if (-not (Test-Path $src)) {
    throw "lucide-static not found at $src. Run: npm install"
}

$version = (Get-Content (Join-Path $repo 'node_modules\lucide-static\package.json') -Raw |
            ConvertFrom-Json).version

# Brume's name on the left, Lucide's on the right. The left column is what the
# app asks for and must not churn: src/index.html references these by filename.
#
# Deliberate choices worth keeping:
#   private     a mask, because a private tab hides who you are, not what you do
#   settings    sliders rather than a gear, which the old set also chose
#   fullscreen  corner brackets; the window's own maximise button is a plain
#               square, so the two read as different actions
#   restore     square-stack rather than copy. copy is the closer match to the
#               Windows restore-down glyph, but it is already spoken for below
#               and two identical files is worse than an approximate one.
$MAP = [ordered]@{
    # --- navigation ---------------------------------------------------------
    'back'            = 'arrow-left'
    'forward'         = 'arrow-right'
    'reload'          = 'rotate-cw'
    'home'            = 'house'

    # --- tabs ---------------------------------------------------------------
    'tab'             = 'app-window'
    'tab-new'         = 'square-plus'
    'tab-pin'         = 'pin'
    'tab-audio'       = 'volume-2'
    'split'           = 'columns-2'
    'sidebar'         = 'panel-left'

    # --- address bar and security -------------------------------------------
    'lock'            = 'lock'
    'shield'          = 'shield'
    'shield-off'      = 'shield-off'
    'info'            = 'info'
    'warning'         = 'triangle-alert'
    'private'         = 'venetian-mask'

    # --- toolbar actions ----------------------------------------------------
    'download'        = 'download'
    'upload'          = 'upload'
    'bookmark'        = 'bookmark'
    'history'         = 'history'
    'extensions'      = 'puzzle'
    'menu'            = 'menu'
    'more'            = 'ellipsis-vertical'
    'settings'        = 'sliders-horizontal'
    'search'          = 'search'
    'zoom-in'         = 'zoom-in'
    'zoom-out'        = 'zoom-out'
    'print'           = 'printer'

    # --- window -------------------------------------------------------------
    'fullscreen'      = 'maximize'
    'fullscreen-exit' = 'minimize'
    'maximize'        = 'square'
    'restore'         = 'square-stack'
    'close'           = 'x'
    'minimize'        = 'minus'

    # --- content ------------------------------------------------------------
    'copy'            = 'copy'
    'trash'           = 'trash-2'
    'edit'            = 'pencil'
    'external'        = 'external-link'
    'check'           = 'check'
    'chevron-down'    = 'chevron-down'
    'chevron-right'   = 'chevron-right'
    'plus'            = 'plus'

    # --- theme --------------------------------------------------------------
    'theme'           = 'contrast'
}

# cleave.svg is NOT in the map and is never touched. It is the mark itself,
# generated from the geometry constants by kit.py, and it is the one glyph in
# this directory that is Brume's own.
$KEEP = @('cleave')

$notice = "<!-- Lucide v$version, ISC. Copyright (c) Lucide Icons and Contributors. -->"

# Lucide ships one element per shape, self-closing, already on the 24 grid.
$shape = '<(?:path|rect|circle|line|polyline|polygon|ellipse)\b[^>]*/>'

$written = 0
foreach ($name in $MAP.Keys) {
    $from = Join-Path $src "$($MAP[$name]).svg"
    if (-not (Test-Path $from)) {
        throw "Lucide has no icon named '$($MAP[$name])' (wanted for '$name')"
    }

    $raw = Get-Content $from -Raw

    # Body only. Lucide's opening tag carries width, height and a class that
    # Brume does not want: the app sizes these in CSS, and a hard width would
    # override it wherever one is inlined rather than masked.
    if ($raw -notmatch '(?s)<svg\b[^>]*>(.*?)</svg>') {
        throw "Could not parse $from"
    }
    $body = $Matches[1]

    $shapes = [regex]::Matches($body, $shape) |
              ForEach-Object { '  ' + ($_.Value -replace '\s+', ' ') }
    if (-not $shapes) { throw "No shapes found in $from" }

    $svg = @(
        $notice
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" ' +
        'stroke="currentColor" stroke-width="2" stroke-linecap="round" ' +
        "stroke-linejoin=`"round`" role=`"img`" aria-label=`"$name`">"
        $shapes
        '</svg>'
    ) -join "`n"

    # ASCII, no BOM. Set-Content -Encoding utf8 writes a BOM on PowerShell 5.1,
    # and a BOM in front of an SVG breaks strict parsers. Lucide path data is
    # ASCII, so nothing is lost.
    Set-Content -Path (Join-Path $out "$name.svg") -Value ($svg + "`n") -Encoding ascii -NoNewline
    $written++
}

# Anything left over is a file the map no longer produces. Reported rather than
# deleted, because cleave.svg lives here too and a silent wipe would take it.
$expected = @($MAP.Keys) + $KEEP
$strays = Get-ChildItem $out -Filter *.svg |
          Where-Object { $expected -notcontains $_.BaseName } |
          Select-Object -ExpandProperty Name

Write-Output "Wrote $written icon(s) from Lucide v$version -> brand/assets/icons"
Write-Output "Kept: $($KEEP -join ', ')"
if ($strays) {
    Write-Warning "Not produced by the map, left in place: $($strays -join ', ')"
}
