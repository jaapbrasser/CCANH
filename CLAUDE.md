# CLAUDE.md

Crane Camera Automated Navigation Hub (CCANH): a Windows-hosted Rust server with an
embedded React UI for managing AXIS IP cameras. **`docs/SPEC.md` is authoritative.**
Read §0–3 before coding, then the section for the current phase (§17).

## Current phase

Phases 0–7 are complete: Foundation, Auth & users, Camera core + demo,
streaming spike (ADR 0002), AXIS provider + credentials + polling + tasks +
runtime settings, Phase 5 (stream manager, demo video, live view, Streams page,
AXIS RTSP via `retina`, grid/detail profile choices), Phase 6 (groups, tags,
favorites, bulk operations, ADR 0009), Phase 7 (dashboards, ADR 0005;
Playwright e2e), Phase 8 (discovery) and Phase 9 (retention service,
connection limits, Windows service + Inno Setup installer, load test, ADRs
0006/0010), all built on the SPEC §18 default answers. Next: review the §18
answers with the owner; there's no Phase 10 in the spec. Don't build ahead of the current phase,
and post a plan before starting one.

## Commands

Backend (repo root):

```bash
cargo run -p crane -- --data-dir data     # serve on http://127.0.0.1:8080
cargo run -p crane -- --demo --data-dir data   # demo mode: simulated cameras, demo.db
cargo run -p crane -- --print-openapi     # OpenAPI JSON to stdout, no server
cargo run -p crane -- --data-dir data --create-admin admin   # prompt for password
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
npm run e2e          # Playwright vs target/release/crane in demo mode (build first)
```

Scripts (`pwsh`, cross-platform):

```bash
pwsh scripts/dev.ps1                      # backend + Vite together
pwsh scripts/gen-api-types.ps1            # regenerate frontend/src/api/schema.d.ts
pwsh scripts/gen-api-types.ps1 -Check     # CI drift check
pwsh scripts/sqlx-prepare.ps1             # regenerate .sqlx/ after query/migration changes
pwsh scripts/build.ps1                    # release binary with embedded UI
pwsh scripts/smoke-test.ps1 -Exe target/release/crane
pwsh scripts/rtsp-test-source.ps1         # local RTSP H.264 source (needs ffmpeg + mediamtx)
pwsh scripts/build-installer.ps1          # Windows: dist\crane-setup-<v>.exe (Inno Setup 6)
pwsh scripts/service-test.ps1 -Exe target/release/crane.exe   # Windows, admin
cargo test --release -p crane --test perf -- --ignored --nocapture   # load test
```

## Layout (as it exists)

```text
Cargo.toml, rust-toolchain.toml, deny.toml, config.example.toml
backend/
  src/main.rs          CLI, startup, signal handling
  src/lib.rs           AppState, app(), serve() with graceful shutdown
  src/api/             router, security headers, problem+json errors, handlers
                       (health, setup, auth, users, audit, cameras, ws), extractors
  src/auth/            roles, Argon2id, sessions, rate limiter, CSRF guard,
                       AuthUser/AdminUser extractors, first-run setup
  src/users/           user model + repository (last-admin guard in SQL)
  src/audit/           append-only audit log
  src/cameras/         camera registry: models, validation, repository, status
  src/providers/       CameraProvider trait, factory, demo/ (simulated fleet),
                       axis/ (VAPIX client, Digest auth, per-camera TLS, tests
                       with wiremock), contract.rs (provider contract suite)
  src/credentials/     CredentialStore (DPAPI on Windows, AES-GCM dev store),
                       credential sets (shared + private)
  src/monitoring/      status monitor (PollingPolicy, transitions, identity refresh)
  src/events/          typed broadcast event bus
  src/demo/            demo seeding and demo.db reset
  src/supervisor.rs    restart background services with backoff; health
  src/retention.rs     daily cleanup of history, tasks/scans and (opt-in) audit
  src/settings/        admin-editable runtime settings (watch channel, DB overrides)
  src/tasks/           background task manager (persisted, cancellable, `tasks` topic)
  src/organization/    groups (tree, cycle checks), tags, favorites, memberships
  src/dashboards/      per-user dashboards (grid, tiles, default)
  src/discovery/       range parsing, HostProber (AXIS: TCP check + one anonymous
                       probe; demo: simulated), scan runner, duplicate rules
  src/streaming/       MediaSource, H.264 parsing, fMP4 muxer, demo clip replay,
                       RTSP source (retina), profile resolution,
                       StreamManager (shared upstream, GOP cache, grace, reconnect)
  src/tls/             rustls (ring) config: PEM files or self-signed
  src/secret.rs        SecretString re-export
  src/config/          figment layering: defaults < TOML < CRANE_* env < CLI
  src/storage/         SQLite pool (WAL), migrations, pre-migration backup
  src/telemetry/       tracing setup, rolling JSON log files
  migrations/          sqlx migrations (embedded)
  tests/               in-process integration tests (common/ = helpers)
.sqlx/                 SQLx offline query cache (committed)
assets/demo/           bundled H.264 demo clips (embedded; scripts/gen-demo-clips.ps1)
frontend/e2e/          Playwright specs (run against the release binary)
frontend/src/          App (routes), pages/, components/, auth/ (guards),
                       streaming/ (shared media sockets, MSE player),
                       api/ (client.ts, hooks, generated schema.d.ts), test/ (MSW)
packaging/             Inno Setup script (crane.iss)
scripts/               build, dev, gen-api-types, smoke-test, service/installer tests
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
- New endpoint: `#[utoipa::path]` with an explicit `operation_id` and
  `Required role: none|viewer|admin` in its description; take `AuthUser` or
  `AdminUser` as the **first** handler argument; use `JsonBody`/`PathParam`/
  `QueryParams` (problem+json rejections); register with `routes!` in
  `api::v1_routes`; run `gen-api-types.ps1`. Tests fail on missing roles,
  duplicate operation IDs, dangling `$ref`s, and any route that doesn't enforce
  its declared role.
