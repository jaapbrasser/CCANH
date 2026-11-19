-- Phase 8: camera discovery (SPEC §5). A job is a `discovery` task; results
-- are never registered automatically.
CREATE TABLE discovery_jobs (
    id            TEXT PRIMARY KEY NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    ranges        TEXT NOT NULL,                -- JSON array of the ranges as entered
    address_count INTEGER NOT NULL,
    scanned       INTEGER NOT NULL DEFAULT 0,   -- addresses checked so far
    responsive    INTEGER NOT NULL DEFAULT 0,   -- addresses with an open HTTP(S) port
    found         INTEGER NOT NULL DEFAULT 0,   -- AXIS devices identified
    created_at    TEXT NOT NULL
) STRICT;

CREATE TABLE discovery_results (
    id                     TEXT PRIMARY KEY NOT NULL,   -- UUIDv7
    job_id                 TEXT NOT NULL REFERENCES discovery_jobs (id) ON DELETE CASCADE,
    ip                     TEXT NOT NULL,
    port                   INTEGER NOT NULL,
    scheme                 TEXT NOT NULL CHECK (scheme IN ('http', 'https')),
    manufacturer           TEXT,
    model                  TEXT,
    firmware_version       TEXT,
    serial_number          TEXT,
    mac_address            TEXT,
    hostname               TEXT,
    device_name            TEXT,
    tls_fingerprint_sha256 TEXT,                -- seen during the scan; pinned on register
    response_time_ms       INTEGER,
    registered_camera_id   TEXT REFERENCES cameras (id) ON DELETE SET NULL,
    found_at               TEXT NOT NULL,
    UNIQUE (job_id, ip)
) STRICT;
CREATE INDEX discovery_results_job ON discovery_results (job_id);
