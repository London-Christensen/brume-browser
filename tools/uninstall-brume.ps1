# Removes Brume from this machine, including from a half-installed state.
#
# This is not a replacement for the real uninstaller. A healthy install should be
# removed through Settings > Installed apps, which runs `uninstall.exe` and does
# the job properly. This script exists for the cases that path cannot reach:
#
#   * `uninstall.exe` is missing, or its directory no longer exists
#   * the registry entry is gone, so Settings never lists Brume at all
#   * the install landed somewhere unexpected - notably when the installer was
#     run inside a sandboxed process, which redirects %LOCALAPPDATA% and leaves
#     the shortcut and the registry pointing at two different places
#   * a scratch install from testing needs clearing out
#
# It also removes one directory the real uninstaller misses. The uninstaller
# deletes `%LOCALAPPDATA%\<bundle id>`, but the installer shell is a separate
# Tauri app whose id is `<bundle id>.setup`, so its WebView2 profile is an exact
# path the uninstaller never names and is orphaned on every uninstall.
#
#   pwsh tools/uninstall-brume.ps1                 # report only, changes nothing
#   pwsh tools/uninstall-brume.ps1 -Run            # remove the app, keep browsing data
#   pwsh tools/uninstall-brume.ps1 -Run -IncludeData   # remove everything
#
# Reporting is the default on purpose: this deletes bookmarks and history when
# asked to, and a tool that does that on a bare invocation is a tool that
# eventually does it by accident.

