#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Runs the backend and the Vite dev server together for development.
.DESCRIPTION
    Starts `cargo run` (API on http://127.0.0.1:8080, data in ./data) as a background
    job, then the Vite dev server in the foreground (http://localhost:5173), which
    proxies /api and WebSocket traffic to the backend. Ctrl+C stops both.
.PARAMETER Port
    Backend port. Passed to Vite as CRANE_BACKEND_URL.
#>
[CmdletBinding()]
param([int]$Port = 8080)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$frontend = Join-Path $root 'frontend'
if (-not (Test-Path (Join-Path $frontend 'node_modules'))) {
    Push-Location $frontend
    try { npm ci } finally { Pop-Location }
}

$backend = Start-Job -ScriptBlock {
    param($root, $port)
    Set-Location $root
    cargo run -p crane -- --port $port --data-dir data 2>&1
} -ArgumentList $root, $Port

$env:CRANE_BACKEND_URL = "http://127.0.0.1:$Port"
Push-Location $frontend
try {
    npm run dev
} finally {
    Pop-Location
    Stop-Job $backend -ErrorAction SilentlyContinue
    Receive-Job $backend -ErrorAction SilentlyContinue | Select-Object -Last 20
    Remove-Job $backend -Force -ErrorAction SilentlyContinue
}
