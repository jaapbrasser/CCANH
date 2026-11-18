#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Generates the bundled demo video clips (SPEC §12.3) in assets/demo/.
.DESCRIPTION
    Short looping H.264 clips made from ffmpeg's built-in test sources, so there
    are no licensing questions. Each pattern gets a "main" (1280x720 @ 25 fps)
    and a "grid" (640x360 @ 15 fps) version, matching the demo cameras' stream
    profiles.

    Format: raw H.264 Annex-B elementary streams with access unit delimiters
    (one AUD per frame), Main profile, no B-frames, 2 s GOP. The server embeds
    them and replays them in real time (backend/src/streaming/demo.rs); the
    frame rate and size come from clips.json.

    If ffmpeg has the drawtext filter, a timestamp is burned in; otherwise the
    clips are plain patterns (the UI shows the camera name).

    ffmpeg is a development tool only; it's never needed at runtime. Rerun this
    script only when the clips should change, then commit assets/demo/.
#>
[CmdletBinding()]
param([int]$Seconds = 6)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'assets/demo'
New-Item -ItemType Directory -Force $out | Out-Null

$hasDrawtext = (ffmpeg -hide_banner -filters 2>$null | Select-String ' drawtext ') -ne $null

$patterns = [ordered]@{
    'testsrc2'   = 'testsrc2=size={0}:rate={1}'
    'mandelbrot' = 'mandelbrot=size={0}:rate={1}:end_scale=0.1'
    'gradients'  = 'gradients=size={0}:rate={1}:speed=0.03:n=4'
    'bars'       = 'smptehdbars=size={0}:rate={1}'
}
$profiles = @(
    @{ name = 'main'; size = '1280x720'; width = 1280; height = 720; fps = 25; bitrate = '700k' },
    @{ name = 'grid'; size = '640x360';  width = 640;  height = 360; fps = 15; bitrate = '200k' }
)

$manifest = @()
foreach ($p in $patterns.GetEnumerator()) {
    foreach ($prof in $profiles) {
        $file = "$($p.Key)-$($prof.name).h264"
        $source = [string]::Format($p.Value, $prof.size, $prof.fps)
        $vf = if ($hasDrawtext) { "drawtext=text='%{pts\:hms}':x=16:y=16:fontsize=24:fontcolor=white:box=1:boxcolor=black@0.5" } else { 'null' }
        $gop = 2 * $prof.fps
        ffmpeg -hide_banner -loglevel error -y `
            -f lavfi -i $source -t $Seconds -vf $vf `
            -c:v libx264 -profile:v main -pix_fmt yuv420p -preset slow -bf 0 `
            -b:v $prof.bitrate -maxrate $prof.bitrate -bufsize $prof.bitrate `
            -x264-params "aud=1:keyint=$($gop):min-keyint=$($gop):scenecut=0" `
            -f h264 (Join-Path $out $file)
        $manifest += [ordered]@{
            pattern = $p.Key; profile = $prof.name; file = $file
            width = $prof.width; height = $prof.height; fps = $prof.fps
        }
        Write-Host "  $file ($([math]::Round((Get-Item (Join-Path $out $file)).Length / 1KB)) KB)"
    }
}
$manifest | ConvertTo-Json | Set-Content -Path (Join-Path $out 'clips.json') -Encoding utf8
$total = (Get-ChildItem $out -Filter *.h264 | Measure-Object Length -Sum).Sum
Write-Host "Wrote $($manifest.Count) clips, $([math]::Round($total / 1MB, 2)) MB total"
