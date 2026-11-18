-- Phase 5b: per-camera stream profile choices for grid tiles and the detail
-- view (SPEC §7.4). NULL means "use the global default, else pick by size".
ALTER TABLE cameras ADD COLUMN grid_profile TEXT;
ALTER TABLE cameras ADD COLUMN detail_profile TEXT;
