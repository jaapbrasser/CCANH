#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Build a diff package: only the files that changed between two offline
    packages, plus a script that rebuilds the full new package on the target.
.DESCRIPTION
    Compares the SHA256SUMS.txt of the old and new offline zips
    (scripts/package-offline.ps1) and packs the changed and added files with:
      DIFF-MANIFEST.txt     what changed, was added or removed (with hashes)
      base-SHA256SUMS.txt   the old package's checksums (checked before applying)
      Apply-Diff.ps1        rebuilds the new package folder from the old one
      README-DIFF.txt       how to use it, and what to copy for portable-only use
    Nothing is downloaded and the target needs nothing but Windows PowerShell.
.EXAMPLE
    pwsh scripts/package-diff.ps1 -Old dist/0.1.0/ccanh-offline-0.1.0.zip `
        -New dist/0.2.0/full/ccanh-offline-0.2.0.zip -OutDir dist/0.2.0/diff
.OUTPUTS
    <OutDir>/ccanh-offline-<new>-diff-from-<old>.zip and .zip.sha256
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Old,
    [Parameter(Mandatory)][string]$New,
    [Parameter(Mandatory)][string]$OutDir
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$work = Join-Path ([IO.Path]::GetTempPath()) "ccanh-diff-$PID"
New-Item -ItemType Directory -Force $work | Out-Null

function Expand-Package([string]$zip, [string]$into) {
    Expand-Archive -Path $zip -DestinationPath $into -Force
    $dir = Get-ChildItem $into -Directory | Select-Object -First 1
    if (-not $dir -or -not (Test-Path (Join-Path $dir.FullName 'SHA256SUMS.txt'))) {
        throw "$zip is not an offline package (no SHA256SUMS.txt)"
    }
    $dir.FullName
}

function Read-Sums([string]$file) {
    $map = [ordered]@{}
    foreach ($line in Get-Content $file) {
        if ($line -match '^([0-9a-f]{64})\s+(.+)$') { $map[$Matches[2]] = $Matches[1] }
    }
    $map
}

try {
    $oldDir = Expand-Package (Resolve-Path $Old).Path (Join-Path $work 'old')
    $newDir = Expand-Package (Resolve-Path $New).Path (Join-Path $work 'new')
    $oldName = Split-Path $oldDir -Leaf
    $newName = Split-Path $newDir -Leaf
    $oldVersion = $oldName -replace '^ccanh-offline-', ''
    $newVersion = $newName -replace '^ccanh-offline-', ''

    $oldSums = Read-Sums (Join-Path $oldDir 'SHA256SUMS.txt')
    $newSums = Read-Sums (Join-Path $newDir 'SHA256SUMS.txt')
    # SHA256SUMS.txt doesn't list itself; it always changes with the files.
    $newSums['SHA256SUMS.txt'] = (Get-FileHash (Join-Path $newDir 'SHA256SUMS.txt') -Algorithm SHA256).Hash.ToLower()
    $oldSums['SHA256SUMS.txt'] = (Get-FileHash (Join-Path $oldDir 'SHA256SUMS.txt') -Algorithm SHA256).Hash.ToLower()

    $changed = @(); $added = @(); $removed = @(); $same = @()
    foreach ($path in $newSums.Keys) {
        if (-not $oldSums.Contains($path)) { $added += $path }
        elseif ($oldSums[$path] -ne $newSums[$path]) { $changed += $path }
        else { $same += $path }
    }
    foreach ($path in $oldSums.Keys) { if (-not $newSums.Contains($path)) { $removed += $path } }

    $name = "ccanh-offline-$newVersion-diff-from-$oldVersion"
    $stage = Join-Path $work $name
    New-Item -ItemType Directory -Force (Join-Path $stage 'files') | Out-Null
    foreach ($path in $changed + $added) {
        $dest = Join-Path $stage "files/$path"
        New-Item -ItemType Directory -Force (Split-Path $dest -Parent) | Out-Null
        Copy-Item (Join-Path $newDir $path) $dest
    }
    Copy-Item (Join-Path $oldDir 'SHA256SUMS.txt') (Join-Path $stage 'base-SHA256SUMS.txt')

    $manifest = @(
        "CCANH diff package: $oldVersion -> $newVersion"
        "Base folder:   $oldName"
        "Result folder: $newName"
        ''
    )
    $manifest += $changed | ForEach-Object { "changed  $($newSums[$_])  $_" }
    $manifest += $added | ForEach-Object { "added    $($newSums[$_])  $_" }
    $manifest += $removed | ForEach-Object { "removed  $_" }
    $manifest += $same | ForEach-Object { "same     $($newSums[$_])  $_" }
    $manifest | Set-Content (Join-Path $stage 'DIFF-MANIFEST.txt') -Encoding ASCII

    $apply = Get-Content (Join-Path $root 'packaging/offline/diff/Apply-Diff.ps1') -Raw
    $apply = $apply.Replace('__BASE__', $oldName).Replace('__RESULT__', $newName)
    Set-Content (Join-Path $stage 'Apply-Diff.ps1') $apply -Encoding UTF8 -NoNewline

    $readme = Get-Content (Join-Path $root 'packaging/offline/diff/README-DIFF.txt') -Raw
    $list = (($changed + $added) | ForEach-Object { "  files\$($_ -replace '/', '\')" }) -join "`r`n"
    $readme = $readme.Replace('__OLD__', $oldVersion).Replace('__NEW__', $newVersion).
        Replace('__BASE__', $oldName).Replace('__RESULT__', $newName).Replace('__FILES__', $list)
    $readme = $readme -replace "`r?`n", "`r`n"
    [IO.File]::WriteAllText((Join-Path $stage 'README-DIFF.txt'), $readme, [Text.UTF8Encoding]::new($false))

    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $zip = Join-Path (Resolve-Path $OutDir).Path "$name.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path $stage -DestinationPath $zip
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    "$hash  $name.zip" | Set-Content "$zip.sha256" -Encoding ASCII
    $mb = { param($p) [math]::Round((Get-Item $p).Length / 1MB, 1) }
    Write-Host "Built $zip ($(& $mb $zip) MB vs $(& $mb $New) MB full): $($changed.Count) changed, $($added.Count) added, $($removed.Count) removed, $($same.Count) unchanged"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
