#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Assemble the offline deployment package (zip) from a release build and
    the installer: everything needed on a Windows 11 machine without internet.
    Run scripts/build.ps1 and scripts/build-installer.ps1 first (Windows).
.OUTPUTS
    dist/ccanh-offline-<version>.zip
#>
[CmdletBinding()]
param(
    [string]$Exe = 'target/release/crane.exe'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$exePath = (Resolve-Path (Join-Path $root $Exe)).Path
$installer = Join-Path $root "dist/crane-setup-$version.exe"
if (-not (Test-Path $installer)) { throw "installer not found: $installer (run scripts/build-installer.ps1)" }

$name = "ccanh-offline-$version"
$stage = Join-Path $root "dist/$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'portable'), (Join-Path $stage 'tools') | Out-Null

Copy-Item $installer $stage
Copy-Item $exePath (Join-Path $stage 'portable/crane.exe')
Copy-Item (Join-Path $root 'config.example.toml') (Join-Path $stage 'portable')
Copy-Item (Join-Path $root 'packaging/offline/README.txt') $stage
Copy-Item (Join-Path $root 'packaging/offline/TEST-PLAN.html') $stage
Copy-Item (Join-Path $root 'packaging/offline/tools/*.ps1') (Join-Path $stage 'tools')

$commit = (git -C $root rev-parse --short HEAD 2>$null)
@(
    "CCANH offline package $version"
    "Commit: $commit"
    "Built: $((Get-Date).ToUniversalTime().ToString('u'))"
    "Target: Windows 10/11 x64, no prerequisites (static C runtime, embedded web UI)"
) | Set-Content (Join-Path $stage 'BUILD-INFO.txt') -Encoding UTF8

# Windows-style line endings for files people open in Notepad.
foreach ($f in 'README.txt', 'BUILD-INFO.txt', 'portable/config.example.toml') {
    $p = Join-Path $stage $f
    $text = [IO.File]::ReadAllText($p) -replace "`r?`n", "`r`n"
    [IO.File]::WriteAllText($p, $text, [Text.UTF8Encoding]::new($false))
}

$sums = Get-ChildItem $stage -Recurse -File | Sort-Object FullName | ForEach-Object {
    $rel = $_.FullName.Substring($stage.Length + 1) -replace '\\', '/'
    "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $rel
}
$sums | Set-Content (Join-Path $stage 'SHA256SUMS.txt') -Encoding ASCII

$zip = Join-Path $root "dist/$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path $stage -DestinationPath $zip
$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$zipHash  $name.zip" | Set-Content (Join-Path $root "dist/$name.zip.sha256") -Encoding ASCII
Write-Host "Built $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB), SHA-256 $zipHash"
