-- Baseline schema. Real tables arrive with their phases (users in Phase 1, ...).
CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

INSERT INTO app_meta (key, value) VALUES ('schema', '1');
