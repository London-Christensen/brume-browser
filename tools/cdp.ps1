# Drives a running Brume over the WebView2 DevTools Protocol.
#
# Why this exists
# ---------------
# The obvious way to test a desktop app is to synthesise mouse and keyboard
# input. It also makes the machine unusable while the tests run: SetForegroundWindow
# steals focus mid-keystroke, SetCursorPos moves the real pointer, and SendKeys
# types into whatever happens to be focused if the timing slips.
#
# WebView2 exposes the same DevTools protocol Chrome does. Talking to that over a
# socket drives the app precisely, needs no focus, moves nothing on screen, and
# works while the window is behind other windows or positioned off-screen.
#
# Usage
# -----
#   . tools/cdp.ps1
#   Start-BrumeDebug                       # launch with the debug port open
#   Invoke-BrumeJs 'document.title'        # evaluate in the chrome webview
#   Invoke-BrumeJs -Target content 'location.href'
#   Get-BrumeShot -Target content -Out page.png
#   Stop-BrumeDebug
#
# The debug port is opt-in and never enabled in a shipped build - it is passed as
# an environment variable at launch, not compiled in.

$script:BrumePort = 9222

function Start-BrumeDebug {
    [CmdletBinding()]
    param(
        [string]$Exe = 'D:\Brume\src-tauri\target\debug\brume.exe',
        [int]$Port = 9222,
        # Parks the window off-screen so automated runs never flash into view.
        # The webviews still render and screenshot normally.
        [switch]$OffScreen,
        [int]$TimeoutSeconds = 30
    )

    $script:BrumePort = $Port
    Stop-Process -Name brume -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 600

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
    Start-Process $Exe

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 500
        $targets = Get-BrumeTargets -Quiet
    } while (-not $targets -and (Get-Date) -lt $deadline)

    if (-not $targets) { throw "Brume did not expose a debug port on $Port within ${TimeoutSeconds}s" }

    if ($OffScreen) { Move-BrumeOffScreen }
    Write-Output "Brume running with CDP on port $Port ($($targets.Count) targets)"
}

function Stop-BrumeDebug {
    Stop-Process -Name brume -Force -ErrorAction SilentlyContinue
    Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
}

