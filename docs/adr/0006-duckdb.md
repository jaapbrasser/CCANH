# ADR 0006: DuckDB for analytics — no-go for now

- Status: accepted
- Date: 2026-11-19
- Phase: 9

## Context

The original stack included DuckDB for analytical logging. SPEC §11.3 put that
off to Phase 9 and asked for a go/no-go based on whether it adds enough (for
example long-term fleet analytics) to justify a large native dependency.

## What we store today

- **Status history** (`camera_status_history`): one row per *transition*
  (online ↔ offline), not per check, with an index on `(camera_id,
  occurred_at)`. At the target scale (250 cameras), a normal day is a few
  hundred to a few thousand rows. The worst case is every camera flapping on
  every 10 s check: about 2.2 M rows/day, which SQLite handles. Retention
  (default 30 days, ADR 0008 settings, Phase 9 cleanup service) bounds it.
- **Stream metrics** are in memory only (per-second rates on the Streams page).
  Nothing is written per frame or per second.
- **Tasks, discovery results, audit log**: small, retention-bounded (the audit
  log is kept forever by default, at a few hundred rows a day).
- The Phase 9 load test (docs/performance.md) shows the API p95 at 34 ms with
  all of this running at target scale.

## Costs of DuckDB

- The `duckdb` crate bundles the C++ engine: much longer clean builds
  (minutes, especially on Windows CI), a binary several times larger (tens of
  MB), and a C++ toolchain on every build machine.
- A second storage engine to back up, migrate and keep consistent with SQLite.
- No current feature needs columnar scans. The UI's history views are
  per-camera timelines, which a SQLite index already serves.

## Decision

**No-go.** Keep SQLite as the only store. History access stays in the cameras
repository; if long-term analytics (e.g. fleet uptime over a year,
per-camera bandwidth trends) become a requirement, revisit by:

1. Adding the data first (e.g. hourly per-camera rollups in SQLite, still
   small).
2. Introducing a `MetricsStore` seam at that point, with DuckDB (or exporting
   Parquet for external tools) as one implementation if SQLite queries prove
   too slow on real data.

## Consequences

- Fast builds and one small `crane.exe` remain.
- Long retention of raw transitions is possible but bounded to 10 years by the
  settings; heavy analytics would need the steps above.
