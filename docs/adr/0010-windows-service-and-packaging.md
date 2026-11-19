# ADR 0010: Windows service mode and installer

- Status: accepted
- Date: 2026-11-19
- Phase: 9

## Context

SPEC §16 and §18 Q2 (default answer: interactive exe first, service mode in
Phase 9, DPAPI machine scope) ask for an optional Windows service mode
(install, uninstall, run) and an optional installer.

## Decisions

### Service mode
- `windows-service` crate (Mullvad; MIT/Apache-2.0, maintained) for the
  service control dispatcher, control handler and service manager calls.
- CLI: `--install-service` registers `CraneCameraHub` ("Crane Camera Automated
  Navigation Hub"), auto start, as **LocalSystem**, so the DPAPI machine-scope
  credential store (ADR 0003) works unchanged. Options given with it
  (`--data-dir`, `--config`, `--bind`, `--port`, `--log-level`, `--demo`) are
  made absolute and become the service's command line, since services start
  in `System32`. The default data directory on Windows is
  `%ProgramData%\CraneCamera`.
- `--uninstall-service` stops the service (waits up to a minute) and deletes
  it; data is kept. `--service` is hidden and only for the service manager.
- A service Stop or system shutdown cancels the same token Ctrl+C does, so
  shutdown is graceful (in-flight requests finish, tasks are cancelled,
  streams closed, "shutdown complete" logged).
- On other platforms the flags exit with a clear error (use systemd/launchd).

### Installer
- **Inno Setup 6** (`packaging/crane.iss`, `scripts/build-installer.ps1`):
  free, scriptable, small, widely used. WiX produces MSIs (better for Group
  Policy deployment) but is far more complex. Revisit if MSI is required.
- Installs `crane.exe` and `config.example.toml` to `Program Files\CCANH`,
  creates `%ProgramData%\CraneCamera`, and (task "service", on by default)
  registers and starts the service. Start-menu shortcuts open the web UI and
  the data folder. Uninstalling removes the service and program files, but
  keeps the data folder.
- The server binds to `127.0.0.1` by default, so no firewall rule is added.
  Serving the LAN means setting `server.bind` and TLS in
  `%ProgramData%\CraneCamera\config.toml` or the service's options. That's
  documented, not automated.

### Tests
- CI (Windows job) installs the service from the built exe, starts it, checks
  `/api/v1/health`, stops it, checks the graceful-shutdown log line, and
  uninstalls it (`scripts/service-test.ps1`). It then builds the installer,
  installs it silently with the service task, checks health on port 8080,
  uninstalls it and checks the service is gone (`scripts/installer-test.ps1`).

### Code signing (notes, not done)
- Unsigned binaries trigger SmartScreen warnings. To sign: get an OV or EV
  code-signing certificate (EV avoids the SmartScreen reputation ramp), then
  run `signtool sign /fd SHA256 /tr <timestamp-url> /td SHA256` on `crane.exe`
  before packaging and on `crane-setup-*.exe` after (Inno Setup's `SignTool`
  directive can do both). Keep the key in an HSM or a CI secret store. Never
  commit it.

## Consequences

- One more Windows-only dependency (small, no build-time tools).
- The installer needs Inno Setup on the build machine (CI installs it with
  Chocolatey when missing).
