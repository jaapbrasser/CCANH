-- Phase 4b: admin-editable runtime settings and tracked background tasks.

-- Overrides for runtime settings. The startup config supplies the defaults;
-- a row here wins (SPEC §3.4). `value` is JSON.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by TEXT                                  -- username, NULL for system
) STRICT;

-- Long-running operations (SPEC §10.3). Recurring polling is not a task.
CREATE TABLE tasks (
    id                  TEXT PRIMARY KEY NOT NULL,   -- UUIDv7
    kind                TEXT NOT NULL,               -- e.g. 'test_connection'
    camera_id           TEXT REFERENCES cameras (id) ON DELETE SET NULL,
    created_by          TEXT REFERENCES users (id) ON DELETE SET NULL,
    created_by_username TEXT,
    status              TEXT NOT NULL
                        CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    progress_current    INTEGER NOT NULL DEFAULT 0,
    progress_total      INTEGER,                     -- NULL = indeterminate
    message             TEXT,                        -- current step, human readable
    error_category      TEXT,
    error_message       TEXT,
    result              TEXT,                        -- JSON, kind-specific
    created_at          TEXT NOT NULL,
    started_at          TEXT,
    completed_at        TEXT
) STRICT;

CREATE INDEX tasks_created_at ON tasks (created_at);
CREATE INDEX tasks_created_by ON tasks (created_by);
CREATE INDEX tasks_status ON tasks (status);
