# Builds the complete, shippable Brume installer.
#
# Two stages, and the order is not optional: the installer shell embeds the NSIS
# installer with include_bytes!, so the NSIS artifact has to exist before the
# shell is compiled.
#
#   1. `tauri build`  -> src-tauri/target/release/bundle/nsis/Brume_<ver>_x64-setup.exe
#   2. `cargo build`  -> installer-shell/target/release/Brume-Setup.exe
#
# The stage 2 output is the file to publish. It is self-contained: the NSIS
# installer is inside it.
#
#   pwsh tools/build-installer.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot

# Rust lands in the persistent user PATH, but a shell opened before Rust was
# installed keeps the environment it started with.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }

# Both toolchains report progress on stderr, and PowerShell 5.1 turns any stderr
# from a native command into a terminating error while ErrorActionPreference is
# Stop - even on a successful build. Exit codes are the reliable signal, so the
# preference is relaxed around each native call and checked explicitly.
function Invoke-Native {
    param([string]$What, [scriptblock]$Command)

    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }

    if ($code -ne 0) { throw "$What failed with exit code $code" }
}

# --- updater signing key ----------------------------------------------------
#
# With createUpdaterArtifacts enabled, `tauri build` produces a signed .nsis.zip
# alongside the installer. Signing needs the private key, and the lookup order
# matters: an explicitly set environment variable always wins, so CI can inject
# a secret without touching this file.
#
# The key deliberately lives outside the repository. Losing it means no existing
# install can ever be updated again - a new key would be rejected by every
# client already carrying the old public key. See docs/RELEASING.md.
$keyFile  = Join-Path $env:USERPROFILE '.tauri\brume-updater.key'
$passFile = Join-Path $env:USERPROFILE '.tauri\brume-updater.pass'

if ($env:TAURI_SIGNING_PRIVATE_KEY) {
    Write-Output '      signing with TAURI_SIGNING_PRIVATE_KEY from the environment'
} elseif (Test-Path $keyFile) {
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyFile -Raw).Trim()
    if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
        if (-not (Test-Path $passFile)) {
            # Fail here rather than let the build reach the signing step.
            #
            # Without the password Tauri prompts on stdin, and an unattended
            # build has no stdin to answer with - it hangs indefinitely, long
            # after compiling and bundling have apparently succeeded. That looks
            # exactly like a stalled build. Better to stop now and say why.
            #
            # Setting the variable to an empty string is not a workaround:
            # Windows cannot represent an empty environment variable, so both
            # `$env:X = ''` and SetEnvironmentVariable(X, '') delete it outright
            # and the child process sees nothing. A key with no password simply
            # cannot be used non-interactively on Windows.
            throw @"
Updater signing key found, but no password.

  key:      $keyFile
  password: $passFile  (missing)

Tauri will prompt for the password on stdin, which an unattended build cannot
answer - the build hangs rather than fails. Set
TAURI_SIGNING_PRIVATE_KEY_PASSWORD, or restore the password file.
See docs/RELEASING.md.
"@
        }
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content $passFile -Raw).Trim()
    }
    Write-Output "      signing with $keyFile"
} else {
    Write-Warning @"
No updater signing key found.

  Looked for: `$env:TAURI_SIGNING_PRIVATE_KEY  (not set)
              $keyFile  (missing)

The build will still produce an installer, but no signed update artifact, and
existing installs would refuse an unsigned one. See docs/RELEASING.md.
"@
}

Write-Output ''
Write-Output '=== Stage 1/2  browser + NSIS installer ==='

# Force the main binary to be rebuilt before bundling.
#
# `tauri build` stamps the bundle type into the executable by overwriting a
# placeholder variable compiled into it, and the updater reads that marker later
# to work out how to apply an update. The placeholder is consumed the first time
# it is written, so bundling an already-bundled binary fails with
# "__TAURI_BUNDLE_TYPE variable not found".
#
# It is only a warning, which is what makes it dangerous: the build succeeds, the
# installer is produced, and the package may simply refuse to update itself much
# later.
#
# Deleting target/release/brume.exe is NOT enough. Cargo links into
# target/release/deps/brume-<hash>.exe and hardlinks that to brume.exe, so
# patching one patches both - and deleting the copy just restores the already
# patched original without relinking. `cargo clean -p brume` discards the crate's
# own artifacts while leaving the dependency tree cached, which forces a genuine
# relink and a fresh placeholder for the cost of one link step.
Push-Location (Join-Path $repo 'src-tauri')
try {
    Invoke-Native 'cargo clean -p brume' { cargo clean -p brume --release }
} finally {
    Pop-Location
}

Push-Location $repo
try {
    Invoke-Native 'tauri build' { npm run tauri build }
} finally {
    Pop-Location
}

$mainBinary = Join-Path $repo 'src-tauri\target\release\brume.exe'

if (-not (Test-Path $mainBinary)) { throw 'Stage 1 did not produce brume.exe' }

$nsis = Get-ChildItem (Join-Path $repo 'src-tauri\target\release\bundle\nsis') -Filter '*setup.exe' |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $nsis) { throw 'Stage 1 produced no NSIS installer.' }
Write-Output ("      payload: {0}  ({1:N2} MB)" -f $nsis.Name, ($nsis.Length / 1MB))

Write-Output ''
Write-Output '=== Stage 2/2  installer shell ==='
Push-Location (Join-Path $repo 'installer-shell')
try {
    Invoke-Native 'cargo build' { cargo build --release }
} finally {
    Pop-Location
}

$setup = Join-Path $repo 'installer-shell\target\release\Brume-Setup.exe'
if (-not (Test-Path $setup)) { throw "Stage 2 produced no Brume-Setup.exe" }

$out = Join-Path $repo 'dist'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Copy-Item $setup (Join-Path $out 'Brume-Setup.exe') -Force

$final = Get-Item (Join-Path $out 'Brume-Setup.exe')
Write-Output ''
Write-Output ("=== Done.  dist\Brume-Setup.exe  ({0:N2} MB) ===" -f ($final.Length / 1MB))
Write-Output ''
