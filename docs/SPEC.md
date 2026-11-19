# Crane Camera Automated Navigation Hub (CCANH) — Project Specification

> **For Claude Code:** This is the authoritative spec for the project. Read sections 0–3 in full before writing any code. Then read the section for the phase you're working on (see §17). Don't build ahead of the current phase.

---

## 0. How to work on this project

### 0.1 Working agreement

1. **Work in phases (§17).** Finish a phase, including its tests and acceptance criteria, before starting the next. Don't scaffold empty modules for future phases.
2. **Plan before large changes.** Before starting a phase, or any change touching more than ~5 files, write a short plan: files, approach, risks. If the user is present, wait for their OK.
3. **Record decisions.** Any significant technical choice (a new dependency, a streaming protocol, a storage layout, a security mechanism) gets a short ADR in `docs/adr/NNNN-title.md`: context, options considered, decision, consequences. §16 lists the ADRs this spec already expects.
4. **When the spec is ambiguous:** if the choice is cheap to reverse, pick the most reasonable option, record it (ADR or a code comment), and keep going. If it's expensive to reverse, such as a schema shape that gets persisted, a security model, or a public API contract, stop and ask.
5. **Open questions (§18) have stated defaults.** Use the default and don't block on them. Flag anything you built that depends on one.
6. **Small, reviewable commits.** Use Conventional Commits (`feat:`, `fix:`, `test:`, `docs:`, `refactor:`, `chore:`). One logical change per commit. Never commit secrets, real camera IPs or real credentials.
7. **Tests ship with the code.** A feature isn't done until it has tests (§15).
8. **Don't invent VAPIX endpoints.** Every AXIS endpoint you use must be checked against the official docs and logged in `docs/vapix.md` (§4.4).

### 0.2 Definition of done (every task)

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] Frontend: `npm run lint`, `npm run typecheck` and `npm test` pass (once the frontend exists)
- [ ] New or changed API endpoints are annotated, so they show up in the OpenAPI document
- [ ] Frontend API types are regenerated and nothing has drifted (§13.3)
- [ ] No `unwrap()`/`expect()` in non-test code, except where an invariant is truly guaranteed (add a comment saying why)
- [ ] No credentials or secrets in logs, API responses, errors or `Debug` output
- [ ] Docs are updated where behaviour changed (`docs/`, `CLAUDE.md`)

### 0.3 CLAUDE.md

Phase 0 creates a short `CLAUDE.md` at the repo root holding the **real** build, test and run commands, the layout as it actually exists, and the conventions above. Keep it under ~150 lines and point to this spec rather than copying it. Update it whenever commands or structure change.

---

## 1. Project overview

Build a **Windows-hosted, Rust-based IP camera management platform** for managing AXIS network cameras.

The application runs as a server on a Windows machine and serves a web frontend that users open in a browser. It supports multiple simultaneous users, centralized camera connections, per-user configurable dashboards and background camera monitoring.

The main goal is a single interface for viewing and managing a large collection of IP cameras **while minimizing bandwidth on the camera network**.

The application must include a fully functional **demo mode** that shows the interface and management features with no physical cameras and no camera network.

### 1.1 Core principles

1. **Rust-first backend.**
2. **API-first:** every feature is reachable through documented API endpoints. The frontend has no privileged back doors.
3. **Never block:** network requests, camera operations, polling and logging never block request handling or the UI.
4. **Centralized streaming:** viewers of the same camera stream share one upstream camera connection.
5. **Modular:** camera providers, streaming, auth, storage and frontend can each be maintained on their own.
6. **Demo and production share code paths:** the same binary, the same interfaces and the same streaming pipeline. Only the provider differs.
7. **Secure by default:** camera credentials and user authentication are designed in from Phase 0, not bolted on later.
8. **Tests accompany implementation.**

### 1.2 Non-goals (MVP)

- No recording or archiving of video. Only transient, bounded in-memory buffering.
- No PTZ control, I/O ports, event rules or camera configuration changes. The MVP is read-only toward cameras.
- No transcoding in the default path.
- No camera vendors other than AXIS. The provider abstraction should allow them later.
- No access from outside the local network or through NAT (see §18).
- No mobile-specific UI. Desktop browsers come first.

---

## 2. Technology stack

| Component | Choice | Notes |
|---|---|---|
| Backend | Rust (stable, edition 2021 or later) | Pin the toolchain in `rust-toolchain.toml` |
| Async runtime | Tokio | |
| HTTP framework | Axum | |
| OpenAPI | `utoipa` (+ `utoipa-axum` if suitable) | Generated from code annotations |
| API docs UI | Swagger UI (`utoipa-swagger-ui`) | Served at `/api/docs` |
| Real-time | WebSockets (Axum built-in) | |
| Serialization | Serde | |
| Database | SQLite via SQLx | WAL mode; migrations with `sqlx::migrate!` |
| Analytics store | DuckDB — **deferred to Phase 9** | See §11.3 |
| Password hashing | Argon2id (`argon2` crate) | |
| Secret handling | `secrecy` crate (or equivalent) | Secrets never appear in `Debug` output |
| Logging/tracing | `tracing`, `tracing-subscriber`, `tracing-appender` | |
| Frontend asset embedding | `rust-embed` (or equivalent) | |
| Frontend | React + TypeScript + Vite | |
| Styling | Tailwind CSS | |
| Server state (frontend) | TanStack Query | |
| UI state (frontend) | Zustand (or React context while it stays small) | |
| Drag and drop / grid | `dnd-kit` or `react-grid-layout` | Decide in an ADR during Phase 7 |
| Frontend tests | Vitest + Testing Library | Playwright e2e later |
| API types for frontend | `openapi-typescript` generated from the OpenAPI document | |
| Dev scripts | PowerShell 7 (`pwsh`), cross-platform | In `scripts/` |
| Deployment | One Windows `.exe` | Installer is optional (Phase 9) |

