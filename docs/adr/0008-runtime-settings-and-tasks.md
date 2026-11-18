# ADR 0008: Runtime settings and background tasks

- Status: accepted
- Date: 2026-11-19
- Phase: 4b

## Context

SPEC §3.4 asks for some settings to be admin-editable at runtime (polling
interval, timeouts, discovery limits, stream grace period), stored in SQLite,
with the startup config as defaults. SPEC §10.3 asks for long-running
operations to be tracked tasks with progress, cancellation, a categorized
error, a REST API, WebSocket updates, persistence, and "interrupted" handling
after a restart. Both define persisted shapes and public API contracts.

## Decisions

### Runtime settings
- **Storage:** table `settings(key, value JSON, updated_at, updated_by)`, one row
  per settings group. Only `monitoring` exists so far; the stream grace period
  (Phase 5) and discovery limits (Phase 8) become their own groups.
- **Precedence:** startup config/env/CLI supply defaults, and a stored row
  overrides them. "Reset to defaults" deletes the row. An invalid stored value
  (e.g. after a bounds change in a new version) is logged and ignored.
- **Bounds:** every value has documented bounds, validated on write. The API
  answers 400 with the field name.
- **Propagation:** a `tokio::sync::watch` channel (`SettingsHandle`). Services
  read the latest value on their next tick, so no restart is needed. A shorter
  monitoring interval pulls already-scheduled checks forward, and a changed
  timeout rebuilds providers.
- **Audit:** every change is audited (`settings.update`) with the full before
  and after objects.
- **API:**
  - `GET /api/v1/settings`: current values, defaults and monitor counters.
  - `PATCH /api/v1/settings/monitoring`: partial update.
  - `DELETE /api/v1/settings/monitoring`: reset.

  All admin-only.

### Polling (SPEC §10.2 additions)
- **Retries within a check:** up to `retries` (default 1) for transient errors
  only (`unreachable`, `timeout`), 250 ms doubling ±jitter, capped at 2 s.
- **Offline backoff:** after `failure_threshold` failures the interval doubles
  per further failure, up to `offline_max_interval_secs` (default 60).
- **Auth failures:** a status call rejected for authentication waits the full
  offline interval at once, never retrying in a tight loop. (AXIS status checks
  are anonymous; this protects other providers and firmware that require auth.)
- **Metrics:** checks, failures, retries, identity checks, in-flight checks and
  cameras monitored, shown on the settings page.

### Tasks
- **Storage:** table `tasks`: kind, optional camera, creator (user id plus
  username snapshot), status (`queued | running | succeeded | failed |
  cancelled`), progress (current/total, `null` total = indeterminate), current
  message, error category and message, JSON result, and timestamps.
- **Concurrency:** at most 4 tasks run at once; the rest wait as `queued`.
  Cancelling a queued task never runs it. A panic inside a task becomes
  `failed` / `internal`.
- **Cancellation:** a per-task `CancellationToken`, and the task future is also
  aborted. Server shutdown cancels all tasks; `queued`/`running` rows left by a
  crash are marked `failed` (interrupted) at startup, before requests are
  served.
- **Visibility:** the creator and admins can see a task. Anyone else gets a 404
  for get and cancel, so task ids reveal nothing. The WebSocket `tasks` topic
  applies the same rule per event.
- **Retention:** finished tasks are purged after 30 days by the background
  sweeper.
- **Result vs failure:** a task `succeeded` when it ran to completion. Domain
  outcomes live in `result`: a connection test that found problems succeeds
  with `result.ok = false`. `failed` means the task itself couldn't complete.
- **Connection tests are tasks:** `POST /api/v1/cameras/test-connection`
  validates input synchronously (400s as before), then returns **202 with the
  task**. The result is the `TestConnectionResult` schema (also registered in
  the OpenAPI components).

## Consequences
- **Clients follow tasks** by subscribing to `tasks` and falling back to polling
  `GET /tasks/{id}` while a task is running (the UI does both). Toasts are
  derived from the creator's terminal events.
- **New long-running features** (discovery, bulk operations) only implement a
  job closure plus a `kind` label.
- **Bounds are enforced** both when an admin saves settings and when stored
  values are loaded.
