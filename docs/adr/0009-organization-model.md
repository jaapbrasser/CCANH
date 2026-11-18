# ADR 0009: Groups, tags and favorites

- Status: accepted
- Date: 2026-11-19
- Phase: 6

## Context

SPEC §9 asks for hierarchical groups of unlimited depth (adjacency list, no
cycles, filters that include descendants, deletion that asks what happens to
children and cameras), global tags with case-insensitive unique names, and
per-user favorites. SPEC §6.1 adds list filters, membership columns and bulk
actions. SPEC §18 Q4 assumes a camera can be in several groups and that
per-group access control may come later. These choices fix persisted shapes
and public API contracts.

## Decisions

- **Tables:** `groups(id, name, parent_id → groups ON DELETE RESTRICT)`,
  `camera_groups`, `tags(name COLLATE NOCASE UNIQUE)`, `camera_tags`,
  `favorites(user_id, camera_id)`. Join rows cascade when a camera, group,
  tag or user is deleted. Sibling group names are unique ignoring case
  (unique index on `(COALESCE(parent_id, ''), name COLLATE NOCASE)`); the same
  name may appear at different levels.
- **Cycles:** checked in the application on move (`parent_id` may not be the
  group itself or any descendant, found with a recursive CTE). SQLite has no
  cheap declarative check for this.
- **Descendant filter:** `GET /cameras?group_id=` matches cameras in the group
  or any descendant, using a recursive CTE inside the list query, so it stays
  one indexed query and works with pagination and other filters.
- **Delete modes** (`DELETE /groups/{id}?mode=`, required):
  - `move_up`: subgroups get the deleted group's parent; its cameras join the
    parent (for a top-level group, subgroups become top-level and cameras
    simply leave it).
  - `detach`: subgroups become top-level; cameras leave the group.
  Cameras are never deleted. A name clash at the destination is a 409 and
  nothing changes (one transaction).
- **Memberships in responses:** camera summaries carry `groups` and `tags`
  (id + name, direct memberships only); clients build group paths from
  `GET /groups`. Summaries are broadcast on the `cameras` topic, so they
  contain nothing per-user.
- **Favorites** are fetched separately (`GET /favorites` → camera ids) and
  filtered server-side with `favorites=true`, which always means the caller's
  own favorites. They are not audited (not security-relevant).
- **Bulk operations** (`POST /cameras/bulk`): up to 500 cameras, one action
  (add/remove tags or groups, monitoring on/off, delete). Validation happens
  before the 202, so unknown ids fail fast; the work runs as a `bulk_cameras`
  task (ADR 0008) with progress and cancellation between cameras. Tag and
  group changes are single statements; monitoring and delete go camera by
  camera through the same code as single edits (delete stops live streams and
  audits each camera). Non-delete actions write one `camera.bulk_update`
  audit entry.
- **Live updates:** membership changes arrive as `camera_changed`; group or
  tag list changes as `organization_changed {kind}` on the `cameras` topic
  (ADR 0007).
- **Access control later:** groups are the natural unit for per-group access
  (SPEC §18 Q4). The list query and `ws::may_see` are the two places a group
  filter would plug in.

## Consequences

- Group trees are small (dozens to hundreds of nodes), so recursive CTEs and
  sending the whole flat list to clients are cheap.
- Deleting a group is never silent about its contents, and is reversible only
  by recreating the group.
