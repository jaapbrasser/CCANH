# Development guide

## Prerequisites

- Rust (via rustup; the version is pinned in `rust-toolchain.toml`). On Windows, the
  MSVC toolchain.
- Node.js 22 LTS and npm (frontend only, never needed at runtime).
- PowerShell 7 (`pwsh`) for the scripts in `scripts/`.

## Everyday commands

```bash
cargo run -p crane -- --data-dir data        # backend on http://127.0.0.1:8080
```

```bash
cd frontend && npm ci && npm run dev         # UI on http://localhost:5173, proxies /api
```

Or both at once: `pwsh scripts/dev.ps1`.

Debug builds serve `frontend/dist` from disk if it exists, otherwise a placeholder
page. Use the Vite dev server for UI work.

## Checks (definition of done, SPEC §0.2)

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

```bash
cd frontend && npm run lint && npm run typecheck && npm test
```

```bash
pwsh scripts/gen-api-types.ps1 -Check
```

## First run and accounts

With an empty database the server prints a one-time setup token to the console
and log. Open `/setup` and enter it to create the first admin, or stop the server
and run:

```bash
cargo run -p crane -- --data-dir data --create-admin admin
```

`--create-admin` also works when users exist, as the recovery path if every
admin password is lost. There are no default credentials.

## API clients and CSRF

State-changing requests (`POST`, `PATCH`, `DELETE`, …) must send an `Origin`
header matching the server, e.g. with curl:

```bash
curl -c jar -H "Origin: http://127.0.0.1:8080" -H "Content-Type: application/json" -d '{"username":"admin","password":"..."}' http://127.0.0.1:8080/api/v1/auth/login
```

See ADR 0004.

## TLS

Set `server.tls.mode = "self_signed"` to get HTTPS with a generated certificate
(stored in `<data_dir>/tls/`, fingerprint logged at startup), or `"files"` with
`cert_path`/`key_path` for your own certificate. On Windows, restrict the data
directory's ACL to administrators and the account running crane: it holds the
database and the TLS private key.

## Local RTSP test source (streaming development)

No AXIS camera is needed for streaming work. A local RTSP source publishes H.264
test patterns shaped like a camera's main and grid profiles (SPEC §15.5):

```bash
pwsh scripts/rtsp-test-source.ps1
```

This serves `rtsp://127.0.0.1:8554/test/main` (720p25, 2 Mbit/s) and
`rtsp://127.0.0.1:8554/test/grid` (360p15, 400 kbit/s). It needs `ffmpeg` and
`mediamtx` on PATH, as dev tools only: on macOS `brew install ffmpeg mediamtx`;
on Windows `winget install Gyan.FFmpeg` plus the MediaMTX release zip. The
Phase 3 spike that used it is on branch `spike/streaming`; the findings are in
ADR 0002.

With the source running, check the production RTSP client against it:

```bash
cargo test -p crane --test rtsp_source -- --ignored
```

(`CRANE_RTSP_TEST_URL` picks another stream, for example `…/test/main`.) The
regular test suite doesn't need it: `providers/axis/rtsp_tests.rs` runs a fake
AXIS RTSP server in-process.

## End-to-end tests (Playwright)

They run against the release binary in demo mode (a fresh demo database in a
temp directory, port 18080):

```bash
pwsh scripts/build.ps1
cd frontend && npx playwright install chromium   # once
npm run e2e
```

`CRANE_E2E_EXE` and `CRANE_E2E_PORT` override the binary and port. Rebuild the
binary after frontend changes, since the UI is embedded. CI runs them in the
`e2e` job and uploads the report on failure.

## Windows service and installer

```bash
crane --install-service --data-dir C:\ProgramData\CraneCamera   # as administrator
sc start CraneCameraHub
crane --uninstall-service
```

Options given with `--install-service` become the service's options. The
service runs as LocalSystem; logs are in `<data dir>\logs`. To build the
installer (Inno Setup 6 needed): `pwsh scripts/build.ps1`, then
`pwsh scripts/build-installer.ps1`, which writes `dist\crane-setup-<version>.exe`.
`scripts/service-test.ps1` and `scripts/installer-test.ps1` are what CI runs.
See ADR 0010.

## Offline deployment package

`pwsh scripts/package-offline.ps1` (Windows, after `build.ps1` and
`build-installer.ps1`) writes `dist\ccanh-offline-<version>.zip`: installer,
portable exe, README, `TEST-PLAN.html` (83 site test cases; records results in
the browser and exports CSV), `tools\Test-Ccanh.ps1`,
`tools\Collect-Diagnostics.ps1` and SHA-256 checksums. CI builds it in the
Windows job (artifact `ccanh-offline-windows-x64`), checks the exe imports only
Windows DLLs, and runs the pre-flight check from the extracted zip. Sources are
in `packaging/offline/`.

## Load test

`cargo test --release -p crane --test perf -- --ignored --nocapture` runs the
target-scale test (250 cameras, 25 users, 50 streams); results and caveats in
docs/performance.md.

## Database queries (SQLx offline mode)

Queries use `sqlx::query!` macros, checked at compile time against `.sqlx/`.
After adding or changing a query or migration:

```bash
pwsh scripts/sqlx-prepare.ps1
```

Then commit `.sqlx/`. CI runs it with `-Check`. Needs `sqlx-cli` (see
`docs/dependencies.md`). If a build fails with "no cached data for this query",
the cache is stale.

## API types

After changing any endpoint or schema, run `pwsh scripts/gen-api-types.ps1` and
commit `frontend/src/api/schema.d.ts`. CI fails if it's stale.

## Release build

`pwsh scripts/build.ps1` produces `target/release/crane(.exe)` with the UI embedded.
`pwsh scripts/smoke-test.ps1 -Exe target/release/crane` checks it starts and serves
the UI, health and OpenAPI.

## Configuration

See `config.example.toml`. Useful overrides while developing:

- `--data-dir data` keeps the database and logs inside the repo (git-ignored).
- `RUST_LOG=crane=debug,tower_http=debug` for request-level logs.
- `CRANE_LOGGING__FILE=false` disables log files.
