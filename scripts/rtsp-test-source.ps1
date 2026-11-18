#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Runs a local RTSP test source: MediaMTX plus ffmpeg publishing H.264 test
    patterns. Development only (SPEC §15.5); never needed at runtime.
.DESCRIPTION
    Publishes two profiles, like a camera's main and grid streams:
      rtsp://127.0.0.1:8554/test/main  1280x720 @ 25 fps, ~2 Mbit/s
      rtsp://127.0.0.1:8554/test/grid   640x360 @ 15 fps, ~400 kbit/s
    H.264 with a 2 s GOP and no B-frames (typical for AXIS defaults).
    Requires ffmpeg and mediamtx on PATH (macOS: brew install ffmpeg mediamtx;
    Windows: winget install Gyan.FFmpeg and the MediaMTX release zip).
    Ctrl+C stops both.
.PARAMETER Port
    RTSP port (default 8554).
#>
[CmdletBinding()]
param([int]$Port = 8554)

$ErrorActionPreference = 'Stop'
foreach ($tool in 'ffmpeg', 'mediamtx') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool not found on PATH" }
}

# MediaMTX writes generated certificates to its working directory, so run it
# from a temp folder rather than the repo.
$workDir = Join-Path ([IO.Path]::GetTempPath()) "crane-mediamtx-$PID"
New-Item -ItemType Directory -Force $workDir | Out-Null
$config = Join-Path $workDir 'mediamtx.yml'
@"
logLevel: warn
rtsp: yes
rtspAddress: 127.0.0.1:$Port
rtmp: no
hls: no
webrtc: no
srt: no
api: no
paths:
  all_others:
"@ | Set-Content -Path $config -Encoding utf8

$mtx = Start-Process mediamtx -ArgumentList $config -WorkingDirectory $workDir -PassThru -NoNewWindow
Start-Sleep -Seconds 1
try {
    $url = "rtsp://127.0.0.1:$Port/test"
    Write-Host "Publishing $url/main and $url/grid (Ctrl+C to stop)"
    ffmpeg -hide_banner -loglevel warning -re `
        -f lavfi -i "testsrc2=size=1280x720:rate=25" `
        -filter_complex "[0:v]split=2[m][g];[g]scale=640:360,fps=15[g2]" `
        -map "[m]" -c:v libx264 -preset veryfast -tune zerolatency -profile:v main `
            -bf 0 -g 50 -keyint_min 50 -sc_threshold 0 -b:v 2M -maxrate 2M -bufsize 2M `
            -f rtsp -rtsp_transport tcp "$url/main" `
        -map "[g2]" -c:v libx264 -preset veryfast -tune zerolatency -profile:v main `
            -bf 0 -g 30 -keyint_min 30 -sc_threshold 0 -b:v 400k -maxrate 400k -bufsize 400k `
            -f rtsp -rtsp_transport tcp "$url/grid"
} finally {
    if (-not $mtx.HasExited) { Stop-Process -Id $mtx.Id -Force }
    Remove-Item -Recurse -Force $workDir -ErrorAction SilentlyContinue
}
