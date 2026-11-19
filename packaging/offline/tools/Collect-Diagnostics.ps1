<#
.SYNOPSIS
    Gather what's needed to analyse a CCANH problem into one zip on the Desktop.

.DESCRIPTION
    Collects: Windows and Edge versions, service configuration and state,
    crane version, health and setup state, listening ports and connections,
    firewall rules mentioning CCANH, Service Control Manager events for the
    service, config.toml, and the last 7 days of logs.

    Never collects the database (it holds password hashes and encrypted camera
    credentials) or TLS private keys. Logs never contain passwords; review the
    zip before sending it anywhere anyway.
#>
[CmdletBinding()]
param(
    [string]$Url = 'http://127.0.0.1:8080',
    [string]$DataDir = (Join-Path $env:ProgramData 'CraneCamera'),
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'CCANH'),
    [int]$Days = 7
)

$ErrorActionPreference = 'Continue'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$work = Join-Path $env:TEMP "ccanh-diagnostics-$stamp"
New-Item -ItemType Directory -Path $work -Force | Out-Null

function Save([string]$Name, [scriptblock]$Block) {
    try { & $Block 2>&1 | Out-File -FilePath (Join-Path $work $Name) -Encoding UTF8 -Width 400 }
    catch { "failed: $($_.Exception.Message)" | Out-File (Join-Path $work $Name) }
}

if ($Url -like 'https:*') { [System.Net.ServicePointManager]::ServerCertificateValidationCallback = { $true } }

Save 'system.txt' {
    Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture, LastBootUpTime
    Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer, Model, NumberOfLogicalProcessors, TotalPhysicalMemory
    Get-TimeZone
    $edge = Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'
    if (Test-Path $edge) { "Edge $((Get-Item $edge).VersionInfo.ProductVersion)" }
}
Save 'service.txt' {
    Get-Service CraneCameraHub | Format-List *
    Get-CimInstance Win32_Service -Filter "Name='CraneCameraHub'" | Format-List Name, State, StartMode, StartName, PathName, ProcessId
    sc.exe qc CraneCameraHub
    sc.exe queryex CraneCameraHub
}
Save 'version.txt' { & (Join-Path $InstallDir 'crane.exe') --version }
Save 'health.json' { Invoke-RestMethod "$Url/api/v1/health" -TimeoutSec 5 | ConvertTo-Json -Depth 6 }
Save 'setup.json' { Invoke-RestMethod "$Url/api/v1/setup" -TimeoutSec 5 | ConvertTo-Json }
Save 'network.txt' {
    $ids = @(Get-Process crane -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
    'crane processes: ' + ($ids -join ', ')
    Get-NetTCPConnection | Where-Object { $ids -contains $_.OwningProcess } |
        Sort-Object State, RemoteAddress | Format-Table LocalAddress, LocalPort, RemoteAddress, RemotePort, State -AutoSize
    Get-NetIPAddress -AddressFamily IPv4 | Format-Table InterfaceAlias, IPAddress, PrefixLength -AutoSize
}
Save 'firewall.txt' {
    Get-NetFirewallRule | Where-Object { $_.DisplayName -match 'CCANH|crane' } | Format-List DisplayName, Enabled, Direction, Action, Profile
}
Save 'service-events.txt' {
    Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Service Control Manager'; StartTime = (Get-Date).AddDays(-$Days) } -ErrorAction SilentlyContinue |
        Where-Object { $_.Message -match 'Crane' } | Format-List TimeCreated, Id, LevelDisplayName, Message
}
$cfg = Join-Path $InstallDir 'config.toml'
if (Test-Path $cfg) { Copy-Item $cfg (Join-Path $work 'config.toml') }
$logs = Join-Path $DataDir 'logs'
if (Test-Path $logs) {
    New-Item -ItemType Directory -Path (Join-Path $work 'logs') -Force | Out-Null
    Get-ChildItem $logs -File | Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-$Days) } |
        Copy-Item -Destination (Join-Path $work 'logs')
}
Save 'data-folder.txt' { Get-ChildItem $DataDir -Recurse -File | Select-Object FullName, Length, LastWriteTime | Format-Table -AutoSize }

$zip = Join-Path ([Environment]::GetFolderPath('Desktop')) "ccanh-diagnostics-$stamp.zip"
Compress-Archive -Path (Join-Path $work '*') -DestinationPath $zip -Force
Remove-Item $work -Recurse -Force
Write-Host "Diagnostics saved to $zip (no database, no keys). Review it before sharing." -ForegroundColor Green