**Dependency rules**
- Before adding a crate or npm package, check that it is maintained (recent releases, active issues). Prefer widely used options.
- Commit `Cargo.lock` and `package-lock.json`.
- Note each non-trivial dependency in an ADR or in `docs/dependencies.md` with a one-line reason.
- Avoid crates that need system libraries on Windows unless they are vendored or bundled. Plain `cargo build` on a fresh Windows machine with the MSVC toolchain must work.

**Runtime requirements:** end users install nothing: no Rust, no Node, no ffmpeg, no runtimes. The frontend is built and embedded into the Rust binary, which serves the UI and the API from one process.

**Development:** run the frontend on its own with the Vite dev server, proxying `/api` and WebSocket traffic to the backend.

---

## 3. Architecture

### 3.1 High-level

```text
            Browsers (multiple users)
                     │  HTTPS / WSS
        ┌────────────┴─────────────┐
        │  Axum server             │  static UI, REST API, WebSocket, stream egress
        ├──────────────────────────┤
        │  Application core        │  auth, users, cameras, dashboards,
        │                          │  groups/tags/favorites, tasks, events
        ├──────────────────────────┤
        │  Background services     │  polling, discovery, stream manager,
        │                          │  event bus, logging
        ├──────────────────────────┤
        │  CameraProvider trait    │
        │   ├─ AxisProvider ───────┼──► AXIS cameras (VAPIX / RTSP)
        │   └─ DemoProvider ───────┼──► simulated cameras + bundled clips
        ├──────────────────────────┤
        │  Persistence             │  SQLite (+ DuckDB later), filesystem
        └──────────────────────────┘
```

### 3.2 Internal communication

- **Event bus:** a typed internal event channel (for example `tokio::sync::broadcast`) carrying domain events such as camera status changes, task progress and discovery results. The WebSocket layer subscribes to it and forwards events to clients the user is authorized to see them.
- **Shared state:** `AppState`, built once at startup and passed through Axum `State`. It holds repositories, managers and the provider factory behind traits, so tests can swap in fakes.
- **Cancellation:** long-running work takes a `CancellationToken` (`tokio-util`). Graceful shutdown cancels the root token and waits, with a timeout, for tasks to finish.

### 3.3 Repository layout

Use a Cargo workspace. Create directories when the functionality arrives, not before.

```text
crane-camera-platform/
├── Cargo.toml              # workspace
├── Cargo.lock
├── rust-toolchain.toml
├── CLAUDE.md
├── README.md
├── docs/
│   ├── SPEC.md             # this file
│   ├── adr/
│   ├── architecture.md
│   ├── api.md
│   ├── database.md
│   ├── development.md
│   ├── vapix.md            # verified VAPIX endpoints used (§4.4)
│   └── dependencies.md
├── backend/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs         # CLI parsing, startup, shutdown
│   │   ├── lib.rs          # builds the app, so tests can run it in-process
│   │   ├── api/            # routes, handlers, middleware, websocket, error mapping
│   │   ├── auth/
│   │   ├── users/
│   │   ├── cameras/        # registry, manager, models, capabilities
│   │   ├── providers/      # traits.rs, axis/, demo/
│   │   ├── streaming/      # manager, sessions, gop buffer, egress
│   │   ├── polling/
│   │   ├── discovery/
│   │   ├── tasks/          # background task manager
│   │   ├── events/
│   │   ├── dashboards/
│   │   ├── organization/   # groups, tags, favorites
│   │   ├── credentials/    # CredentialStore trait + DPAPI impl
│   │   ├── storage/        # sqlite pool, repositories
│   │   ├── config/
│   │   └── telemetry/      # tracing setup, redaction
│   ├── migrations/         # SQLite migrations (sqlx)
│   └── tests/              # integration tests (Cargo only finds them here)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   └── src/ (components, pages, layouts, hooks, api, stores, types, utils)
├── assets/demo/            # small bundled demo video clips (§12.3)
└── scripts/                # build.ps1, dev.ps1, gen-api-types.ps1, …
```

> Integration tests live in `backend/tests/`, not a root `tests/` folder, because Cargo only discovers integration tests inside a crate.

### 3.4 Configuration

