# Database

SQLite in WAL mode with foreign keys on, at `<data_dir>/crane.db` (`demo.db` in
demo mode). Migrations live in `backend/migrations/`, run automatically at
startup, and an existing database is snapshotted (`VACUUM INTO`) before pending
migrations are applied. All tables are `STRICT`.

| Table | Purpose | Notes |
|---|---|---|
| `app_meta` | Key/value metadata | `schema` version marker |
| `users` | Local accounts | `username` unique `COLLATE NOCASE`; Argon2id PHC hash; `role` ∈ admin/viewer; RFC 3339 UTC timestamps; UUIDv7 IDs as text |
| `sessions` | Server-side sessions | Primary key is SHA-256 of the cookie token; times are unix seconds; cascades on user delete |
| `audit_log` | Security audit trail | Append-only (an `UPDATE` trigger aborts); `details` is a JSON object with no secrets |
| `cameras` | Camera registry | Unique `(host, port)`; TLS mode + optional pinned fingerprint; identity columns and JSON `capabilities`/`stream_profiles`/`network` filled from the provider; `grid_profile`/`detail_profile` choices (NULL = global default) |
| `camera_status` | Current status, one row per camera | Updated on every check; cascades on camera delete |
| `camera_status_history` | Status transitions | Written only when the state changes (SPEC §10.2) |
| `credential_sets` | Encrypted camera credentials | Ciphertext only (ADR 0003); shared (named) or private (per camera) |
| `groups` | Group hierarchy (ADR 0009) | Adjacency list (`parent_id`, RESTRICT); sibling names unique ignoring case |
| `camera_groups` | Camera ↔ group (many-to-many) | Cascades with camera or group |
| `tags` | Global tags | `name` unique, `COLLATE NOCASE` |
| `camera_tags` | Camera ↔ tag | Cascades with camera or tag |
| `favorites` | Per-user favorite cameras | Cascades with user or camera; never shown to other users |
| `settings` | Runtime setting overrides | One JSON row per group (`monitoring`, `streaming`); defaults come from config (ADR 0008) |
| `tasks` | Background tasks | Status, progress, categorized error, JSON result; interrupted tasks marked failed at startup; purged after 30 days |

Queries are compile-time checked with `sqlx::query!` against `.sqlx/`
(see development.md).
