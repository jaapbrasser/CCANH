-- Phase 6: groups, tags and favorites (SPEC §9, ADR 0009).

-- Global, admin-managed hierarchy (adjacency list, unlimited depth). Cycles
-- are prevented by the application when moving a group.
CREATE TABLE groups (
    id         TEXT PRIMARY KEY NOT NULL,                    -- UUIDv7
    name       TEXT NOT NULL,
    parent_id  TEXT REFERENCES groups (id) ON DELETE RESTRICT, -- NULL = top level
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
-- Sibling names are unique, ignoring case (NULL parents compare equal here).
CREATE UNIQUE INDEX groups_sibling_name
    ON groups (COALESCE(parent_id, ''), name COLLATE NOCASE);
CREATE INDEX groups_parent ON groups (parent_id);

-- A camera can be in several groups (SPEC §18 Q4).
CREATE TABLE camera_groups (
    camera_id TEXT NOT NULL REFERENCES cameras (id) ON DELETE CASCADE,
    group_id  TEXT NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    PRIMARY KEY (camera_id, group_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX camera_groups_group ON camera_groups (group_id);

-- Global, admin-managed labels; names unique ignoring case.
CREATE TABLE tags (
    id         TEXT PRIMARY KEY NOT NULL,                    -- UUIDv7
    name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE camera_tags (
    camera_id TEXT NOT NULL REFERENCES cameras (id) ON DELETE CASCADE,
    tag_id    TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (camera_id, tag_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX camera_tags_tag ON camera_tags (tag_id);

-- Per user; never visible to other users.
CREATE TABLE favorites (
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    camera_id  TEXT NOT NULL REFERENCES cameras (id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, camera_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX favorites_camera ON favorites (camera_id);