- Security-relevant actions write an audit entry (`audit::action::*`).
- Secrets are `SecretString`; never log request bodies of credential endpoints.
- Significant decisions get an ADR in `docs/adr/`. New dependencies go in
  `docs/dependencies.md`.
- Any new `CameraProvider` method gets a DemoProvider implementation in the same
  change, and the contract suite (`providers/contract.rs`) covers it.
- New WebSocket message types or topics: update `api::ws` and ADR 0007.
- Long-running operations: `state.tasks.spawn(kind, camera, creator, job)` and
  return 202 with the task; report progress via `TaskContext::progress` and
  honour `ctx.cancel` (ADR 0008).
- New runtime-editable settings: add a group to `settings/` with bounds,
  defaults from config, and subscribe via `state.settings.subscribe()`.
- Every AXIS endpoint must be verified against official VAPIX docs and logged in
  `docs/vapix.md`, with a fixture in `backend/tests/fixtures/vapix/`.
- Camera credentials: only ever ciphertext in SQLite and responses expose
  `has_credentials` / set name only. Decrypt at the point of use
  (`MonitorTarget::provider_target`).
- Frontend API calls go through `src/api/client.ts` (typed `openapi-fetch`).

## Gotchas

- TypeScript is pinned to 5.9 (peer deps not ready for 7). See ADR 0001.
- `/api/docs` has a looser CSP (inline styles) than the app. See ADR 0001.
- State-changing API calls need a matching `Origin` header (CSRF, ADR 0004);
  integration tests send `Origin: http://localhost` + `Host: localhost`.
- `query!` macros need `.sqlx/` (offline) or `DATABASE_URL`. CI sets
  `SQLX_OFFLINE=true`.
- MSW 3 uses `onUnhandledFrame`, not `onUnhandledRequest`. The live socket is
  mocked in `src/test/server.ts` (`live.broadcast(...)` to push messages).
- AppState::new takes the `CredentialStore`; tests use
  `AesGcmStore::ephemeral()` so no key files are written.
- Status checks use anonymous VAPIX calls, so wrong credentials don't make a
  camera "offline": they show as `identity_error_category = auth_failed`.
- Tokio paused-time tests need `#[tokio::test(start_paused = true)]` (dev
  feature `test-util` is enabled).
- Without rustup (e.g. Homebrew Rust), `rust-toolchain.toml` is ignored locally.
- Live view needs MSE; jsdom has none, so `LivePlayer` renders "not supported"
  in tests and opens no socket. Test stream sharing via `StreamRegistry` with a
  fake socket factory (`src/streaming/registry.test.ts`).
- Live video uses its own media WebSocket per stream, not the control socket.
- Dashboard tiles must stay keyed by camera id and positioned with CSS grid
  coordinates (never reparented), or rearranging remounts players (ADR 0005).
- Playwright's Chromium may lack H.264; e2e asserts on `/api/v1/streams`, not
  on decoded video. Rebuild the binary before `npm run e2e` (UI is embedded).
- Discovery never auto-registers. Duplicates are computed when results are
  read (against current cameras), so registering one result updates the rest.
- Fresh demo cameras answer 409 on the media socket until their profiles
  load; the stream registry retries refused sockets with backoff.
- Service mode code is `#[cfg(windows)]` in `main.rs`; it only compiles on
  Windows (CI). The service runs as LocalSystem from `System32`, so paths it
  gets must be absolute (`--install-service` makes them so).
- Stream limits live in `streaming::policy::LimitsPolicy` (the `StreamPolicy`
  hook); `StreamManager::new` uses it, tests can pass `AdmitAll`.
- AXIS RTSP tests use the in-process fake server in
  `providers/axis/rtsp_tests.rs` (`FakeCamera::start_with`); the AXIS contract
  suite needs it too, since `open_stream` really connects.
