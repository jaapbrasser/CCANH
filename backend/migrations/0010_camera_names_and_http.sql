-- Phase 10 (site feedback).
-- #14: a camera added without a name shows its address until the first
-- successful identity check fills in the camera's own name.
ALTER TABLE cameras ADD COLUMN name_auto INTEGER NOT NULL DEFAULT 0 CHECK (name_auto IN (0, 1));

-- #17: certificate settings don't apply to HTTP cameras. tls_mode is NOT NULL,
-- so HTTP cameras keep the neutral 'strict' (the API reports null) and lose any pin.
UPDATE cameras SET tls_mode = 'strict', tls_pinned_sha256 = NULL WHERE scheme = 'http';
