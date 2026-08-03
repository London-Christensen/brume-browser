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
    [switch]$SkipDirtyCheck,

    # Attach latest.json to the release as well, purely so installs from before
    # the endpoint moved can find one last update.
    #
    # Up to and including 0.2.0 the compiled-in endpoint was
    #   .../releases/latest/download/latest.json
    # which resolves against whichever release is newest. Those installs cannot
    # be told about the new address, because the old one is baked into them. But
    # if the first release under the new scheme still carries latest.json as an
    # asset, they find it, update, and the version they land on reads from the
    # repository from then on.
    #
    # It is a one-release bridge. Pass it for that release and never again: once
    # a later release omits the asset the old URL 404s, which is harmless because
    # nothing is left pointing at it.
    [switch]$AttachLegacyFeed
)

$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must be plain semver like 1.2.3 (got '$Version'). No leading v, no suffix."
}

$repo    = Split-Path -Parent $PSScriptRoot
$owner   = 'London-Christensen'
$project = 'brume-browser'
$tag     = "v$Version"

# Writes text without a byte-order mark.
#
# `Set-Content -Encoding utf8` on PowerShell 5.1 prepends a UTF-8 BOM, and that
# is not cosmetic: it corrupts every strict parser downstream. A BOM on
# tauri.conf.json makes the build fail with "expected value at line 1 column 1",
# which names neither the file's real problem nor the tool that caused it. This
# script bumps five config files, so it was silently poisoning four of them.
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
function Write-TextFile([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, $Utf8NoBom)
}

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
    Write-TextFile $full $updated
    Write-Output "      $($b.Path)"
}

# Guard against the failure above ever returning quietly.
foreach ($b in $bumps) {
    $bytes = [System.IO.File]::ReadAllBytes((Join-Path $repo $b.Path))
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        throw "$($b.Path) was written with a UTF-8 BOM. Strict parsers reject it; see Write-TextFile."
    }
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
# Pinned to the version being released, and the signature derived from the
# artifact rather than picked independently.
#
# This directory is never cleaned, so it accumulates every build ever made -
# 0.1.0 and 0.2.0 sit side by side in it today. Taking "the newest .exe" and,
# separately, "the newest .sig" never checked that the two belonged together. If
# signing failed for the current build, the newest .exe was the new version
# while the newest .sig was still the previous release's, and they were paired
# without complaint.
#
# The result would be a manifest advertising the new version, linking the new
# installer, and carrying the old signature: every client downloads it and
# rejects it, silently, which is the exact failure RELEASING.md lists under
# "Clients see it but installation fails". A .sig existing is not the same as
# the .sig being the right one.
$artifact = Get-ChildItem $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like "*_${Version}_*" } |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1

if (-not $artifact) {
    $seen = (Get-ChildItem $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
             ForEach-Object { "        $($_.Name)" }) -join "`n"
    if (-not $seen) { $seen = '        (none)' }
    throw @"
No installer for $Version found in $nsisDir.

Stage 1 of the build did not produce one. Present in that directory:
$seen
"@
}

# The NSIS installer is no longer published, only verified.
#
# It is embedded inside Brume-Setup.exe by installer-shell/build.rs, so shipping
# it separately meant publishing a second copy of a file already inside the one
# above it. Confirming it exists at the right version still matters: it proves
# stage 1 built what stage 2 went on to embed.
Write-Output ''
Write-Output "      nsis payload    : $($artifact.Name)  ($([math]::Round($artifact.Length/1MB,2)) MB, embedded)"

# --- sign the file the updater will actually download -----------------------
#
# Tauri signs the NSIS installer automatically, but that is not the file being
# advertised any more. The signature in the manifest has to cover the exact
# bytes a client downloads, and that is Brume-Setup.exe, so it is signed here.
#
# The updater runs whatever the manifest points at with `/P /R /UPDATE`, and
# Brume-Setup.exe forwards those straight to the installer it carries instead of
# drawing its UI. See installer-shell/src/main.rs.
$dist = Join-Path $repo 'dist'
$setupExe = Join-Path $dist 'Brume-Setup.exe'
if (-not (Test-Path $setupExe)) { throw "Stage 2 produced no $setupExe" }

