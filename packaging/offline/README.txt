CCANH - Crane Camera Automated Navigation Hub
Offline installation package for Windows 11 (x64)
=================================================

This package needs nothing else: no internet, no Visual C++ runtime, no .NET,
no database server. The web interface is built into crane.exe and is used
with Microsoft Edge, which is part of Windows 11.

Contents
--------
  crane-setup-<version>.exe   Installer (program, Windows service, Start menu)
  portable\crane.exe          The same program without installing (demo/tests)
  portable\config.example.toml
  tools\Test-Ccanh.ps1        Automated checks (before/after install, offline audit)
  tools\Collect-Diagnostics.ps1  Problem report zip (no database, no keys)
  TEST-PLAN.html              Complete site test plan (open in Edge; records results)
  SHA256SUMS.txt              Checksums of every file in this package
  BUILD-INFO.txt              Version, commit and build date

Upgrading from the previous version? A smaller diff package
(ccanh-offline-<new>-diff-from-<old>.zip) holds only the changed files; its
README-DIFF.txt explains how Apply-Diff.ps1 rebuilds this full package from
the previous one.

Quick start (PowerShell as administrator, in the package folder)
----------------------------------------------------------------
  1. Get-ChildItem . -Recurse | Unblock-File
  2. powershell -ExecutionPolicy Bypass -File .\tools\Test-Ccanh.ps1 -Preflight
  3. Run crane-setup-<version>.exe (keep "Run as a Windows service" ticked).
     The installer isn't code-signed: if SmartScreen appears, choose
     "More info" then "Run anyway".
  4. powershell -ExecutionPolicy Bypass -File .\tools\Test-Ccanh.ps1
     It shows the one-time SETUP TOKEN.
  5. Open http://127.0.0.1:8080 in Edge, enter the token and create the
     first admin (password of at least 12 characters).
  6. Follow TEST-PLAN.html.

Where things are
----------------
  Program:  C:\Program Files\CCANH   (put config.toml here to change settings)
  Data:     C:\ProgramData\CraneCamera  (database, logs, TLS certificate;
            readable by SYSTEM and Administrators only; kept on uninstall)
  Service:  CraneCameraHub (automatic start, restarts after a crash)
  Address:  http://127.0.0.1:8080 (this PC only) until you configure
            [server] bind and [server.tls] in config.toml (test case NET-01)

Ports
-----
  Inbound:  8080/TCP (web interface; configurable)
  Outbound: to cameras only - 80/443 (VAPIX), 554 (RTSP live video)

Try it without cameras
----------------------
  .\portable\crane.exe --demo --data-dir C:\ccanh-demo --port 8081
  then http://127.0.0.1:8081, sign in as admin / crane-demo-admin.

Problems
--------
  powershell -ExecutionPolicy Bypass -File .\tools\Collect-Diagnostics.ps1
  creates ccanh-diagnostics-<time>.zip on the Desktop (logs, versions,
  service state; never the database or keys). Review it before sharing.
  Lost the admin password:
    & 'C:\Program Files\CCANH\crane.exe' --data-dir C:\ProgramData\CraneCamera --create-admin <name>
