-- Phase 4: encrypted camera credentials and credential sets (SPEC §4.5, §4.6).

-- `secret` is ciphertext from a CredentialStore (ADR 0003); never plaintext.
-- Shared sets have a name and can be reused by many cameras. Private sets hold
-- credentials entered for a single camera and have no name.
CREATE TABLE credential_sets (
    id         TEXT PRIMARY KEY NOT NULL,                 -- UUIDv7
    name       TEXT COLLATE NOCASE UNIQUE,
    shared     INTEGER NOT NULL CHECK (shared IN (0, 1)),
    secret     BLOB NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (shared = 0 OR name IS NOT NULL),
    CHECK (shared = 1 OR name IS NULL)
) STRICT;

ALTER TABLE cameras ADD COLUMN credential_set_id TEXT
    REFERENCES credential_sets (id) ON DELETE RESTRICT;
-- Basic auth over plain HTTP sends the password in the clear; off unless an
-- admin explicitly enables it for this camera.
ALTER TABLE cameras ADD COLUMN allow_basic_over_http INTEGER NOT NULL DEFAULT 0
    CHECK (allow_basic_over_http IN (0, 1));

CREATE INDEX cameras_credential_set ON cameras (credential_set_id);

-- Result of the last identity/credential check, separate from reachability:
-- a camera can be online but reject its credentials.
ALTER TABLE camera_status ADD COLUMN identity_error_category TEXT;
ALTER TABLE camera_status ADD COLUMN identity_error_message TEXT;
