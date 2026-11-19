<#
.SYNOPSIS
    Rebuild the new CCANH offline package folder from the previous one plus
    this diff package. Works in Windows PowerShell 5.1; nothing is downloaded.
.DESCRIPTION
    1. Checks every file of the base folder (__BASE__) against base-SHA256SUMS.txt,
       so a damaged or edited base is caught before anything changes.
    2. Copies the base folder to the result folder (__RESULT__); the base folder
       itself is left untouched.
    3. Copies the changed and added files from .\files, removes deleted ones.
    4. Checks the result against its SHA256SUMS.txt: identical to the full package.
.PARAMETER Base
    The extracted previous package folder (the one containing SHA256SUMS.txt).
.PARAMETER Destination
    Where to create the new package folder. Default: next to the base folder.
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\Apply-Diff.ps1 -Base C:\ccanh\__BASE__
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Base,
    [string]$Destination
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Read-Sums([string]$file) {
    $map = @{}
    foreach ($line in Get-Content $file) {
        if ($line -match '^([0-9a-f]{64})\s+(.+)$') { $map[$Matches[2]] = $Matches[1] }
    }
    $map
}

function Test-Folder([string]$dir, [hashtable]$sums, [string]$what) {
    $bad = @()
    foreach ($path in $sums.Keys) {
        $file = Join-Path $dir $path
        if (-not (Test-Path $file)) { $bad += "missing: $path"; continue }
        $hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
        if ($hash -ne $sums[$path]) { $bad += "different: $path" }
    }
    if ($bad.Count) { throw "$what check failed:`n  $($bad -join "`n  ")" }
}

$Base = (Resolve-Path $Base).Path
if (-not $Destination) { $Destination = Join-Path (Split-Path $Base -Parent) '__RESULT__' }
if (Test-Path $Destination) { throw "$Destination already exists; remove it or pass -Destination" }

Write-Host "Checking the base folder $Base ..."
Test-Folder $Base (Read-Sums (Join-Path $here 'base-SHA256SUMS.txt')) 'Base folder'

Write-Host "Creating $Destination ..."
Copy-Item $Base $Destination -Recurse
foreach ($line in Get-Content (Join-Path $here 'DIFF-MANIFEST.txt')) {
    if ($line -match '^(changed|added)\s+[0-9a-f]{64}\s+(.+)$') {
        $rel = $Matches[2]
        $dest = Join-Path $Destination $rel
        New-Item -ItemType Directory -Force (Split-Path $dest -Parent) | Out-Null
        Copy-Item (Join-Path $here "files/$rel") $dest -Force
    } elseif ($line -match '^removed\s+(.+)$') {
        Remove-Item (Join-Path $Destination $Matches[1]) -Force
    }
}

Write-Host 'Checking the result ...'
Test-Folder $Destination (Read-Sums (Join-Path $Destination 'SHA256SUMS.txt')) 'Result'
Write-Host "OK: $Destination is identical to the full __RESULT__ package." -ForegroundColor Green
