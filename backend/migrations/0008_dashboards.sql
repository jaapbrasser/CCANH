-- Phase 7: per-user dashboards (SPEC §8, ADR 0005). Private to their owner.
CREATE TABLE dashboards (
    id         TEXT PRIMARY KEY NOT NULL,                 -- UUIDv7
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    rows       INTEGER NOT NULL CHECK (rows BETWEEN 1 AND 8),
    cols       INTEGER NOT NULL CHECK (cols BETWEEN 1 AND 8),
    -- JSON array of {"position": n, "camera_id": "<uuid>"}; empty tiles omitted.
    tiles      TEXT NOT NULL DEFAULT '[]',
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX dashboards_user ON dashboards (user_id);
-- At most one default dashboard per user.
CREATE UNIQUE INDEX dashboards_one_default ON dashboards (user_id) WHERE is_default = 1;
