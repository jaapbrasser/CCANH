<#
.SYNOPSIS
    On-site checks for a CCANH installation on Windows (no internet needed).
    Works with the built-in Windows PowerShell 5.1.

.DESCRIPTION
    -Preflight     Before installing: Windows version, admin rights, Microsoft Edge,
                   port free, disk space, package checksums, download block.
    (default)      After installing: service state, version, health endpoint,
                   first-run setup token, recent log errors.
    -Connections   Lists the server's network connections and flags any that
                   go to a public (internet) address. Run while cameras stream.

    Results are printed and saved to ccanh-check-<timestamp>.txt next to
    this script.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\Test-Ccanh.ps1 -Preflight
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\Test-Ccanh.ps1
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\Test-Ccanh.ps1 -Url https://ccanh-host:8443 -Connections
#>
[CmdletBinding()]
param(
    [switch]$Preflight,
    [switch]$Connections,
    # Base URL of the server (default: the installer's local address).
    [string]$Url = 'http://127.0.0.1:8080',
    [string]$DataDir = (Join-Path $env:ProgramData 'CraneCamera'),
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'CCANH')
)

$ErrorActionPreference = 'Stop'
$ServiceName = 'CraneCameraHub'
$results = New-Object System.Collections.Generic.List[string]
$script:failures = 0
$script:warnings = 0

function Report([string]$Level, [string]$Check, [string]$Detail) {
    $line = '{0,-5} {1}: {2}' -f $Level, $Check, $Detail
    $results.Add($line)
    switch ($Level) {
        'PASS' { Write-Host $line -ForegroundColor Green }
        'WARN' { Write-Host $line -ForegroundColor Yellow; $script:warnings++ }
        'FAIL' { Write-Host $line -ForegroundColor Red; $script:failures++ }
        default { Write-Host $line }
    }
}

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Self-signed HTTPS (server.tls.mode = "self_signed") is accepted for these
# local checks only.
function Invoke-Ccanh([string]$Path) {
    if ($Url -like 'https:*') {
        [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.SecurityProtocolType]::Tls12
        [System.Net.ServicePointManager]::ServerCertificateValidationCallback = { $true }
    }
    Invoke-RestMethod -Uri ($Url.TrimEnd('/') + $Path) -TimeoutSec 5 -UseBasicParsing
}

function Test-PrivateAddress([string]$Address) {
    $ip = $null
    if (-not [System.Net.IPAddress]::TryParse($Address, [ref]$ip)) { return $false }
    if ($ip.IsIPv4MappedToIPv6) { $ip = $ip.MapToIPv4() }
    if ($ip.AddressFamily -eq 'InterNetworkV6') {
        return ($ip.IsIPv6LinkLocal -or $ip.IsIPv6SiteLocal -or [System.Net.IPAddress]::IsLoopback($ip) -or
            $Address.ToLower().StartsWith('fc') -or $Address.ToLower().StartsWith('fd'))
    }
    $b = $ip.GetAddressBytes()
    return ($b[0] -eq 10 -or $b[0] -eq 127 -or
        ($b[0] -eq 172 -and $b[1] -ge 16 -and $b[1] -le 31) -or
        ($b[0] -eq 192 -and $b[1] -eq 168) -or
        ($b[0] -eq 169 -and $b[1] -eq 254) -or
        ($b[0] -eq 100 -and $b[1] -ge 64 -and $b[1] -le 127) -or   # CGNAT, often used internally
        $Address -eq '0.0.0.0')
}

