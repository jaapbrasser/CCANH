#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Starts a built crane binary, checks it serves the UI and /api/v1/health, then stops it.
.PARAMETER Exe
    Path to the crane binary.
.PARAMETER Port
    Port to listen on (default 18080).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [int]$Port = 18080
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$data = Join-Path ([IO.Path]::GetTempPath()) "crane-smoke-$PID"
$base = "http://127.0.0.1:$Port"

$proc = Start-Process -FilePath $exePath `
    -ArgumentList '--port', $Port, '--data-dir', $data `
    -PassThru -NoNewWindow
try {
    $deadline = (Get-Date).AddSeconds(30)
    $health = $null
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) { throw "crane exited early with code $($proc.ExitCode)" }
        try { $health = Invoke-RestMethod "$base/api/v1/health"; break } catch { Start-Sleep -Milliseconds 250 }
    }
    if (-not $health) { throw 'health endpoint did not respond within 30 s' }
    if ($health.status -ne 'ok') { throw "health status is '$($health.status)'" }
    Write-Host "health: $($health.status), version $($health.version)"

    $index = Invoke-WebRequest "$base/" -UseBasicParsing
    if ($index.Content -notmatch '<div id="root">') {
        throw 'UI shell not served; frontend/dist was probably not embedded'
    }
    Write-Host 'UI shell: embedded'

    $setup = Invoke-RestMethod "$base/api/v1/setup"
    if (-not $setup.setup_required) { throw 'fresh install should require first-run setup' }
    Write-Host 'Setup: required on fresh install'

    $docs = Invoke-WebRequest "$base/api/openapi.json" -UseBasicParsing -SkipHttpErrorCheck
    if ($docs.StatusCode -ne 401) { throw "OpenAPI should be admin-only, got HTTP $($docs.StatusCode)" }
    Write-Host 'OpenAPI: protected'
} finally {
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
}
