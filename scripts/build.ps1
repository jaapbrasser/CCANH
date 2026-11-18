#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Builds the release binary with the frontend embedded.
.DESCRIPTION
    1. npm ci && npm run build in frontend/ (produces frontend/dist)
    2. cargo build --release, which embeds frontend/dist via rust-embed
    Output: target/release/crane(.exe)
.PARAMETER SkipFrontend
    Reuse an existing frontend/dist instead of rebuilding it.
#>
[CmdletBinding()]
param([switch]$SkipFrontend)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $PSScriptRoot

if (-not $SkipFrontend) {
    Push-Location (Join-Path $root 'frontend')
    try {
        npm ci
        npm run build
    } finally { Pop-Location }
}

if (-not (Test-Path (Join-Path $root 'frontend/dist/index.html'))) {
    throw 'frontend/dist/index.html is missing; the release binary would only contain the placeholder page.'
}

Push-Location $root
try {
    cargo build --release -p crane
} finally { Pop-Location }

$exe = if ($IsWindows) { 'crane.exe' } else { 'crane' }
$out = Join-Path $root "target/release/$exe"
Write-Host "Built $out ($([math]::Round((Get-Item $out).Length / 1MB, 1)) MB)"