$setupSig = "$setupExe.sig"
# Removed first: signer failures must not leave the previous release's signature
# sitting there to be picked up as if it were this one's.
if (Test-Path $setupSig) { Remove-Item $setupSig -Force }

Write-Output '      signing dist\Brume-Setup.exe'
# Reads TAURI_SIGNING_PRIVATE_KEY and ..._PASSWORD, which build-installer.ps1
# has already put in the environment. Native command, so the exit code is
# checked by hand.
npx tauri signer sign $setupExe
if ($LASTEXITCODE -ne 0) { throw "Signing $setupExe failed with exit code $LASTEXITCODE." }

if (-not (Test-Path $setupSig)) {
    throw @"
Signing reported success but produced no signature.

  expected: $setupSig

Without it every client rejects the update. Check the signing key and password;
build-installer.ps1 reports which it used.
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
            signature = (Get-Content $setupSig -Raw).Trim()
            url       = "https://github.com/$owner/$project/releases/download/$tag/Brume-Setup.exe"
        }
    }
}

New-Item -ItemType Directory -Force $dist | Out-Null
$manifestPath = Join-Path $dist 'latest.json'
Write-TextFile $manifestPath ($manifest | ConvertTo-Json -Depth 6)

# The tracked copy, which is what the compiled-in endpoint actually fetches.
# Written but deliberately NOT committed here - see the publish step for why the
# order matters.
$feedPath = Join-Path $repo 'updates\latest.json'
New-Item -ItemType Directory -Force (Split-Path $feedPath) | Out-Null

Write-Output ''
Write-Output "=== Release $tag prepared ==="
Write-Output "      published asset : dist\Brume-Setup.exe  ($([math]::Round((Get-Item $setupExe).Length/1MB,2)) MB)"
Write-Output "      update feed     : updates\latest.json  (committed, not attached)"

# What actually gets attached to the release.
#
# The .sig is deliberately NOT here. Its contents are already embedded in
# latest.json as the `signature` field, and that is the only place the updater
# reads them from: it fetches the manifest, then the installer, and verifies one
# against the other. It never requests a .sig URL. Uploading the file as well
# just put a fourth item on the release page that nothing downloads.
#
# The file is still produced and still checked above, because a missing or stale
# .sig is how we detect that signing did not run. It is a build artifact, not a
# release asset.
#
# The three below are all load-bearing:
#   Brume-Setup.exe            humans, first install
#   Brume_<ver>_x64-setup.exe  the updater downloads this; latest.json links it
#   latest.json                the update endpoint itself
#
# GitHub adds "Source code (zip)" and "Source code (tar.gz)" to every release on
# its own. Those cannot be turned off.
# The release page body, which is not the same text as the update prompt.
#
# $Notes alone goes into latest.json, because that is what the running app shows
# in its "an update is available" dialog. Someone reading that dialog is not
# about to download anything, so install instructions there would be nonsense.
#
# The GitHub page has the opposite problem: a visitor sees several files and no
# indication which one to click, and two of them are a machine-readable update
# feed that no person should ever download. So the page body leads with the
# answer and says plainly what the rest are for.
$releaseBody = @"
### Install

Download **Brume-Setup.exe** and run it. That is the only file you need.

$Notes

<details>
<summary>What are the other files?</summary>

``Brume_${Version}_x64-setup.exe`` and ``latest.json`` are how an installed copy
of Brume updates itself. It reads ``latest.json``, then downloads and verifies
that installer in the background. Neither is meant to be downloaded by hand.
</details>
"@

