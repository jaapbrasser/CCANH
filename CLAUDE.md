# CLAUDE.md

Crane Camera Automated Navigation Hub (CCANH): a Windows-hosted Rust server with an
embedded React UI for managing AXIS IP cameras. **`docs/SPEC.md` is authoritative.**
Read §0–3 before coding, then the section for the current phase (§17).

## Current phase

Phase 0 (Foundation) is complete. Next: Phase 1 (Auth & users). Don't build ahead of
the current phase, and post a plan before starting a phase.

## Commands

Backend (repo root):

```bash
cargo run -p crane -- --data-dir data     # serve on http://127.0.0.1:8080
cargo run -p crane -- --print-openapi     # OpenAPI JSON to stdout, no server
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check                          # needs `cargo install cargo-deny`
```

Frontend (`frontend/`):

```bash
npm ci
npm run dev          # http://localhost:5173, proxies /api (+ws) to :8080
npm run lint
npm run typecheck
npm test
npm run build        # -> frontend/dist, embedded by release builds
```

Scripts (`pwsh`, cross-platform):

```bash
pwsh scripts/dev.ps1                      # backend + Vite together
pwsh scripts/gen-api-types.ps1            # regenerate frontend/src/api/schema.d.ts
pwsh scripts/gen-api-types.ps1 -Check     # CI drift check
pwsh scripts/build.ps1                    # release binary with embedded UI
pwsh scripts/smoke-test.ps1 -Exe target/release/crane
```

## Layout (as it exists)

```text
Cargo.toml, rust-toolchain.toml, deny.toml, config.example.toml
backend/
  src/main.rs          CLI, startup, signal handling
  src/lib.rs           AppState, app(), serve() with graceful shutdown
  src/api/             router, security headers, problem+json errors, health, static files
  src/config/          figment layering: defaults < TOML < CRANE_* env < CLI
  src/storage/         SQLite pool (WAL), migrations, pre-migration backup
  src/telemetry/       tracing setup, rolling JSON log files
  migrations/          sqlx migrations (embedded)
  tests/               in-process integration tests
frontend/src/          App, components/, api/ (client.ts, generated schema.d.ts), test/
scripts/               build, dev, gen-api-types, smoke-test
docs/                  SPEC.md, adr/, development.md, dependencies.md
.github/workflows/ci.yml
```

## Conventions

- Conventional Commits, one logical change each. Never commit secrets, real camera
  IPs or credentials.
- Tests ship with the code. No `unwrap()`/`expect()` outside tests unless the
  invariant is guaranteed (comment why).
- Module errors: `thiserror`. `anyhow` only in `main.rs`. API errors: problem+json
  via `api::error::ApiError`; never expose internal details.
- New endpoint: `#[utoipa::path]` with `Required role: …` in its description,
  register with `routes!` in `api::v1_routes`, then run `gen-api-types.ps1`.
  A test fails if an operation omits its required role.
- Significant decisions get an ADR in `docs/adr/`. New dependencies go in
  `docs/dependencies.md`.
- Every AXIS endpoint must be verified against official VAPIX docs and logged in
  `docs/vapix.md` (from Phase 4).
- Frontend API calls go through `src/api/client.ts` (typed `openapi-fetch`).

## Gotchas

- TypeScript is pinned to 5.9 (peer deps not ready for 7). See ADR 0001.
- `/api/docs` has a looser CSP (inline styles) than the app. See ADR 0001.
- Without rustup (e.g. Homebrew Rust), `rust-toolchain.toml` is ignored locally.