function Invoke-Preflight {
    $os = Get-CimInstance Win32_OperatingSystem
    $build = [int]$os.BuildNumber
    if ($build -ge 22000) { Report 'PASS' 'Windows' "$($os.Caption) build $build" }
    elseif ($build -ge 17763) { Report 'WARN' 'Windows' "$($os.Caption) build $build (tested target is Windows 11)" }
    else { Report 'FAIL' 'Windows' "$($os.Caption) build $build is too old" }
    if ($os.OSArchitecture -notmatch '64') { Report 'FAIL' 'Architecture' "$($os.OSArchitecture): crane.exe is x64 only" }
    else { Report 'PASS' 'Architecture' $os.OSArchitecture }

    if (Test-Admin) { Report 'PASS' 'Administrator' 'running elevated (needed to install the service)' }
    else { Report 'WARN' 'Administrator' 'not elevated: run PowerShell as administrator to install' }

    $edge = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{56EB18F8-B008-4CBD-B6D2-8C97FE7E9062}' -ErrorAction SilentlyContinue
    $edgeExe = Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'
    if ($edge -or (Test-Path $edgeExe)) {
        $v = if ($edge) { $edge.pv } else { (Get-Item $edgeExe).VersionInfo.ProductVersion }
        Report 'PASS' 'Browser' "Microsoft Edge $v (plays H.264 live video)"
    } else { Report 'WARN' 'Browser' 'Microsoft Edge not found; use a current Chromium-based or Firefox browser' }

    $port = ([Uri]$Url).Port
    $busy = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    if ($busy) {
        $owner = ($busy | Select-Object -First 1).OwningProcess
        $name = (Get-Process -Id $owner -ErrorAction SilentlyContinue).ProcessName
        if ($name -eq 'crane') { Report 'INFO' "Port $port" 'already used by crane (installed earlier?)' }
        else { Report 'FAIL' "Port $port" "in use by $name (pid $owner); free it or set another port" }
    } else { Report 'PASS' "Port $port" 'free' }

    $drive = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$($env:SystemDrive)'"
    $freeGb = [math]::Round($drive.FreeSpace / 1GB, 1)
    if ($freeGb -ge 2) { Report 'PASS' 'Disk space' "$freeGb GB free on $($env:SystemDrive)" }
    else { Report 'WARN' 'Disk space' "only $freeGb GB free on $($env:SystemDrive)" }

    $root = Split-Path $PSScriptRoot -Parent
    $sums = Join-Path $root 'SHA256SUMS.txt'
    if (Test-Path $sums) {
        $bad = 0
        foreach ($line in Get-Content $sums) {
            if ($line -notmatch '^([0-9a-fA-F]{64})\s+\*?(.+)$') { continue }
            $file = Join-Path $root ($Matches[2] -replace '/', '\')
            if (-not (Test-Path $file)) { Report 'FAIL' 'Checksum' "missing $($Matches[2])"; $bad++; continue }
            $actual = (Get-FileHash $file -Algorithm SHA256).Hash
            if ($actual -ne $Matches[1].ToUpper()) { Report 'FAIL' 'Checksum' "$($Matches[2]) does not match"; $bad++ }
        }
        if ($bad -eq 0) { Report 'PASS' 'Checksums' 'all package files match SHA256SUMS.txt' }
    } else { Report 'WARN' 'Checksums' 'SHA256SUMS.txt not found next to the tools folder' }

    $blocked = Get-ChildItem $root -Recurse -File | Where-Object {
        Get-Item $_.FullName -Stream Zone.Identifier -ErrorAction SilentlyContinue
    }
    if ($blocked) {
        Report 'WARN' 'Download block' "$(@($blocked).Count) file(s) are marked as downloaded; run: Get-ChildItem '$root' -Recurse | Unblock-File"
    } else { Report 'PASS' 'Download block' 'no files are marked as downloaded' }
}

function Invoke-PostInstall {
    $svc = Get-Service $ServiceName -ErrorAction SilentlyContinue
    if ($svc) {
        if ($svc.Status -eq 'Running') { Report 'PASS' 'Service' "$ServiceName is running" }
        else { Report 'FAIL' 'Service' "$ServiceName is $($svc.Status); try: Start-Service $ServiceName" }
        if ($svc.StartType -eq 'Automatic') { Report 'PASS' 'Service start' 'starts with Windows' }
        else { Report 'WARN' 'Service start' "start type is $($svc.StartType)" }
        $failure = (sc.exe qfailure $ServiceName) -join ' '
        if ($failure -match 'RESTART') { Report 'PASS' 'Crash recovery' 'the service restarts automatically after a failure' }
        else { Report 'WARN' 'Crash recovery' "no restart actions; set them with: sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/10000/restart/60000" }
        $wmi = Get-CimInstance Win32_Service -Filter "Name='$ServiceName'"
        Report 'INFO' 'Service account' $wmi.StartName
        Report 'INFO' 'Service command' $wmi.PathName
    } else { Report 'WARN' 'Service' "$ServiceName is not installed (fine if you run crane.exe in a console)" }

    $exe = Join-Path $InstallDir 'crane.exe'
    if (Test-Path $exe) {
        $version = (& $exe --version) 2>&1
        Report 'PASS' 'Program' "$exe ($version)"
    } else { Report 'WARN' 'Program' "$exe not found (installed elsewhere?)" }

    $configFile = Join-Path $InstallDir 'config.toml'
    if (Test-Path $configFile) { Report 'INFO' 'Config' "$configFile is in use" }
    else { Report 'INFO' 'Config' 'no config.toml next to crane.exe: defaults apply (127.0.0.1:8080, plain HTTP)' }

    try {
        $health = Invoke-Ccanh '/api/v1/health'
        if ($health.status -eq 'ok') { Report 'PASS' 'Health' "$Url answers: version $($health.version), mode $($health.mode), up $($health.uptime_seconds) s" }
        else { Report 'FAIL' 'Health' ($health | ConvertTo-Json -Compress -Depth 5) }
        Report 'INFO' 'Database' ($health.checks.database | ConvertTo-Json -Compress)
        # `services` maps each background service to its state and restart count.
        foreach ($p in $health.services.PSObject.Properties) {
            Report 'INFO' 'Background service' "$($p.Name): $($p.Value | ConvertTo-Json -Compress)"
        }
    } catch { Report 'FAIL' 'Health' "$Url did not answer: $($_.Exception.Message)" }

    try {
        $setup = Invoke-Ccanh '/api/v1/setup'
        if ($setup.setup_required) {
            Report 'INFO' 'First run' 'no accounts yet: open the web interface and use the setup token below'
            $log = Get-ChildItem (Join-Path $DataDir 'logs') -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
            $token = $null
            if ($log) {
                $m = Select-String -Path $log.FullName -Pattern '"setup_token":"([^"]+)"' | Select-Object -Last 1
                if ($m) { $token = $m.Matches[0].Groups[1].Value }
            }
            if ($token) { Report 'INFO' 'Setup token' $token }
            else { Report 'WARN' 'Setup token' "not found in the log; or create an admin with: & '$exe' --data-dir '$DataDir' --create-admin admin" }
        } else { Report 'PASS' 'First run' 'an admin account exists' }
    } catch { Report 'WARN' 'First run' "could not query setup state: $($_.Exception.Message)" }

    if (Test-Path $DataDir) {
        Report 'PASS' 'Data folder' $DataDir
        $acl = Get-Acl $DataDir
        $open = $acl.Access | Where-Object {
            $_.IdentityReference -match 'Users|Everyone|Authenticated Users' -and $_.AccessControlType -eq 'Allow'
        }
        if ($open) { Report 'WARN' 'Data folder access' "readable by $(($open | ForEach-Object { $_.IdentityReference }) -join ', '); restrict with: icacls `"$DataDir`" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F" }
        else { Report 'PASS' 'Data folder access' 'only SYSTEM and Administrators' }
        $logs = Get-ChildItem (Join-Path $DataDir 'logs') -File -ErrorAction SilentlyContinue |
            Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-1) }
        $errors = 0
        foreach ($l in $logs) { $errors += @(Select-String -Path $l.FullName -Pattern '"level":"ERROR"').Count }
        if ($errors -eq 0) { Report 'PASS' 'Log errors' 'no ERROR entries in the last 24 hours' }
        else { Report 'WARN' 'Log errors' "$errors ERROR entries in the last 24 hours (see $DataDir\logs)" }
        $tls = Join-Path $DataDir 'tls\server.crt'
        if (Test-Path $tls) {
            $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $tls
            $sha = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($cert.RawData)) -replace '-', ':'
            Report 'INFO' 'TLS certificate' "SHA-256 $sha (compare with the browser's certificate details)"
        }
    } else { Report 'FAIL' 'Data folder' "$DataDir not found" }
}

function Invoke-Connections {
    $procs = Get-Process crane -ErrorAction SilentlyContinue
    if (-not $procs) { Report 'FAIL' 'Connections' 'crane is not running'; return }
    $ids = @($procs | ForEach-Object { $_.Id })
    $conns = Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess }
    $listen = $conns | Where-Object { $_.State -eq 'Listen' }
    foreach ($l in $listen) { Report 'INFO' 'Listening' "$($l.LocalAddress):$($l.LocalPort)" }
    $outbound = $conns | Where-Object { $_.State -ne 'Listen' -and $_.RemoteAddress -and $_.RemoteAddress -ne '0.0.0.0' -and $_.RemoteAddress -ne '::' }
    $public = @()
    $byRemote = $outbound | Group-Object RemoteAddress
    foreach ($g in $byRemote) {
        $ports = ($g.Group | ForEach-Object { $_.RemotePort } | Sort-Object -Unique) -join ','
        if (Test-PrivateAddress $g.Name) { Report 'INFO' 'Connection' "$($g.Name) ports $ports ($($g.Count))" }
        else { $public += $g.Name; Report 'FAIL' 'Internet connection' "$($g.Name) ports ${ports}: not a private address" }
    }
    if ($public.Count -eq 0) { Report 'PASS' 'Offline' 'no connections to public (internet) addresses' }
    Report 'INFO' 'Note' 'camera addresses in public ranges would be flagged too; check them against your camera list'
}

Write-Host "CCANH checks - $(Get-Date -Format s) on $env:COMPUTERNAME" -ForegroundColor Cyan
if ($Preflight) { Invoke-Preflight }
elseif ($Connections) { Invoke-Connections }
else { Invoke-PostInstall; Invoke-Connections }

$summary = "Summary: $script:failures failed, $script:warnings warnings"
$results.Add($summary)
$color = if ($script:failures) { 'Red' } elseif ($script:warnings) { 'Yellow' } else { 'Green' }
Write-Host $summary -ForegroundColor $color
$out = Join-Path $PSScriptRoot ("ccanh-check-{0}.txt" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
try { $results | Set-Content -Path $out -Encoding UTF8; Write-Host "Saved to $out" } catch { Write-Host "Could not save results: $($_.Exception.Message)" }
if ($script:failures) { exit 1 }
