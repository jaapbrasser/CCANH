-- Phase 2: camera registry, current status and status-transition history.

CREATE TABLE cameras (
    id                  TEXT PRIMARY KEY NOT NULL,           -- UUIDv7
    name                TEXT NOT NULL,
    host                TEXT NOT NULL COLLATE NOCASE,        -- IP or hostname, no scheme/port
    port                INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    scheme              TEXT NOT NULL CHECK (scheme IN ('http', 'https')),
    tls_mode            TEXT NOT NULL DEFAULT 'strict'
                        CHECK (tls_mode IN ('strict', 'pinned', 'insecure_skip_verify')),
    tls_pinned_sha256   TEXT,                                -- required when tls_mode = 'pinned'
    monitoring_enabled  INTEGER NOT NULL DEFAULT 1 CHECK (monitoring_enabled IN (0, 1)),

    -- Identity, filled in from the provider (NULL until first successful probe).
    manufacturer        TEXT,
    model               TEXT,
    firmware_version    TEXT,
    serial_number       TEXT,
    mac_address         TEXT,
    device_name         TEXT,
    device_hostname     TEXT,
    capabilities        TEXT,                                -- JSON array of capabilities
    stream_profiles     TEXT,                                -- JSON array of stream profiles
    network             TEXT,                                -- JSON object, network info
    identity_updated_at TEXT,

    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,

    UNIQUE (host, port),
    CHECK (tls_mode != 'pinned' OR tls_pinned_sha256 IS NOT NULL)
) STRICT;

CREATE INDEX cameras_name ON cameras (name COLLATE NOCASE);

-- Current status: one row per camera, updated on every check.
CREATE TABLE camera_status (
    camera_id            TEXT PRIMARY KEY NOT NULL REFERENCES cameras (id) ON DELETE CASCADE,
    state                TEXT NOT NULL DEFAULT 'unknown'
                         CHECK (state IN ('unknown', 'online', 'offline')),
    last_success_at      TEXT,
    last_failure_at      TEXT,
    last_change_at       TEXT,
    last_checked_at      TEXT,
    response_time_ms     INTEGER,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    error_category       TEXT,                               -- see providers::ErrorCategory
    error_message        TEXT                                -- short, never contains secrets
) STRICT;

-- Written only when the state changes (SPEC §10.2), not on every poll.
CREATE TABLE camera_status_history (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id      TEXT NOT NULL REFERENCES cameras (id) ON DELETE CASCADE,
    occurred_at    TEXT NOT NULL,
    from_state     TEXT NOT NULL,
    to_state       TEXT NOT NULL,
    error_category TEXT
) STRICT;

CREATE INDEX camera_status_history_camera ON camera_status_history (camera_id, id);