[CmdletBinding()]
param(
    # Perform the removal. Without this the script only reports what it found.
    [switch]$Run,

    # Also delete settings, history, bookmarks and the WebView2 profiles
    # (cookies, cache, site data). Matches the "Delete application data"
    # checkbox on the real uninstaller's confirm page, which is likewise opt-in.
    [switch]$IncludeData
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot

# The bundle identifier decides where the data directories live, so it is read
# from the config rather than duplicated here. Falls back to the known value so
# the script still works when copied to a machine without the repo.
$bundleId = 'com.londonchristensen.brume'
$conf = Join-Path $repo 'src-tauri\tauri.conf.json'
if (Test-Path $conf) {
    try {
        $parsed = Get-Content $conf -Raw | ConvertFrom-Json
        if ($parsed.identifier) { $bundleId = $parsed.identifier }
    } catch {
        Write-Warning "Could not read $conf; assuming bundle id '$bundleId'."
    }
}

$product = 'Brume'
$uninstKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$manuKey = 'Software\London Christensen'

# Everything found, as {Kind, Path, Note}. Collected first and acted on second,
# so the report and the removal cannot describe different things.
$plan = New-Object System.Collections.ArrayList
function Add-Target($kind, $path, $note) {
    [void]$plan.Add([pscustomobject]@{ Kind = $kind; Path = $path; Note = $note })
}

# --- running process -------------------------------------------------------
# A running Brume holds its own exe open, so this has to go first or the
# directory removal fails halfway and leaves a partial install behind.
$running = @(Get-Process -Name 'brume' -ErrorAction SilentlyContinue)
foreach ($p in $running) {
    Add-Target 'process' "brume.exe (pid $($p.Id))" 'running - will be stopped'
}

# --- install directories ---------------------------------------------------
# Discovered from the registry rather than assumed, because the install location
# is user-selectable on the installer's directory page.
$dirs = New-Object System.Collections.Generic.List[string]
foreach ($hive in 'HKCU:', 'HKLM:') {
    $full = "$hive\$uninstKey"
    $props = Get-ItemProperty -Path $full -ErrorAction SilentlyContinue
    if ($props) {
        Add-Target 'registry' $full "DisplayVersion $($props.DisplayVersion)"
        if ($props.InstallLocation) {
            # NSIS writes this value quoted; the quotes are part of the string
            # and have to come off before it can be used as a path.
            $loc = $props.InstallLocation.Trim('"')
            if ($loc) { $dirs.Add($loc) }
        }
    }
}
# The default location, in case the registry entry is the thing that is missing.
$dirs.Add((Join-Path $env:LOCALAPPDATA $product))

foreach ($d in ($dirs | Select-Object -Unique)) {
    if (Test-Path $d) {
        $size = (Get-ChildItem $d -Recurse -File -ErrorAction SilentlyContinue |
                 Measure-Object -Property Length -Sum).Sum
        Add-Target 'directory' $d "$([math]::Round($size / 1MB, 1)) MB"
    }
}

# --- shortcuts -------------------------------------------------------------
# Both Start Menu roots and both desktops, because a per-machine install writes
# to the all-users locations and a per-user install to the current user's.
$shortcuts = @(
    (Join-Path $env:APPDATA      "Microsoft\Windows\Start Menu\Programs\$product.lnk")
    (Join-Path $env:ProgramData  "Microsoft\Windows\Start Menu\Programs\$product.lnk")
    (Join-Path $env:USERPROFILE  "Desktop\$product.lnk")
    (Join-Path $env:PUBLIC       "Desktop\$product.lnk")
)
$shell = New-Object -ComObject WScript.Shell
foreach ($s in $shortcuts) {
    if (Test-Path $s) {
        $target = ''
        try { $target = $shell.CreateShortcut($s).TargetPath } catch { $target = '<unreadable>' }
        # The target is reported rather than used as a filter. In the broken
        # case it is precisely the thing that points somewhere unexpected, so
        # matching on it would skip exactly the shortcuts worth removing.
        Add-Target 'shortcut' $s "-> $target"
    }
}

# --- registry --------------------------------------------------------------
# Where the installer records the auto-update choice, read by settings.rs on
# first run. Left behind, it silently seeds a preference into a later reinstall.
$manuProduct = "HKCU:\$manuKey\$product"
if (Test-Path $manuProduct) {
    Add-Target 'registry' $manuProduct 'installer auto-update preference'
}

# Autostart, if a build ever set it. Harmless when absent.
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runVal = Get-ItemProperty -Path $runKey -Name $product -ErrorAction SilentlyContinue
if ($runVal) { Add-Target 'registry-value' "$runKey\$product" 'autostart entry' }

# --- application data ------------------------------------------------------
# Opt-in. The two `.setup` entries are the installer shell's own WebView2
# profile - see the header for why the real uninstaller never removes them.
$dataDirs = @(
    (Join-Path $env:APPDATA      $bundleId)
    (Join-Path $env:LOCALAPPDATA $bundleId)
    (Join-Path $env:LOCALAPPDATA "$bundleId.setup")
)
$kept = New-Object System.Collections.ArrayList
foreach ($d in $dataDirs) {
    if (Test-Path $d) {
        $size = (Get-ChildItem $d -Recurse -File -ErrorAction SilentlyContinue |
                 Measure-Object -Property Length -Sum).Sum
        $mb = [math]::Round($size / 1MB, 1)
        if ($IncludeData) {
            Add-Target 'data' $d "$mb MB"
        } else {
            [void]$kept.Add("$d ($mb MB)")
        }
    }
}

# --- report ----------------------------------------------------------------
if ($plan.Count -eq 0) {
    Write-Output ''
    Write-Output "No trace of $product found on this machine."
    return
}

Write-Output ''
Write-Output "Found $($plan.Count) item(s):"
Write-Output ''
foreach ($item in $plan) {
    Write-Output ("  {0,-14} {1}" -f $item.Kind, $item.Path)
    if ($item.Note) { Write-Output ("  {0,-14} {1}" -f '', $item.Note) }
}

if ($kept.Count -gt 0) {
    Write-Output ''
    Write-Output 'Keeping (add -IncludeData to remove):'
    Write-Output ''
    foreach ($k in $kept) { Write-Output "  $k" }
}
Write-Output ''

if (-not $Run) {
    Write-Output 'Nothing was changed. To remove what is listed above:'
    Write-Output ''
    Write-Output '    pwsh tools/uninstall-brume.ps1 -Run'
    return
}

# --- removal ---------------------------------------------------------------
# Each item is removed independently. One failure - a locked file, a key needing
# elevation - should not abandon the rest, so the loop reports and continues
# rather than throwing on the first problem.
$removed = 0
$failed = 0

foreach ($item in $plan) {
    try {
        # The verb is set per branch rather than by an early exit: `continue`
        # inside a PowerShell switch applies to the switch, not to the enclosing
        # foreach, so using it here would quietly skip the tally below.
        $verb = 'removed'
        switch ($item.Kind) {
            'process' {
                Get-Process -Name 'brume' -ErrorAction SilentlyContinue | Stop-Process -Force
                # Stop-Process returns before the handles are actually released,
                # and the directory removal below needs them gone.
                Start-Sleep -Milliseconds 500
                $verb = 'stopped'
            }
            'registry-value' {
                Remove-ItemProperty -Path $runKey -Name $product -Force
            }
            default {
                Remove-Item -Path $item.Path -Recurse -Force
            }
        }
        Write-Output ("  {0,-9} {1}" -f $verb, $item.Path)
        $removed++
    } catch {
        Write-Warning "  FAILED    $($item.Path) - $($_.Exception.Message)"
        $failed++
    }
}

# The manufacturer key is shared with any other product from the same publisher,
# so it goes only when nothing is left under it.
$manuFull = "HKCU:\$manuKey"
if (Test-Path $manuFull) {
    $leftovers = @(Get-ChildItem $manuFull -ErrorAction SilentlyContinue)
    if ($leftovers.Count -eq 0) {
        Remove-Item -Path $manuFull -Force
        Write-Output "  removed   $manuFull (now empty)"
        $removed++
    }
}

Write-Output ''
Write-Output "Removed $removed item(s), $failed failure(s)."
if ($failed -gt 0) {
    Write-Output 'Some items need an elevated prompt, or are still locked by a running process.'
    exit 1
}