- A TOML config file (default `config.toml` next to the exe, overridable with `--config`), plus `CRANE_*` environment variable overrides, plus CLI flags. Precedence: CLI > env > file > defaults.
- Validate at startup and fail fast with clear messages.
- The data directory (SQLite DB, logs, keys) is configurable and defaults to `%ProgramData%\CraneCamera\` on Windows and `./data/` elsewhere.
- Some settings are editable at runtime by admins through the API (polling interval, timeouts, discovery limits, stream grace period). They are stored in SQLite, and the startup config supplies the defaults.

### 3.5 Error handling

- Each module defines its own error enum with `thiserror`. Use `anyhow` only at the binary edge.
- API errors use RFC 9457 `application/problem+json` with a stable `type`/code, a human-readable `title`/`detail`, and no internal or secret details.
- Provider errors are categorized (§10.4). `Unsupported` is a normal result, not a failure.

---

## 4. AXIS camera integration

### 4.1 Source of truth

Integrate through **VAPIX**: https://developer.axis.com/vapix/

The official AXIS documentation is authoritative for endpoints, authentication, capabilities and protocols. **Never invent an endpoint or assume every AXIS model supports the same features.** If the docs can't be reached, stop and ask rather than guessing.

### 4.2 Initial functionality (Phase 4)

Read-only operations:

- Connect by IP address or hostname, with an optional port and HTTP/HTTPS.
- Authenticate with the camera's configured credentials.
- Get device identification: manufacturer, model, firmware, serial number, device name and hostname, where available.
- Get available capabilities (supported APIs and their versions) through the documented capability or API-discovery mechanisms.
- Get basic network and device information where supported.
- Check availability and connectivity, measuring response time.
- Get available video streaming capabilities: codecs, resolutions, stream profiles.
- Get basic status.
- When a feature is unsupported, record it and carry on. Never fail the whole integration.

### 4.3 Provider abstraction

One provider instance per registered camera. The application core depends only on the trait.

```rust
#[async_trait]   // needed while providers are used as `dyn CameraProvider`
pub trait CameraProvider: Send + Sync {
    async fn probe(&self) -> Result<ProbeResult, ProviderError>;           // reachability + identity
    async fn get_device_info(&self) -> Result<DeviceInfo, ProviderError>;
    async fn get_status(&self) -> Result<CameraStatus, ProviderError>;
    async fn get_capabilities(&self) -> Result<CameraCapabilities, ProviderError>;
    async fn get_stream_info(&self) -> Result<Vec<StreamProfile>, ProviderError>;

