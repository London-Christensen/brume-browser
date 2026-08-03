# Generates the installer artwork that NSIS needs.
#
# NSIS will only take .bmp for header and sidebar images - not SVG, not PNG - and
# the dimensions are fixed by MUI2. Both images are therefore composited here
# from the real brand rasters onto an Ink field, so they sit flush against
# the MUI_BGCOLOR set in installer.nsi rather than floating on a white square.
#
#   powershell tools/make-installer-art.ps1
#
# Output: src-tauri/installer/header.bmp, src-tauri/installer/sidebar.bmp

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$repo  = Split-Path -Parent $PSScriptRoot
$brand = Join-Path $repo 'brand\assets\png'
$out   = Join-Path $repo 'src-tauri\installer'

if (-not (Test-Path $out)) { New-Item -ItemType Directory -Force -Path $out | Out-Null }

# Ink, from brand/assets/css/tokens.css. Must match MUI_BGCOLOR in installer.nsi.
$ink = [System.Drawing.Color]::FromArgb(255, 0x10, 0x14, 0x18)

function New-Composite {
    param(
        [int]$Width,
        [int]$Height,
        [string]$Source,     # PNG to place, already in the Paper colourway
        [int]$TargetWidth,   # width to scale the source to
        [string]$Align,      # 'right' or 'center'
        [string]$OutFile
    )

    if (-not (Test-Path $Source)) { throw "missing brand asset: $Source" }

    # 24bpp: NSIS is happiest with a plain RGB bitmap and no alpha channel.
    $bmp = New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g   = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode     = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode   = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    $g.Clear($ink)

    $src = [System.Drawing.Image]::FromFile($Source)
    try {
        $scale  = $TargetWidth / $src.Width
        $tw     = [int]$TargetWidth
        $th     = [int][math]::Round($src.Height * $scale)

        switch ($Align) {
            'right'  { $x = $Width - $tw - [int]($Height * 0.24) }
            default  { $x = [int](($Width - $tw) / 2) }
        }
        $y = [int](($Height - $th) / 2)

        $g.DrawImage($src, $x, $y, $tw, $th)
    } finally {
        $src.Dispose()
    }

    $bmp.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $g.Dispose(); $bmp.Dispose()
    Write-Output ("{0}  ({1}x{2})" -f (Split-Path $OutFile -Leaf), $Width, $Height)
}

# Header strip, shown top-right on every page. MUI2 fixes this at 150x57.
New-Composite -Width 150 -Height 57 `
    -Source (Join-Path $brand 'mark-paper-128.png') `
    -TargetWidth 34 -Align 'right' `
    -OutFile (Join-Path $out 'header.bmp')

# Sidebar, shown on the finish page. MUI2 fixes this at 164x314.
New-Composite -Width 164 -Height 314 `
    -Source (Join-Path $brand 'lockup-v-paper-264.png') `
    -TargetWidth 104 -Align 'center' `
    -OutFile (Join-Path $out 'sidebar.bmp')
