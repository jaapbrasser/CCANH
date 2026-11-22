-- Phase 12: tags are removed (issue #35). Groups and favorites stay.
-- The pre-migration backup (storage::backup) keeps the old data.
DROP INDEX IF EXISTS camera_tags_tag;
DROP TABLE IF EXISTS camera_tags;
DROP TABLE IF EXISTS tags;
