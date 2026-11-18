#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Regenerates frontend/src/api/schema.d.ts from the backend's OpenAPI document.
.DESCRIPTION
    Runs `crane --print-openapi` (no server or database needed), then
    openapi-typescript. With -Check, fails if the committed file is out of date
    instead of overwriting it (used in CI).
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'frontend/src/api/schema.d.ts'
$tmp = Join-Path ([IO.Path]::GetTempPath()) "crane-openapi-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null

try {
    $spec = Join-Path $tmp 'openapi.json'
    $generated = Join-Path $tmp 'schema.d.ts'

    Push-Location $root
    try {
        $json = cargo run --quiet -p crane -- --print-openapi
        # Write UTF-8 without BOM so output is identical on every OS.
        [IO.File]::WriteAllText($spec, ($json -join "`n"))
    } finally { Pop-Location }

    Push-Location (Join-Path $root 'frontend')
    try {
        npx --no-install openapi-typescript $spec -o $generated
    } finally { Pop-Location }

    $new = [IO.File]::ReadAllText($generated) -replace "`r`n", "`n"
    if ($Check) {
        $old = if (Test-Path $target) { [IO.File]::ReadAllText($target) -replace "`r`n", "`n" } else { '' }
        if ($old -ne $new) {
            Write-Error 'frontend/src/api/schema.d.ts is out of date. Run scripts/gen-api-types.ps1 and commit the result.'
        }
        Write-Host 'API types are up to date.'
    } else {
        [IO.File]::WriteAllText($target, $new)
        Write-Host "Wrote $target"
    }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
