# ADR 0001: Workspace layout and dependency baseline

- Status: accepted
- Date: 2026-11-18
- Phase: 0

## Context

Phase 0 sets up the repository skeleton (SPEC §3.3, §17). Several choices made now
are awkward to change later: workspace shape, how the frontend is embedded, how
configuration is layered, and the baseline crate/npm versions.

## Decisions

### Workspace
- A Cargo workspace at the root with one member, `backend/` (package and binary
  `crane`). Further crates are split out only when there's a concrete reason (e.g.
  compile times, a reusable provider crate).
- `backend/src/lib.rs` builds the app; `main.rs` is a thin CLI. Integration tests in
  `backend/tests/` drive the router in-process.
- Modules are created when their phase arrives (no empty placeholders).

### Toolchain
- `rust-toolchain.toml` pins `1.99.1` (current stable) with rustfmt and clippy.
  `rust-version = 1.94` in `Cargo.toml` is the minimum, set by SQLx 0.9's MSRV.

### HTTP stack
- Axum 0.8 + Tokio, tower-http for tracing and security headers.
- OpenAPI via `utoipa` 6 + `utoipa-axum` 0.3: routes are registered through
  `OpenApiRouter`, so a route cannot be added without appearing in the document.
- Swagger UI via `utoipa-swagger-ui` with the **`vendored`** feature. The default
  feature downloads Swagger UI from GitHub in `build.rs`; vendoring keeps builds
  offline-capable and reproducible on Windows.
- `--print-openapi` builds the document without config, database or network, for
  type generation (SPEC §13.3). A test asserts the served and printed documents match.

### Security headers
- A strict CSP (`script-src 'self'`, no inline scripts or styles) plus `nosniff`,
  `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and
  `frame-ancestors 'none'` are applied to every response from Phase 0 (SPEC §14.3).
- Exception: `/api/docs` allows inline **styles** (`style-src 'unsafe-inline'`),
  because Swagger UI's React components set `style` attributes. Scripts stay `'self'`.

### Configuration
- `figment` layers defaults → TOML file → `CRANE_*` env (nested with `__`) → CLI
  flags. Unknown keys are rejected (`deny_unknown_fields`) so typos fail fast.
- Data directory defaults to `%ProgramData%\CraneCamera` on Windows, `./data`
  elsewhere. Demo mode will use a separate `demo.db` in the same directory.

### Storage
- SQLx 0.9 with SQLite (bundled `libsqlite3-sys`, no system library needed), WAL,
  `foreign_keys = ON`, `synchronous = NORMAL`.
- Migrations embedded with `sqlx::migrate!` and run at startup. Before applying
  pending migrations to an existing file, a snapshot is written with `VACUUM INTO`
  (`<db>.bak-<unix-time>`).
- Phase 0 uses runtime-checked queries (`sqlx::query`, not `query!`). There is
  almost no SQL yet. Compile-time-checked queries and the committed `.sqlx/` offline
  cache (SPEC §16) are introduced in Phase 1 with the first real tables.

### Frontend embedding
- `rust-embed` with `allow_missing = true` embeds `frontend/dist` in release builds
  and reads it from disk in debug builds. If `index.html` is missing, a static
  placeholder page explains how to run the dev server, so `cargo build` never needs
  Node (SPEC §16).
- Unknown `/api/*` paths return problem+json 404; paths with a file extension
  that don't exist return 404; everything else returns `index.html` (SPA routing).

### Frontend toolchain
- Vite 8, React 19, Tailwind CSS 4 (via `@tailwindcss/vite`, no PostCSS config),
  TanStack Query 5, Vitest 5 + Testing Library, ESLint 10 flat config.
- `openapi-fetch` as the typed client over `openapi-typescript` output. The client
  resolves `fetch` per call so tests (and MSW later) can replace it.
- **TypeScript is pinned to 5.9.** TypeScript 7 (the native compiler) is out, but
  `openapi-typescript` and `typescript-eslint` still declare `typescript@^5` peer
  dependencies. Revisit when they support 7.
- Zustand and MSW are not installed yet; they arrive with the first feature that
  needs them.

### Errors
- Module errors use `thiserror`; `anyhow` only in `main.rs`.
- API errors are RFC 9457 problem+json with `type = urn:crane:problem:<code>` and a
  duplicated `code` field for convenience. `Internal` errors log their cause and
  return no detail.

## Consequences

- One binary, one process; plain `cargo build` works without Node.
- Adding an endpoint means annotating it, registering it with `routes!`, and
  regenerating `frontend/src/api/schema.d.ts` (CI checks for drift).
- New-major versions of several crates (utoipa 6, sqlx 0.9, tower-http 0.7) were
  adopted at their latest releases; APIs used so far match earlier majors closely.
