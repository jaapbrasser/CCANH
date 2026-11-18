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
| argon2 | Argon2id password hashing (SPEC §14.2) |
| secrecy | `SecretString`: redacted `Debug`, no `Serialize`, zeroize on drop (SPEC §4.6) |
| getrandom | OS randomness for session and setup tokens |
| sha2, base64 | Token hashing/encoding; certificate fingerprints |
| axum-extra (`cookie`) | Cookie jar extractor and cookie builder |
| time | Timestamps (RFC 3339 via `time::serde::rfc3339`), also the cookie `Max-Age` type. Chosen over chrono, which RUSTSEC-2026-0543 flags as unmaintained |
| uuid (v7) | IDs (SPEC §13.1) |
| axum-server (`tls-rustls-ring`) | HTTP/HTTPS serving with graceful shutdown |
| rustls (`ring`) | Server TLS without native build deps on Windows (ADR 0004). PEM parsing via `rustls-pki-types` (`rustls-pemfile` is archived, RUSTSEC-2025-0134) |
| rcgen (`ring`) | Self-signed certificate generation on first run |
| rpassword | Hidden password prompt for `--create-admin` |
| async-trait | `dyn CameraProvider` with async methods (SPEC §4.3) |
| bytes | Cheaply cloned media buffers shared by all viewers of a stream (ADR 0002) |
| reqwest (`rustls-no-provider`, `json`) | VAPIX HTTP client; TLS via our rustls config (ring), no aws-lc |
| rustls-platform-verifier | "Strict" camera TLS mode uses the operating-system trust store (incl. enterprise CAs on Windows) |
| md-5 | HTTP Digest MD5 (RFC 7616 / 2617); SHA-256 Digest uses `sha2` |
| aes-gcm | Development credential store (non-Windows) (ADR 0003) |
| windows-sys (Windows only) | DPAPI `CryptProtectData` for the credential store (ADR 0003) |
| futures-util | Splitting the WebSocket into sink and stream |
| *dev:* tempfile, http-body-util, figment (`test`), tokio-rustls, tokio-tungstenite, tokio (`test-util`), wiremock | Test helpers; TLS and WebSocket clients; paused-time tests; mock VAPIX server |

## Frontend (npm)

| Package | Why |
|---|---|
| react, react-dom | UI (SPEC §2) |
| @tanstack/react-query | Server state |
| react-router | Client-side routing and route guards |
| openapi-fetch | Typed fetch client over the generated schema |
| vite, @vitejs/plugin-react | Dev server and bundler |
| tailwindcss, @tailwindcss/vite | Styling |
| typescript (5.9, see ADR 0001) | Type checking |
| openapi-typescript | Generates `src/api/schema.d.ts` from the OpenAPI document |
| eslint, typescript-eslint, react-hooks / react-refresh plugins | Linting |
| vitest, @testing-library/*, jsdom | Unit and component tests |
| msw | API mocking in tests (SPEC §15.4) |

## Dev tools (not dependencies)

- `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features sqlite,rustls`):
  regenerates `.sqlx/` via `scripts/sqlx-prepare.ps1`.
- `cargo-deny`: supply-chain checks (`cargo deny check`).

## Known advisories

- **picomatch (GHSA-fc5q-39xc-vjpf, ReDoS)**: pulled in by Vite, Vitest and
  typescript-eslint via tinyglobby. No patched release exists yet (`npm audit`
  reports range `*`). Dev-time only, and it only processes glob patterns we write,
  so it isn't exploitable here. CI doesn't gate on `npm audit`. Recheck when a fix ships.
