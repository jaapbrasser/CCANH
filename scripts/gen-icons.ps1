#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Regenerates every application icon from frontend/public/favicon.svg.
.DESCRIPTION
    The SVG logo is the single source. This writes:
      frontend/public/favicon.ico            16/32/48 px, for browsers that want .ico
      frontend/public/icon-180.png           apple-touch-icon
      frontend/public/icon-192.png, -512.png web app manifest
      packaging/crane.ico                    16-256 px, embedded in crane.exe and
                                             used by the installer and shortcuts
    The outputs are committed, so builds never need ImageMagick. Run this only
    after changing the logo. Requires ImageMagick 7 (`magick`).
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $PSScriptRoot
$svg = Join-Path $root 'frontend/public/favicon.svg'
$public = Join-Path $root 'frontend/public'
$tmp = Join-Path ([IO.Path]::GetTempPath()) "ccanh-icons-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null

function Render([int]$size, [string]$out) {
    # Render large, then downscale: sharper small sizes than rendering small.
    magick -background none -density 1536 $svg -resize "${size}x${size}" -strip "PNG32:$out"
}

try {
    foreach ($s in 180, 192, 512) { Render $s (Join-Path $public "icon-$s.png") }

    $favicon = foreach ($s in 16, 32, 48) { $p = Join-Path $tmp "f$s.png"; Render $s $p; $p }
    magick @favicon (Join-Path $public 'favicon.ico')

    $app = foreach ($s in 16, 24, 32, 48, 64, 128, 256) { $p = Join-Path $tmp "a$s.png"; Render $s $p; $p }
    magick @app (Join-Path $root 'packaging/crane.ico')
    Write-Host 'Icons regenerated.'
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
