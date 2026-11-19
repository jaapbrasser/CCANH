#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Build the Windows installer (Inno Setup 6) from target\release\crane.exe.
    Run scripts/build.ps1 first. Output: dist\crane-setup-<version>.exe.
.PARAMETER Exe
    The binary to package (default target\release\crane.exe).
#>
[CmdletBinding()]
param([string]$Exe = 'target/release/crane.exe')

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'build-installer.ps1 runs on Windows only' }
$root = Split-Path $PSScriptRoot -Parent
$exePath = (Resolve-Path (Join-Path $root $Exe)).Path

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $candidate = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
    if (Test-Path $candidate) { $iscc = $candidate }
}
if (-not $iscc) { throw 'Inno Setup 6 (ISCC.exe) not found; install it with: choco install innosetup' }

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
& $iscc "/DAppVersion=$version" "/DSourceExe=$exePath" (Join-Path $root 'packaging/crane.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
$out = Join-Path $root "dist/crane-setup-$version.exe"
Write-Host "Built $out"
