# Cuts a Brume release.
#
#   pwsh tools/new-release.ps1 -Version 0.2.0 -Notes "What changed."
#   pwsh tools/new-release.ps1 -Version 0.2.0 -Notes "..." -Publish
#
# Without -Publish this stops after building and writing dist/latest.json, and
# prints the exact command to publish. Publishing is a one-way, outward-facing
# action, so it is opt-in rather than the default.
#
# What it does:
#   1. Bumps the version everywhere it appears (they must not drift - a mismatch
#      silently breaks updates rather than failing loudly).
#   2. Builds and signs, via tools/build-installer.ps1.
#   3. Writes dist/latest.json from the generated signature.
#   4. Optionally creates the GitHub release and uploads the assets.
#
# See docs/RELEASING.md for the reasoning behind each step.

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Notes = '',
    [switch]$Publish,
    [switch]$SkipDirtyCheck
)

$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must be plain semver like 1.2.3 (got '$Version'). No leading v, no suffix."
}

$repo    = Split-Path -Parent $PSScriptRoot
$owner   = 'London-Christensen'
$project = 'brume-browser'
$tag     = "v$Version"

# --- refuse to build from a dirty tree --------------------------------------
# A release you cannot reproduce from a commit is not a release.
if (-not $SkipDirtyCheck) {
    Push-Location $repo
    try { $dirty = git status --porcelain } finally { Pop-Location }
    if ($dirty) {
        throw "Working tree is dirty. Commit or stash first, or pass -SkipDirtyCheck.`n$dirty"
    }
}

# --- 1. bump ----------------------------------------------------------------
# Every file below carries the version independently. They are updated together
# and then verified, because a mismatch does not fail the build - it produces an
# installer and a manifest that quietly disagree, and updates stop working.
$bumps = @(
    @{ Path = 'package.json';                  Pattern = '("version"\s*:\s*")[^"]+(")' }
    @{ Path = 'src-tauri/tauri.conf.json';     Pattern = '("version"\s*:\s*")[^"]+(")' }
    @{ Path = 'src-tauri/Cargo.toml';          Pattern = '(?m)^(version\s*=\s*")[^"]+(")' }
    @{ Path = 'installer-shell/Cargo.toml';    Pattern = '(?m)^(version\s*=\s*")[^"]+(")' }
    @{ Path = 'installer-shell/tauri.conf.json'; Pattern = '("version"\s*:\s*")[^"]+(")' }
)

Write-Output ''
Write-Output "=== Bumping to $Version ==="
foreach ($b in $bumps) {
    $full = Join-Path $repo $b.Path
    if (-not (Test-Path $full)) { throw "Missing $($b.Path)" }

    $raw = Get-Content $full -Raw
    if ($raw -notmatch $b.Pattern) { throw "No version field found in $($b.Path)" }

    # Only the first match: Cargo.toml's [package] version comes before any
    # dependency version, and package.json's own version precedes dependencies.
    $updated = [regex]::Replace($raw, $b.Pattern, "`${1}$Version`${2}", 1)
    # -NoNewline: Set-Content would otherwise append a trailing newline each run.
    Set-Content $full $updated -NoNewline -Encoding utf8
    Write-Output "      $($b.Path)"
}

# Verify they actually agree now.
foreach ($b in $bumps) {
    $raw = Get-Content (Join-Path $repo $b.Path) -Raw
    $m = [regex]::Match($raw, $b.Pattern)
    if ($m.Groups.Count -lt 1) { throw "Could not re-read version from $($b.Path)" }
    $found = [regex]::Match($raw, $b.Pattern).Value
    if ($found -notmatch [regex]::Escape($Version)) {
        throw "Version in $($b.Path) did not update correctly."
    }
}

# --- 2. build + sign --------------------------------------------------------
& (Join-Path $PSScriptRoot 'build-installer.ps1')

# --- 3. latest.json ---------------------------------------------------------
$nsisDir = Join-Path $repo 'src-tauri\target\release\bundle\nsis'

# The update artifact is the NSIS installer .exe itself, with a detached .sig
# beside it. Older Tauri wrapped it in a .nsis.zip first; 2.11 does not, and
# looking for the zip fails in a way that looks like broken signing rather than
# a changed filename. If a future Tauri reintroduces the archive, this is the
# line to revisit.
$artifact = Get-ChildItem $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
$sig = Get-ChildItem $nsisDir -Filter '*-setup.exe.sig' -ErrorAction SilentlyContinue |
       Sort-Object LastWriteTime -Descending | Select-Object -First 1

if (-not $artifact) {
    throw "No NSIS installer found in $nsisDir - stage 1 of the build did not complete."
}

if (-not $sig) {
    throw @"
The installer was built but never signed.

  installer: $($artifact.Name)
  expected:  $($artifact.Name).sig  (missing)

Without a signature every client will reject this update. Check that
bundle.createUpdaterArtifacts is true in src-tauri/tauri.conf.json, and that the
signing key and password were found - build-installer.ps1 reports which it used.
"@
}

# The updater downloads this asset from the tagged release, so the URL must
# point at the tag rather than at /latest - /latest would make every release
# advertise whichever one happens to be newest at download time.
$manifest = [ordered]@{
    version   = $Version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{
            signature = (Get-Content $sig.FullName -Raw).Trim()
            url       = "https://github.com/$owner/$project/releases/download/$tag/$($artifact.Name)"
        }
    }
}

$dist = Join-Path $repo 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$manifestPath = Join-Path $dist 'latest.json'
$manifest | ConvertTo-Json -Depth 6 | Set-Content $manifestPath -Encoding utf8

Write-Output ''
Write-Output "=== Release $tag prepared ==="
Write-Output "      update artifact : $($artifact.Name)  ($([math]::Round($artifact.Length/1MB,2)) MB)"
Write-Output "      manifest        : dist\latest.json"
Write-Output "      installer       : dist\Brume-Setup.exe"

$assets = @(
    (Join-Path $dist 'Brume-Setup.exe')   # for humans
    $artifact.FullName                     # for the updater
    $sig.FullName
    $manifestPath
)

if (-not $Publish) {
    Write-Output ''
    Write-Output 'Not published (no -Publish). To publish:'
    Write-Output ''
    Write-Output "  git commit -am ""Release $Version"" ; git tag $tag ; git push --follow-tags"
    Write-Output "  gh release create $tag --title ""Brume $Version"" --notes ""$Notes"" ``"
    foreach ($a in $assets) { Write-Output "    ""$a"" ``" }
    Write-Output ''
    return
}

# --- 4. publish -------------------------------------------------------------
Push-Location $repo
try {
    git commit -am "Release $Version"
    git tag $tag
    git push --follow-tags
    gh release create $tag --title "Brume $Version" --notes $Notes @assets
} finally {
    Pop-Location
}

Write-Output ''
Write-Output "=== Published $tag ==="