# Passed to gh as a file rather than inline. The body is multi-line, and
# threading that through a quoted --notes argument breaks in both the printed
# fallback command and the real one. Not a release asset, just a local file.
$notesPath = Join-Path $dist 'release-notes.md'
Write-TextFile $notesPath $releaseBody

# One asset. That is the whole point of this arrangement.
#
# The NSIS installer is inside Brume-Setup.exe already, and latest.json is served
# from the repository, so neither needs attaching. A visitor to the release page
# sees the file to download and the two source archives GitHub adds by itself.
$assets = @($setupExe)

if ($AttachLegacyFeed) {
    # Same file, same contents as the tracked copy. Old clients resolve it
    # through releases/latest/download/, new ones read it from the repository,
    # and both get the same answer.
    $assets += $manifestPath
    Write-Output ''
    Write-Output '      NOTE: attaching latest.json for pre-0.2.0-endpoint installs.'
    Write-Output '            One release only. Do not pass -AttachLegacyFeed again.'
}

if (-not $Publish) {
    Write-Output ''
    Write-Output 'Not published (no -Publish). To publish:'
    Write-Output ''
    # -a: an annotated tag. `git push --follow-tags` ignores lightweight ones, so
    # a plain `git tag` leaves the tag local and `gh release create` then refuses
    # to build a release from a tag the remote has never seen.
    Write-Output "  git commit -am ""Release $Version"" ; git tag -a $tag -m ""Brume $Version"" ; git push --follow-tags"
    Write-Output "  gh release create $tag --title ""Brume $Version"" --notes-file ""$notesPath"" ``"
    foreach ($a in $assets) { Write-Output "    ""$a""" }
    Write-Output ''
    Write-Output '  # Only after the release exists, or the feed points at a 404:'
    Write-Output "  copy ""$manifestPath"" ""$feedPath"""
    Write-Output "  git add updates/latest.json ; git commit -m ""Point the update feed at $Version"" ; git push"
    Write-Output ''
    return
}

# --- 4. publish -------------------------------------------------------------
# Every exit code below is checked by hand.
#
# $ErrorActionPreference = 'Stop' does not apply to native commands: git and gh
# can fail and the script carries straight on. That matters most between the
# push and the release - a rejected push followed by `gh release create` does
# not fail, because gh creates the missing tag itself from the default branch.
# The release then ships built from a different commit than the tag names, which
# is precisely the class of quiet mismatch this script exists to prevent.
Push-Location $repo
try {
    git commit -am "Release $Version"
    if ($LASTEXITCODE -ne 0) { throw "git commit failed. Nothing was released." }

    # Annotated, so --follow-tags actually pushes it.
    git tag -a $tag -m "Brume $Version"
    if ($LASTEXITCODE -ne 0) { throw "git tag $tag failed. Does the tag already exist?" }

    git push --follow-tags
    if ($LASTEXITCODE -ne 0) { throw "git push failed. Nothing was released; the tag is local only." }

    gh release create $tag --title "Brume $Version" --notes-file $notesPath @assets
    if ($LASTEXITCODE -ne 0) {
        throw "gh release create failed. The commit and tag are pushed - delete the tag or retry the release."
    }

    # The update feed goes last, deliberately.
    #
    # It names a download URL on the release that was only just created. Pushing
    # the feed first would advertise a version whose installer does not exist
    # yet, and every client checking in that window would fail its download and
    # report the update as broken. Publish the file, then point at it.
    Copy-Item $manifestPath $feedPath -Force
    git add -- 'updates/latest.json'
    if ($LASTEXITCODE -ne 0) { throw "git add of the update feed failed." }

    git commit -m "Point the update feed at $Version"
    if ($LASTEXITCODE -ne 0) { throw "Committing the update feed failed. The release is live but nobody will be offered it." }

    git push
    if ($LASTEXITCODE -ne 0) { throw "Pushing the update feed failed. The release is live but nobody will be offered it." }
} finally {
    Pop-Location
}

Write-Output ''
Write-Output "=== Published $tag ==="