function Get-BrumeTargets {
    param([switch]$Quiet)
    try {
        # Invoke-WebRequest + explicit ConvertFrom-Json, wrapped in @(), rather
        # than Invoke-RestMethod.
        #
        # Invoke-RestMethod can hand a JSON array back as a *single* pipeline
        # item. Filtering that with Where-Object { $_.type -eq 'page' } then
        # triggers PowerShell member enumeration: $_.type evaluates to an array
        # of every target's type, -eq returns the matching elements, and a
        # non-empty array is truthy - so the filter matches the whole blob and
        # silently does nothing. The symptom is a "target" that is really every
        # target at once, which only surfaces later as a confusing type error.
        $response = Invoke-WebRequest -Uri "http://127.0.0.1:$($script:BrumePort)/json/list" `
                                      -UseBasicParsing -TimeoutSec 5

        # -UseBasicParsing hands back a string or a byte[] depending on the
        # response content type, and guessing wrong throws rather than degrading.
        $body = if ($response.Content -is [byte[]]) {
            [System.Text.Encoding]::UTF8.GetString($response.Content)
        } else {
            [string]$response.Content
        }

        # Enumerate with an explicit loop rather than wrapping in @().
        #
        # ConvertFrom-Json on PowerShell 5.1 emits a JSON array as a *single*
        # pipeline object, and @() around that just wraps the blob again - it
        # does not flatten it. foreach handles both shapes correctly: an array
        # enumerates, a lone object yields one item.
        $parsed = ConvertFrom-Json $body
        $out = @()
        foreach ($item in $parsed) { $out += $item }
        $out
    } catch {
        if (-not $Quiet) { Write-Warning "CDP not reachable on $($script:BrumePort): $($_.Exception.Message)" }
        @()
    }
}

# The chrome webview is served from tauri.localhost; anything else is page content.
#
# Brume's new tab page is the exception that makes this fiddly: it is a *content*
# webview, but it is served from tauri.localhost too, so matching the host alone
# picks it as the chrome and silently drives the wrong webview. Every test that
# touches the chrome would then be reading a page with no toolbar in it.
#
# The chrome is therefore matched on the root path exactly, and the new tab page
# is treated as content, which is what it is.
function Resolve-BrumeTarget {
    param([ValidateSet('chrome', 'content')][string]$Target = 'chrome')

    $pages = @(Get-BrumeTargets) | Where-Object { $_.type -eq 'page' }
    $pages = @($pages)
    if ($pages.Count -eq 0) {
        throw 'No CDP page targets - is Brume running with the debug port open?'
    }

    $isChrome = { param($u) $u -match '^https?://tauri\.localhost/?(\?.*)?$' }

    $found = @(
        if ($Target -eq 'chrome') {
            $pages | Where-Object { & $isChrome $_.url }
        } else {
            $pages | Where-Object { -not (& $isChrome $_.url) }
        }
    )

    if ($found.Count -eq 0) {
        throw "No '$Target' target. Saw: $(($pages | ForEach-Object { $_.url }) -join ', ')"
    }

    $one = $found[0]
    # Guard the exact failure described above: if this is ever not a single
    # target with a single socket URL, fail here with something readable rather
    # than downstream in a parameter binder.
    if ($one.webSocketDebuggerUrl -isnot [string]) {
        throw "Expected one '$Target' target, resolved $($found.Count). This usually means the target list was not enumerated properly."
    }

    $one
}

# --- minimal CDP client ------------------------------------------------------
# One command per connection. Slower than holding a socket open, but each call is
# independent, which matters far more than throughput for a test harness.
function Send-CdpCommand {
    param(
        [Parameter(Mandatory = $true)][string]$WebSocketUrl,
        [Parameter(Mandatory = $true)][string]$Method,
        [hashtable]$Params = @{},
        [int]$TimeoutSeconds = 30
    )

    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $cts = New-Object System.Threading.CancellationTokenSource
    $cts.CancelAfter([TimeSpan]::FromSeconds($TimeoutSeconds))
    $ct = $cts.Token

    try {
        $ws.ConnectAsync([Uri]$WebSocketUrl, $ct).GetAwaiter().GetResult()

        $payload = @{ id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 12 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
        $ws.SendAsync(
            (New-Object System.ArraySegment[byte] -ArgumentList @(, $bytes)),
            [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct
        ).GetAwaiter().GetResult()

        # Responses can arrive in several frames, and events for other ids are
        # interleaved with the reply, so keep reading until id 1 comes back.
        $sb = New-Object System.Text.StringBuilder
        $buffer = New-Object byte[] 131072
        $segment = New-Object System.ArraySegment[byte] -ArgumentList @(, $buffer)

        while ($true) {
            $result = $ws.ReceiveAsync($segment, $ct).GetAwaiter().GetResult()
            [void]$sb.Append([Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count))
            if (-not $result.EndOfMessage) { continue }

            $text = $sb.ToString()
            [void]$sb.Clear()

            $obj = $text | ConvertFrom-Json
            if ($obj.PSObject.Properties.Name -contains 'id' -and $obj.id -eq 1) { return $obj }
            # Otherwise it was an event; keep waiting for the reply.
        }
    } finally {
        try { $ws.Dispose() } catch {}
        $cts.Dispose()
    }
}

<#
.SYNOPSIS
Evaluates JavaScript inside one of Brume's webviews.
#>
function Invoke-BrumeJs {
    param(
        [Parameter(Mandatory = $true, Position = 0)][string]$Script,
        [ValidateSet('chrome', 'content')][string]$Target = 'chrome',
        [int]$TimeoutSeconds = 30
    )

    $t = Resolve-BrumeTarget -Target $Target
    $reply = Send-CdpCommand -WebSocketUrl $t.webSocketDebuggerUrl -Method 'Runtime.evaluate' -Params @{
        expression    = $Script
        returnByValue = $true
        awaitPromise  = $true
    } -TimeoutSeconds $TimeoutSeconds

    if ($reply.result.exceptionDetails) {
        throw "JS error: $($reply.result.exceptionDetails.exception.description)"
    }
    $reply.result.result.value
}

<#
.SYNOPSIS
Captures a webview to a PNG without focusing or raising the window.
#>
function Get-BrumeShot {
    param(
        [Parameter(Mandatory = $true)][string]$Out,
        [ValidateSet('chrome', 'content')][string]$Target = 'content'
    )

    $t = Resolve-BrumeTarget -Target $Target
    $reply = Send-CdpCommand -WebSocketUrl $t.webSocketDebuggerUrl -Method 'Page.captureScreenshot' -Params @{
        format = 'png'
    }

    if (-not $reply.result.data) { throw 'No screenshot data returned' }
    [IO.File]::WriteAllBytes($Out, [Convert]::FromBase64String($reply.result.data))
    Write-Output "captured $Target -> $Out"
}

<#
.SYNOPSIS
Moves the Brume window off the visible desktop.

Rendering and screenshots are unaffected - this only stops automated runs from
appearing on screen while you are doing something else.
#>
function Move-BrumeOffScreen {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public class BrumeWin {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
"@ -ErrorAction SilentlyContinue

    $p = Get-Process brume -ErrorAction SilentlyContinue |
         Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if (-not $p) { return }

    # SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE - move only, never raise or focus.
    [BrumeWin]::SetWindowPos($p.MainWindowHandle, [IntPtr]::Zero, -4000, -4000, 0, 0, 0x0001 -bor 0x0004 -bor 0x0010) | Out-Null
}
