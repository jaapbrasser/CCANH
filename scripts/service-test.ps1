#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Windows only, as administrator: install crane as a service, start it,
    check /api/v1/health, stop it gracefully and uninstall it (ADR 0010).
    Used by CI; safe to run locally (uses demo mode and a temp data dir).
.PARAMETER Exe
    Path to crane.exe.
.PARAMETER Port
    Port for the test service (default 18090).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [int]$Port = 18090
)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'service-test.ps1 runs on Windows only' }
$exe = (Resolve-Path $Exe).Path
$name = 'CraneCameraHub'
$data = Join-Path ([IO.Path]::GetTempPath()) "crane-service-test-$PID"

& $exe --install-service --demo --data-dir $data --port $Port
if ($LASTEXITCODE -ne 0) { throw "--install-service failed ($LASTEXITCODE)" }
try {
    $svc = Get-Service $name
    if ($svc.StartType -ne 'Automatic') { throw "expected an automatic start type, got $($svc.StartType)" }
    Start-Service $name

    $healthy = $false
    for ($i = 0; $i -lt 60 -and -not $healthy; $i++) {
        try {
            $h = Invoke-RestMethod "http://127.0.0.1:$Port/api/v1/health" -TimeoutSec 2
            $healthy = $h.status -eq 'ok'
        } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $healthy) { throw 'the service never answered /api/v1/health' }
    Write-Host "service healthy on port $Port"

    Stop-Service $name
    (Get-Service $name).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
    Write-Host 'service stopped'

    # Graceful shutdown is logged; logs go to <data>\logs.
    $log = Get-ChildItem (Join-Path $data 'logs') -File | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $log -or -not (Select-String -Path $log.FullName -Pattern 'shutdown complete' -Quiet)) {
        throw 'no "shutdown complete" in the service log'
    }
    Write-Host 'graceful shutdown logged'
} finally {
    & $exe --uninstall-service
}
if (Get-Service $name -ErrorAction SilentlyContinue) { throw 'the service is still installed' }
Write-Host 'service uninstalled: OK'
