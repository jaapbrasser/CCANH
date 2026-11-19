#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Windows only, as administrator: silently install the installer with the
    service task, check the service answers on port 8080, then uninstall and
    check the service is gone. Used by CI.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Installer)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'installer-test.ps1 runs on Windows only' }
$setup = (Resolve-Path $Installer).Path
$dir = Join-Path ${env:ProgramFiles} 'CCANH'

$p = Start-Process $setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=service' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "install failed ($($p.ExitCode))" }
if (-not (Test-Path (Join-Path $dir 'crane.exe'))) { throw 'crane.exe not installed' }

$healthy = $false
for ($i = 0; $i -lt 60 -and -not $healthy; $i++) {
    try {
        $healthy = (Invoke-RestMethod 'http://127.0.0.1:8080/api/v1/health' -TimeoutSec 2).status -eq 'ok'
    } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $healthy) { throw 'the installed service never answered /api/v1/health' }
Write-Host 'installed service is healthy'

$p = Start-Process (Join-Path $dir 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "uninstall failed ($($p.ExitCode))" }
for ($i = 0; $i -lt 20 -and (Get-Service CraneCameraHub -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 500 }
if (Get-Service CraneCameraHub -ErrorAction SilentlyContinue) { throw 'the service is still installed' }
Write-Host 'uninstalled: OK'
