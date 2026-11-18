#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Regenerates the SQLx offline query cache in .sqlx/ (or checks it with -Check).
.DESCRIPTION
    Creates a throwaway SQLite database under target/, applies the migrations, and
    runs `cargo sqlx prepare`. The backend then compiles without a live database
    (SPEC §16). Run this after adding or changing any sqlx::query! macro or
    migration, and commit .sqlx/.
    Requires sqlx-cli: cargo install sqlx-cli --no-default-features --features sqlite,rustls
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $PSScriptRoot
$db = Join-Path $root 'target/sqlx-prepare.db'

New-Item -ItemType Directory -Force (Join-Path $root 'target') | Out-Null
Remove-Item -Force "$db*" -ErrorAction SilentlyContinue
$env:DATABASE_URL = "sqlite:$db"
Remove-Item Env:SQLX_OFFLINE -ErrorAction SilentlyContinue

Push-Location $root
try {
    cargo sqlx database create
    cargo sqlx migrate run --source backend/migrations
    if ($Check) {
        cargo sqlx prepare --workspace --check -- --all-targets
        Write-Host '.sqlx is up to date.'
    } else {
        cargo sqlx prepare --workspace -- --all-targets
    }
} finally {
    Pop-Location
    Remove-Item -Force "$db*" -ErrorAction SilentlyContinue
}
