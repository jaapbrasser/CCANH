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
