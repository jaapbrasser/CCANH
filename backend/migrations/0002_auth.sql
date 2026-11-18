-- Phase 1: users, sessions and the audit log.

CREATE TABLE users (
    id                  TEXT PRIMARY KEY NOT NULL,          -- UUIDv7
    username            TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash       TEXT NOT NULL,                      -- Argon2id PHC string
    role                TEXT NOT NULL CHECK (role IN ('admin', 'viewer')),
    disabled            INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
    created_at          TEXT NOT NULL,                      -- RFC 3339 UTC
    updated_at          TEXT NOT NULL,
    password_changed_at TEXT NOT NULL
) STRICT;

-- Only the SHA-256 of the session token is stored, so a copy of the database
-- can't be used to hijack sessions. Times are unix seconds for cheap comparisons.
CREATE TABLE sessions (
    token_hash   TEXT PRIMARY KEY NOT NULL,
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,                          -- absolute expiry
    source_ip    TEXT,
    user_agent   TEXT
) STRICT;

CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX sessions_expires_at ON sessions (expires_at);

CREATE TABLE audit_log (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at    TEXT NOT NULL,                           -- RFC 3339 UTC
    actor_id       TEXT,                                    -- NULL for anonymous
    actor_username TEXT,
    action         TEXT NOT NULL,
    outcome        TEXT NOT NULL CHECK (outcome IN ('success', 'failure')),
    target_type    TEXT,
    target_id      TEXT,
    source_ip      TEXT,
    details        TEXT                                     -- JSON object, never secrets
) STRICT;

CREATE INDEX audit_log_occurred_at ON audit_log (occurred_at);
CREATE INDEX audit_log_action ON audit_log (action);

-- Append-only: rows can't be modified. (Retention cleanup, when added, will
-- delete old rows; updates are never legitimate.)
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
