# Dependencies

One line per non-trivial dependency and why it's here. See ADR 0001 for the baseline.

## Backend (Rust)

| Crate | Why |
|---|---|
| tokio | Async runtime (SPEC §2) |
| axum | HTTP framework (SPEC §2) |
| tower, tower-http | Middleware: request tracing, security headers |
| utoipa, utoipa-axum | OpenAPI generated from handler annotations |
| utoipa-swagger-ui (`vendored`) | Swagger UI at `/api/docs` without a build-time download |
| sqlx (sqlite) | SQLite access and embedded migrations; bundles SQLite, so no system library |
| figment | Layered config: defaults, TOML, env, CLI |
| clap | CLI parsing |
| serde, serde_json | Serialisation |
| thiserror / anyhow | Typed module errors / binary-edge error reporting |
| tracing, tracing-subscriber, tracing-appender | Structured logs, rolling daily files |
| tokio-util | `CancellationToken` for graceful shutdown |
| rust-embed, mime_guess | Embed `frontend/dist` in the release binary |
| *dev:* tempfile, http-body-util, figment (`test`) | Test helpers |

## Frontend (npm)

| Package | Why |
|---|---|
| react, react-dom | UI (SPEC §2) |
| @tanstack/react-query | Server state |
| openapi-fetch | Typed fetch client over the generated schema |
| vite, @vitejs/plugin-react | Dev server and bundler |
| tailwindcss, @tailwindcss/vite | Styling |
| typescript (5.9, see ADR 0001) | Type checking |
| openapi-typescript | Generates `src/api/schema.d.ts` from the OpenAPI document |
| eslint, typescript-eslint, react-hooks / react-refresh plugins | Linting |
| vitest, @testing-library/*, jsdom | Unit and component tests |

## Known advisories

- **picomatch (GHSA-fc5q-39xc-vjpf, ReDoS)**: pulled in by Vite, Vitest and
  typescript-eslint via tinyglobby. No patched release exists yet (`npm audit`
  reports range `*`). Dev-time only, and it only processes glob patterns we write,
  so it isn't exploitable here. CI doesn't gate on `npm audit`. Recheck when a fix ships.
