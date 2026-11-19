# ADR 0005: Dashboard grid and drag and drop

- Status: accepted
- Date: 2026-11-19
- Phase: 7

## Context

SPEC §8 asks for per-user dashboards: fixed grids (presets up to 4×4, custom
up to 8×8) whose tiles hold one camera, with drag to move, swap, replace and
remove, autosave, an expanded view, fullscreen and pausing of tiles that
aren't visible. The hard requirement is that **changing the layout must not
restart streams that are still visible**, verified by subscriber metrics.

## Options considered (drag and drop)

| Option | Fit | Notes |
|---|---|---|
| **@dnd-kit/core** | Good | Small, headless, MIT. Pointer, touch and keyboard sensors with screen-reader announcements. We render the grid ourselves. |
| react-grid-layout | Poor | Built for free-form resizable widgets, not a fixed cell grid with swap semantics. Heavier, and it owns layout and positioning. |
| Native HTML5 drag and drop | Fair | No dependency, but no touch support, awkward keyboard access and inconsistent drag images. |
| @dnd-kit/sortable | Unneeded | List reordering; a fixed grid only needs draggables and droppables. |

## Decisions

- **Library:** `@dnd-kit/core` 6.3 with a pointer sensor (6 px activation
  distance so clicks still expand tiles) and the keyboard sensor. Dropping on
  an empty cell moves the camera; dropping on an occupied cell swaps.
- **No remounts on rearrange:** camera tiles are direct children of one CSS
  grid, keyed by camera id and in a stable DOM order, and positioned only via
  `grid-row`/`grid-column`. Moving a tile changes its style, never its parent
  or key, so the `<video>` element and its MSE pipeline keep running. Streams
  are keyed by `(camera, profile)` in the frontend stream registry (ADR 0002),
  so even a remount would reuse the connection within its 3 s linger.
- **Profiles:** tiles request `purpose=grid`, and the expanded view
  `purpose=detail`. The server resolves both per camera (Phase 5b).
- **Pausing:** a tile pauses after 5 s off-screen (IntersectionObserver) or
  while the tab is hidden (Page Visibility API), and resumes immediately when
  visible again.
- **Storage:** `dashboards(user_id, name, rows, cols, tiles JSON, is_default)`.
  Tiles are `[{position, camera_id}]`, occupied cells only; each camera at
  most once per dashboard. One default per user (partial unique index); the
  first dashboard becomes the default, and deleting the default promotes the
  oldest remaining one. Tiles for deleted cameras drop out when read.
  Dashboards are private: another user's id is a 404, for admins too.
- **Autosave:** the layout changes locally at once and is saved 800 ms after
  the last change with a full `rows`/`cols`/`tiles` patch (last write wins).
  A failed save rolls back to the last saved layout and shows a toast.
- **Refused streams:** a media socket refused before any message (for example
  409 while a camera's profiles aren't known yet) is retried with backoff
  instead of showing "stopped".
- **End-to-end tests:** Playwright against the release binary in demo mode.
  The core-flow test checks the acceptance criterion through
  `GET /api/v1/streams`: same `started_at`, no reconnects, one viewer each
  after a drag.

## Consequences

- One small runtime dependency (`@dnd-kit/core` plus its tiny utilities).
- Simultaneous edits of one dashboard from two tabs: the last save wins.
  That's acceptable for private dashboards. A version check can be added if
  sharing ever arrives.