    /// Open an upstream media source. Called ONLY by the stream manager.
    async fn open_stream(
        &self,
        profile: &StreamProfileId,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError>;
}

pub enum ProviderError {
    Unsupported { feature: String },
    Unreachable(..), Timeout(..), AuthFailed(..), Tls(..),
    Protocol(..), RateLimited(..), Internal(..),
}
```

- `MediaSource` yields timestamped, codec-tagged media units (for example H.264 access units with keyframe flags) plus codec parameters (SPS/PPS). The exact shape is decided in the streaming ADR (§7.3).
- Explicit `connect()`/`disconnect()` from the original draft were dropped. HTTP is connectionless, and stream lifetimes belong to the stream manager. Add them back if a real need comes up.
- A **ProviderFactory** builds the right provider from a camera record plus the mode (production or demo).
- Extend the trait as features arrive, and keep the demo provider in step with it (§12).

### 4.4 VAPIX endpoint log

Keep `docs/vapix.md` up to date. For each endpoint used, record: purpose, method and path, auth requirement, the doc URL it was checked against, the API version and a minimum firmware note if known, and the fixture file used in tests.

### 4.5 Camera authentication and transport

- Credentials are configured per camera, and a **named credential set** can be reused across many cameras.
- Support HTTP Digest and Basic as the camera requires. Detect the scheme from the `WWW-Authenticate` challenge and never assume. Prefer Digest when offered. Basic over plain HTTP is a per-camera setting; new cameras get it from the admin-editable camera defaults, shipped on for ease of use on a closed network (ADR 0011).
- Use HTTPS when the camera supports it. Certificate validation is configurable **per camera**: `strict`, `pinned` (trust a specific certificate fingerprint recorded at registration) or `insecure-skip-verify`; new cameras get the mode from the camera defaults setting (ADR 0011). `insecure-skip-verify` is shown in the UI with a warning and logged.
- RTSP credentials follow the same rules. Never put credentials in a URL that could be logged.

### 4.6 Credential storage

- A `CredentialStore` trait encrypts credentials before they reach SQLite. The database holds only ciphertext.
- **Windows implementation:** DPAPI (`CryptProtectData`). Scope (user or machine) depends on how the app runs (§18 Q2). Default to **machine scope with app-specific entropy**, so it keeps working if the app later runs as a Windows service. Record the decision in an ADR.
- **Non-Windows (dev/CI only):** AES-GCM with a key file in the data directory. On startup it logs a clear warning that it is not a production credential store.
- Credentials are write-only through the API. Responses show only `has_credentials: true` and the credential set's name.
- Wrap all secret values in a `Secret` type so `Debug`/`Display` never print them. Add a test proving that log output for a camera operation contains no password.

---

## 5. Camera discovery (Phase 8)

### 5.1 Requirements

- Admins define one or more IP ranges in CIDR notation (single IPs and dash ranges are nice to have).
- **Safety caps:** at most 4,096 addresses per job by default (about a /20). Admins can raise it with an explicit confirmation.
- Scans run asynchronously as background tasks (§10.3), with configurable concurrency (default 32) and timeouts (TCP connect default 1 s, HTTP default 3 s).
- Scan politely: a quick TCP connect check on the HTTP/HTTPS ports before any HTTP request, and at most one identification request per responsive host.
- Identify AXIS devices with documented VAPIX identification, preferring an endpoint that **doesn't need credentials** if the docs confirm one exists. Verify and record it in `docs/vapix.md`.
- Optionally, mDNS/SSDP/WS-Discovery listeners for the local subnet. These don't cross subnets, so CIDR scanning remains the main mechanism.
- Results are **never auto-registered**. Admins review and approve them.
- Detect duplicates by serial number or MAC (preferred), then by IP or hostname. Mark devices that are already registered.
- Jobs can be started, monitored (progress, hosts scanned, devices found) and cancelled.
- Results stream to the UI over WebSocket as they're found. Discovery must never block the API.

---

## 6. Camera management interface

### 6.1 Camera list

Columns: name, IP or hostname, manufacturer, model, online status, last successful communication, last status change, streaming status (with subscriber count), groups, tags and favorite status.

Features: search (name, IP, model), filters (status, group including subgroups, tag, favorites), sorting, server-side pagination, manual refresh, row click to open details, and bulk selection for tagging, grouping, enabling or disabling monitoring, and deleting. Bulk delete needs confirmation.

The list updates live over WebSocket. Refresh doesn't have to be manual.

### 6.2 Registration (admin)

- Add manually or from a discovery result.
- Friendly name, host, port, scheme, TLS mode, a credential set (existing or new), groups, tags, and a monitoring on/off switch.
- **Test connection** runs a probe without saving and shows identity, capabilities and any errors.
- Remove a camera. This stops its streams, polling and history according to the retention policy.

### 6.3 Camera details

Shows everything the implemented integration provides: identity, model, firmware, network info, connectivity, capabilities (supported and unsupported), stream profiles, last successful communication, a status history timeline, the live view, groups and tags.

Build the page from **sections driven by capability data**, so new VAPIX features become new sections without a redesign.

---

## 7. Video streaming (critical)

### 7.1 Primary requirement

**One upstream connection per (camera, stream profile), no matter how many viewers.**

```text
AXIS camera ──(one RTSP session per profile)──► Stream Manager ──► N browser subscribers
```

### 7.2 Stream lifecycle

1. A user requests the stream for `(camera_id, profile)`.
2. The stream manager looks up the entry.
3. **Running:** subscribe the user. They start from the most recent keyframe in the GOP buffer, so playback begins immediately.
4. **Starting:** the user waits on the same in-flight start. No second upstream connection is opened.
5. **Absent:** create the entry in `Starting` state *atomically*, then call `provider.open_stream`.
6. Fan media out to all subscribers.
7. Track subscribers. When the last one leaves, enter `Draining` for a configurable grace period (default 15 s).
8. If someone subscribes during `Draining`, cancel the shutdown and go back to `Running`.
9. When the grace period ends, close the upstream connection and free its buffers.
10. If the upstream connection fails, reconnect with backoff while subscribers remain, and tell subscribers about the state (`reconnecting`, `failed`).

**Concurrency requirement (tested):** 50 concurrent subscribe requests for the same camera produce **exactly one** `open_stream` call.

### 7.3 Protocol selection — Phase 3 spike, then ADR

Browsers can't play RTSP. Pick the browser delivery path in a **time-boxed spike** (a throwaway branch, not production code) and record it in `docs/adr/0002-streaming-transport.md`.

Candidates:

| Option | Upstream | Browser | Pros | Cons |
|---|---|---|---|---|
| **A. fMP4 over WebSocket → MSE** | RTSP (H.264 passthrough) | Media Source Extensions | No transcoding, simple server, no ICE/STUN, fits LAN | ~0.5–2 s latency; MSE missing on some mobile browsers |
| **B. WebRTC** | RTSP (H.264 passthrough via RTP repacketizing) | `RTCPeerConnection` | Lowest latency, handles packet loss well | Much more complex (signaling, ICE, DTLS) |
| **C. MJPEG over HTTP** | VAPIX MJPEG or RTSP MJPEG | `<img>` multipart | Trivial, works everywhere | High bandwidth; no H.264 savings |
| **D. HLS / LL-HLS** | RTSP | `<video>` / hls.js | Very compatible | High latency, segmenting work |

Rust building blocks to evaluate: `retina` (RTSP client), `str0m` or `webrtc-rs` (WebRTC), and an fMP4 muxer (an existing crate or a small hand-written one).

**Spike acceptance:** against a real AXIS camera, or a local RTSP test source if none is available (§15.5), show one upstream connection playing in two or more browser tabs at once, and measure CPU, latency and bandwidth. The ADR must cover: chosen default, fallback path, codec support (H.264 required; treat H.265 as unsupported in the MVP because browser support varies), and how the demo provider feeds the same pipeline.

**Suggested default if the spike is inconclusive:** A (fMP4/MSE) as the main path, C (MJPEG) as a fallback for cameras or browsers that can't use A, and B later as an upgrade behind the same subscriber interface.

Transcoding is never part of the default path. If ever added, it's an optional, per-camera capability.

### 7.4 Stream profiles and bandwidth

- **Grid tiles use a low-resolution profile; the expanded or detail view uses the main profile.** This is the biggest bandwidth lever in the system: 16 full-resolution H.264 decodes will also overload most browser clients.
- Profile choices come from camera capabilities. Admins set the default "grid" and "detail" profiles globally, with per-camera overrides.
- Switching a tile between grid and detail is a subscribe/unsubscribe on a different `(camera, profile)` key. Both stay shared.

### 7.5 Buffering

- Each upstream stream has a bounded **GOP buffer**: the latest keyframe plus following frames, capped by bytes (configurable, default 4 MB per stream) and a global cap across all streams (default 256 MB).
- Fan-out must never let a slow subscriber stall the others. If a subscriber falls behind past the bound, drop that subscriber's queue and **resync it at the next keyframe**. Never send partial GOPs that corrupt decoding.
- Brief subscriber disconnects (network blips) recover by resubscribing. With the grace period, the upstream connection stays up.
- No footage is written to disk. Buffering is not recording, and code and docs must keep the two clearly separate.

### 7.6 Metrics and admin overview

Track for each stream: subscribers, upstream state, startup time (request to first keyframe sent), inbound bytes/s, outbound bytes/s (total and per subscriber), error and reconnect counts, and buffer usage.

The admin **Streams** page lists active streams with these metrics and lets an admin force-stop a stream.

Design the stream manager so per-camera or global bandwidth limits and priorities can be added later. Leave a policy hook and don't implement it yet.

---

## 8. Dashboards (Phase 7)

### 8.1 Management

Each user has their own dashboards: create, rename, delete, set a default, switch between them. All persisted. A user's dashboards are private to them; sharing is out of MVP scope.

### 8.2 Camera grid

- Presets: 1 (1×1), 2 (1×2), 4 (2×2), 6 (2×3), 9 (3×3), 12 (3×4), 16 (4×4), plus a custom row × column grid (max 8×8).
- Tiles hold a camera or nothing. Drag cameras between tiles, swap, replace, remove.
- Click to expand a tile into the camera detail view (§8.4). Dashboard fullscreen uses the browser Fullscreen API.
- Layout changes save automatically with a debounce. Saves are optimistic, with rollback if the save fails.
- **Changing the layout must not restart streams that are still visible.** Keep subscriptions keyed by `(camera, profile)` in a frontend stream registry, not by tile component instance.
- Tiles that are off-screen or in a hidden browser tab pause their subscription after a short delay to save bandwidth (Page Visibility API).

### 8.3 Camera picker

Searchable by name and IP, filterable by group (including subgroups), tag and favorites. Marks cameras already on the current dashboard.

### 8.4 Expanded camera view

Live stream (detail profile), name, status, IP, model, firmware, device info, capabilities, stream status and other available data. Has a clear way back to the dashboard (button and Esc key).

---

## 9. Groups, tags and favorites (Phase 6)

- **Groups** (global, admin-managed): hierarchical with unlimited depth (store as adjacency list; prevent cycles). Create, rename, delete, move. Deleting a group requires choosing what happens to children and cameras (move up or detach). A camera can be in **multiple groups** (assumed; see §18 Q4). Filtering by a group includes its descendants.
- **Tags** (global, admin-managed): create, rename, delete, assign many per camera, add and remove in bulk, filter by tag. Tag names are unique and case-insensitive.
- **Favorites** (per user): add or remove, filter, dedicated view, persisted. Never visible to or affecting other users.

Example hierarchy:

```text
Site A
 ├─ Building 1 ─ Crane 1, Crane 2
 └─ Building 2 ─ Crane 3, Crane 4
Site B
 ├─ Loading Area
 └─ Storage Area
```

---

## 10. Background processing and polling

### 10.1 Rules

All camera communication is asynchronous. No camera request blocks the API, other cameras or the UI. Background work keeps running whether or not anyone has the frontend open.

### 10.2 Polling (Phase 4)

- Poll each **monitored** camera every 10 s by default (an admin setting).
- In the MVP, active and inactive cameras use the same policy. Model polling as a `PollingPolicy` so different intervals for streamed and idle cameras can be added later. A camera counts as "active" when it has at least one stream subscriber.
- Add **jitter** (±20% by default) so a large fleet doesn't send requests in lockstep.
- Cap in-flight polls globally with a semaphore (default 32). Skip a camera's poll if its previous one is still running.
- Use configurable timeouts and bounded retries with exponential backoff and jitter. After N consecutive failures (default 3), mark the camera offline. Keep polling offline cameras at a slower backoff interval (default up to 60 s).
- Collect: online/offline, last success, last failure, response time, consecutive failure count, last status change.
- **Persist status transitions, not every poll.** Keep current status in the `cameras` row or a `camera_status` table. Write a history row only when the state changes, plus an optional periodic summary (response-time aggregates) for history charts.
- Publish status changes on the event bus, which pushes them to the WebSocket.

### 10.3 Background task manager

Every long-running operation (discovery, bulk operations, connection tests, stream starts if useful) is a tracked task with: UUID, type, optional camera ID, the user who created it, created/started/completed timestamps, status (`queued | running | succeeded | failed | cancelled`), progress (0–100 or n/m), and a categorized error.

- `GET /api/v1/tasks`, `GET /api/v1/tasks/{id}` and `POST /api/v1/tasks/{id}/cancel`.
- Progress and completion are pushed over WebSocket, and the frontend shows toast notifications.
- Persist task records in SQLite. On startup, mark tasks that were `running` as `failed (interrupted)`.
- Recurring polling is **not** a task per poll. It's a supervised service with its own metrics.

### 10.4 Failure handling

- Request timeouts on every network call. No unbounded waits anywhere.
- Retry limits and exponential backoff with jitter.
- Cancellation for long-running operations through `CancellationToken`.
- **Error categories:** `unreachable`, `timeout`, `auth_failed`, `tls_error`, `unsupported`, `protocol_error`, `rate_limited`, `internal`. Categories appear in the API and UI and drive retry policy: for example, never retry `auth_failed` in a tight loop, because repeated attempts can lock out camera accounts.
- Structured logging with camera ID, task ID, operation and error category as fields (§11).
- Service supervision: if a background service panics or exits, log it and restart it with backoff, and expose its health at `/api/v1/health`.

---

## 11. Logging, auditing and analytics

> The original draft was cut off partway through §10 (at "Structured"). Sections 11–18 were rebuilt from the intent in the rest of the document. The owner should review them.

### 11.1 Application logs

- `tracing` with structured fields. Console output in dev, rolling daily files in the data directory in production (`tracing-appender`), with configurable retention (default 14 days).
- Log level set by config, overridable with `RUST_LOG`.
- **Redaction:** secrets are kept out by type (`Secret<T>`), and a test confirms it. Never log request bodies on credential endpoints, `Authorization` headers or RTSP URLs with credentials.

### 11.2 Audit log

An append-only `audit_log` table for security-relevant actions: logins (success and failure), user and role changes, camera create/update/delete, credential changes, discovery jobs, settings changes, and force-stopped streams. Records actor, action, target, timestamp and source IP. Admins can view it in the UI, filtered and paginated.

### 11.3 Analytics store (DuckDB) — deferred

The original stack uses DuckDB for analytical logging. **Defer it to Phase 9.** It adds a large native build dependency (slow compiles, bigger binary on Windows), and the MVP's history needs fit easily in SQLite.

- In the MVP, status history and stream metrics go to SQLite behind a `MetricsStore` trait, with retention (default 30 days) and a daily cleanup job.
- In Phase 9, decide in an ADR whether DuckDB still adds enough (for example, long-term fleet analytics) to justify the cost. If so, add it as a second `MetricsStore` implementation.

---

## 12. Demo mode

### 12.1 Behaviour

- Start with `--demo` (or `mode = "demo"` in config). The UI shows a persistent **DEMO MODE** banner.
- Demo mode uses a **separate SQLite file** (`demo.db`), freshly seeded on each start by default (`--demo-keep-state` keeps it). Demo data must never mix with production data.
- Seeds demo users (`admin`/`viewer` with documented demo passwords, demo mode only), about 24 cameras across a realistic group hierarchy (the Site A / Site B example), tags, favorites and two sample dashboards.
- Works with no network at all. It must not open any outbound connection.

### 12.2 Simulated behaviour (DemoProvider)

The DemoProvider implements the full `CameraProvider` trait with configurable, deterministic (seeded) behaviour:

- A variety of models and firmware versions with **different capability sets**, including some unsupported features, to exercise the "Unsupported" paths.
- Realistic response-time jitter.
- Some cameras always online, some always offline, one that flaps, and one with auth failures.
- **Discovery simulation:** scanning a demo range "finds" unregistered demo devices progressively, with realistic timing and cancellation.

### 12.3 Demo video

Demo streams go through **the same stream manager and egress pipeline** as real streams. Only the `MediaSource` differs.

- Bundle a few short, looping H.264 clips (a few seconds each, low bitrate, a few MB total) in `assets/demo/` and embed them in the binary.
- Generate the clips yourself, for example with ffmpeg `testsrc2` plus an overlaid camera name and timestamp, so there are no licensing issues. Commit the generation script. ffmpeg is a dev-only tool and never needed at runtime.
- The demo `MediaSource` replays the clip in real time with correct timestamps, so shared-upstream behaviour, subscriber counts and bandwidth metrics are all real and visible in demo mode.

### 12.4 Parity rule

Any new `CameraProvider` method gets a DemoProvider implementation in the same change. The provider contract test suite (§15.2) runs against both.

---

## 13. API

### 13.1 Conventions

- REST under `/api/v1/`. JSON bodies. `snake_case` fields. RFC 3339 timestamps in UTC. UUIDs (v7 preferred) for IDs.
- Errors: RFC 9457 problem+json (§3.5).
- Pagination: `?page=&page_size=` (max 200), response includes `total`. Sorting: `?sort=field&order=asc|desc`.
- OpenAPI JSON at `/api/openapi.json`, Swagger UI at `/api/docs`. Admins only in production, unless config opens it up.
- Every endpoint declares its required role in its OpenAPI description.

### 13.2 WebSocket

- One multiplexed connection: `/api/v1/ws`. Authenticated with the session cookie, and the `Origin` header is checked.
- Typed JSON messages with a `type` field. Clients subscribe to topics (`camera_status`, `tasks`, `discovery:{job_id}`, `streams`). The server only sends events the user is allowed to see.
- Heartbeat ping/pong. The client reconnects with backoff and refetches state after reconnecting.
- Media streaming uses its own endpoints (per the streaming ADR), not the control WebSocket.

### 13.3 Frontend type generation

`scripts/gen-api-types.ps1` exports the OpenAPI document (through a CLI flag like `--print-openapi`, with no server needed) and runs `openapi-typescript` to produce `frontend/src/api/schema.d.ts`. CI regenerates it and fails if the committed file differs.

---

## 14. Users, authentication and authorization (Phase 1)

### 14.1 Roles

- **Admin:** everything, including users, cameras, credentials, discovery, groups, tags, settings, the streams overview and the audit log.
- **Viewer:** view cameras and streams, manage their own dashboards and favorites, use groups and tags for filtering.
- Put role checks in one place (an Axum extractor or middleware), not scattered through handlers. Write the authorization layer so per-group camera access can be added later (see §18 Q4).

### 14.2 Authentication

- Local username and password accounts. Argon2id hashing with sensible parameters.
- **First-run bootstrap:** if no users exist, `crane.exe --create-admin <username>` prompts for a password interactively. Alternatively, a one-time setup token is printed to the console and the logs, and it unlocks a setup page. No default production credentials, ever.
- Server-side sessions in SQLite. Cookie is `HttpOnly`, `Secure` when served over TLS, `SameSite=Strict`. Idle timeout (default 8 h) and absolute timeout (default 7 d), both configurable.
- CSRF protection for state-changing requests (SameSite plus an `Origin`/`Referer` check, or a double-submit token).
- Login rate limiting per username and per IP, with exponential delay. Failed logins are written to the audit log.
- Admins can create, disable and delete users, reset passwords and change roles. Users can change their own password.
- The last enabled admin can't be deleted or demoted.

### 14.3 Server transport security

- Users reach the server over the network, so **server-side TLS** is supported from Phase 1: a configured certificate and key (PEM) through `rustls`, or an optional self-signed certificate generated on first run (with the fingerprint shown so users can verify it).
- Plain HTTP is allowed, with a startup warning, when binding to a non-loopback address. Bind address and port are configurable.
- Security headers: CSP (strict, no inline scripts), `X-Content-Type-Options`, `Referrer-Policy`, `frame-ancestors 'none'`.

---

## 15. Testing

### 15.1 Backend

- **Unit tests** next to the code.
- **Integration tests** in `backend/tests/`. They build the app in-process from `lib.rs` with a temporary SQLite database and call it through `tower::ServiceExt::oneshot` or a test HTTP client. No real network needed.
- **AxisProvider tests** run against a mock HTTP server (`wiremock` or similar) loaded with **recorded VAPIX response fixtures** in `backend/tests/fixtures/vapix/`, covering Digest auth challenges, unsupported-API responses, timeouts, malformed responses and TLS modes.

### 15.2 Provider contract suite

A single generic test suite that any `CameraProvider` must pass (identity, capabilities, the `Unsupported` path, timeouts, stream open and close). It runs against DemoProvider and against AxisProvider-with-wiremock.

### 15.3 Critical-path tests (required)

- Stream manager: N concurrent subscribers → exactly 1 `open_stream`. Grace-period cancel and resubscribe. Slow subscriber doesn't stall others and resyncs at a keyframe. Global buffer cap is respected.
- Polling: jitter bounds, only one in-flight poll per camera, status transitions persisted only on change, no tight retry loop on `auth_failed`.
- Discovery: CIDR parsing and size caps, cancellation, deduplication.
- Auth: role enforcement on every admin route (generated from the route table where possible), session expiry, CSRF, login rate limiting.
- Secrets: credentials never appear in API responses, logs or `Debug` output.

### 15.4 Frontend

Vitest and Testing Library for components and hooks. Mock the API with MSW. Add Playwright end-to-end tests against the binary in demo mode by Phase 7 at the latest.

### 15.5 Local test sources

If no AXIS camera is available, use a local RTSP test server (for example MediaMTX fed by ffmpeg `testsrc2`) for streaming development. Document the setup in `docs/development.md`. This is a dev tool only.

### 15.6 CI (GitHub Actions)

Matrix: `windows-latest` (required, it's the target) and `ubuntu-latest` (fast feedback). Jobs: fmt, clippy, tests, frontend lint/typecheck/test, API type drift check, a release build of the embedded single binary on Windows, and `cargo audit` or `cargo deny`.

---

## 16. Build, packaging and deployment

- `scripts/build.ps1`: builds the frontend (`npm ci && npm run build`), then `cargo build --release`, which embeds `frontend/dist`. Output is one `crane.exe`.
- In debug builds, the backend may serve `frontend/dist` from disk. Builds must not fail if `frontend/dist` is missing in dev: serve a placeholder page that says "run the frontend dev server".
- Don't call npm from `build.rs`. Keep the Rust build independent of Node.
- SQLx: use `sqlx prepare` (offline mode, `.sqlx/` committed) so the backend compiles without a live database.
- Startup runs SQLite migrations automatically. Back up the DB file before applying migrations to an existing database.
- Phase 9: optional Windows service mode (install, uninstall, run), optional installer (for example WiX or Inno Setup), code-signing notes.

**Expected ADRs**
- 0001: Workspace layout and dependency baseline (Phase 0)
- 0002: Streaming transport (Phase 3 spike)
- 0003: Credential storage and DPAPI scope (Phase 4)
- 0004: Session and CSRF approach (Phase 1)
- 0005: Grid / drag-and-drop library (Phase 7)
- 0006: DuckDB go/no-go (Phase 9)

---

## 17. Phased delivery plan

Each phase ends with its acceptance criteria met, CI green, and a short summary (what was built, deviations from the spec, follow-ups).

| Phase | Scope | Acceptance criteria |
|---|---|---|
| **0. Foundation** | Workspace, config, tracing, error model, SQLite and migrations, `/api/v1/health`, OpenAPI and Swagger skeleton, Vite + React + Tailwind shell embedded in the binary, dev proxy, CI, `CLAUDE.md`, ADR 0001 | `crane.exe` built on Windows serves the UI shell and `/api/v1/health` from a single process. CI green on both OSes. `CLAUDE.md` lists working commands. |
| **1. Auth & users** | Users, roles, Argon2id, sessions, CSRF, rate limiting, bootstrap admin, server TLS, audit log (basic), login UI, user admin UI, ADR 0004 | Role enforcement tested on all routes. Admin can manage users. No default credentials. TLS works with a provided cert. |
| **2. Camera core + demo** | Camera model and registry, `CameraProvider` trait, ProviderFactory, full DemoProvider (no video yet), demo seeding, `--demo`, camera list and detail UI, WebSocket event bus with live status | App in demo mode shows ~24 cameras with live-updating status, capability-driven detail pages and working filters. Contract suite passes for DemoProvider. |
| **3. Streaming spike** | Throwaway prototype of candidates from §7.3. ADR 0002. | ADR merged with measurements. One upstream connection plays in 2+ tabs. |
| **4. AXIS provider + monitoring** | AxisProvider (identity, capabilities, status, stream profiles), Digest/Basic, TLS modes, CredentialStore with DPAPI, credential sets, test-connection, polling service, task manager, `docs/vapix.md`, ADR 0003 | Contract suite passes against wiremock fixtures. With a real camera (if available): register, see identity and capabilities, live status. Credentials encrypted at rest, never logged. |
| **5. Stream manager + live view** | Production stream manager per §7 (shared upstream connections, GOP buffer, grace period, reconnect, metrics), chosen egress, demo `MediaSource` with bundled clips, live view on camera details, admin Streams page | All §15.3 stream tests pass. Demo: 3 tabs on one camera → 1 upstream connection shown on the Streams page. Same with a real camera if available. |
| **6. Organization** | Groups (hierarchical), tags, favorites, bulk operations, filters across the UI | Filtering by parent group includes descendants. Bulk tag works. Favorites are per user. |
| **7. Dashboards** | Per-user dashboards, grid presets and custom grids, drag and drop, persistence, camera picker, expanded view, fullscreen, grid/detail profiles, visibility-based pausing, ADR 0005 | Rearranging tiles doesn't restart visible streams (verified by subscriber metrics). Layout persists across sessions. Playwright e2e covers the core flow in demo mode. |
| **8. Discovery** | CIDR jobs, caps, concurrency, progressive results, approve and register, deduplication, cancellation, demo discovery | Scanning a /24 never blocks the UI. Results stream live. Cancel works. Duplicates are flagged. |
| — | **MVP complete** | |
| **9. Hardening & packaging** | Windows service mode, installer, retention tuning, DuckDB ADR (0006), bandwidth limit/priority hooks, performance testing at target scale | Per §18 answers |

---

## 18. Open questions for the owner

Claude Code: use the **default** and keep going. Mention any that turn out to matter.

| # | Question | Default assumption |
|---|---|---|
| Q1 | Is a real AXIS camera available for development and testing? Which models and firmware? | No. Develop against fixtures, the demo provider and a local RTSP source. |
| Q2 | Does the app run as an interactive exe, a tray app or a Windows service? | Interactive console exe first. Service mode in Phase 9. DPAPI machine scope. |
| Q3 | Target scale: number of cameras and concurrent users? | Up to 250 cameras, up to 25 concurrent users, up to 50 concurrent upstream streams. |
| Q4 | Do viewers need per-group access restrictions? Can a camera belong to multiple groups? | No restrictions in the MVP (design for them). Yes, multiple groups. |
| Q5 | LAN-only, or access over the internet/VPN? | LAN/VPN only. No NAT traversal. |
| Q6 | Do any cameras use H.265 only? | Assume H.264 is available on all cameras. |
| Q7 | License and ownership (open source, client-owned, proprietary)? | Proprietary/undecided. Add no license file until decided. |
| Q8 | SSO / Active Directory needed? | No. Local accounts only. Keep auth pluggable. |
