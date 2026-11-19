#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Windows only: check that crane.exe only imports DLLs that ship with Windows
    itself, so it runs on a clean Windows 11 with nothing installed (no Visual
    C++ Redistributable, no .NET, no internet). Used by CI.
.PARAMETER Exe
    Path to crane.exe.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Exe)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'check-windows-deps.ps1 runs on Windows only' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$dumpbin = & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin.exe not found (needs the MSVC build tools)' }

$out = & $dumpbin /nologo /dependents (Resolve-Path $Exe).Path
$dlls = $out | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() }
if (-not $dlls) { throw "no imports found in dumpbin output:`n$out" }

# Redistributable runtimes a clean Windows doesn't have.
$redist = '^(vcruntime|msvcp|vccorlib|concrt|ucrtbased|msvcr)'
$bad = @()
foreach ($dll in $dlls) {
    if ($dll -match $redist) { $bad += "$dll (Visual C++ runtime: link statically)"; continue }
    # API sets (api-ms-win-*) are resolved by the OS loader itself.
    if ($dll -like 'api-ms-win-*') { continue }
    if (-not (Test-Path (Join-Path $env:SystemRoot "System32\$dll"))) { $bad += "$dll (not part of Windows)" }
}
Write-Host "crane.exe imports: $($dlls -join ', ')"
if ($bad) { throw "crane.exe depends on DLLs a clean Windows 11 lacks:`n  $($bad -join "`n  ")" }
Write-Host 'OK: only Windows system DLLs'
