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

Queries are compile-time checked with `sqlx::query!` against `.sqlx/`
(see development.md).
