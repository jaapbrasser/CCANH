# ADR 0007: WebSocket protocol for live updates

- Status: accepted
- Date: 2026-11-18
- Phase: 2

(Numbered 0007 because SPEC §16 reserves 0001–0006 for other decisions.)

## Context

SPEC §13.2 asks for one multiplexed WebSocket per client at `/api/v1/ws`,
authenticated with the session cookie, origin-checked, carrying typed JSON
messages, with topic subscriptions, heartbeats, client reconnect with backoff,
and server-side filtering to events the user may see. The message shapes are a
public contract, so they are recorded here.

## Decision

### Connection
- `GET /api/v1/ws` upgrade. Requires a valid session (401 otherwise) and an
  `Origin` matching the server or `server.allowed_origins` (403 otherwise; same
  rule as the CSRF guard, ADR 0004). Max message size 64 KiB.
- The server sends a WebSocket ping every 20 s; if nothing has been heard from
  the client for 60 s it closes with **4408**.
- The session is re-validated every `auth.websocket_recheck_secs` (default 60).
  If it ended, expired, the user was disabled, or their role changed, the server
  closes with **4401**; the client re-checks `/auth/me` and signs in again.
- Media never flows over this socket (separate endpoints, ADR 0002).

### Messages
JSON objects with a `type` discriminator. The schemas are in the OpenAPI
document as `WsClientMessage` and `WsServerMessage`, so the frontend gets
generated types.

Client → server:

| type | fields |
|---|---|
| `subscribe` | `topics: Topic[]` |
| `unsubscribe` | `topics: Topic[]` |

Server → client:

| type | fields | when |
|---|---|---|
| `hello` | `username`, `role`, `heartbeat_secs` | right after connecting |
| `subscribed` | `topics` (full active set) | after each (un)subscribe |
| `camera_status` | `camera_id`, `status` | after each status check (topic `camera_status`) |
| `camera_changed` | `camera` (summary) | camera added/edited/identity refreshed (topic `cameras`) |
| `camera_deleted` | `camera_id` | camera removed (topic `cameras`) |
| `task_updated` | `task` | task created, progressed or finished (topic `tasks`; own tasks only, admins see all) |
| `streams_updated` | `streams` (list as in `GET /api/v1/streams`) | every second while any stream exists, once more after the last ends (topic `streams`, admins only) |
| `resync` | `reason` | client fell behind the event stream; refetch over REST |
| `error` | `code`, `message` | malformed message or unknown topic |

Topics so far: `camera_status`, `cameras`, `tasks` (added in Phase 4b, see
ADR 0008), `streams` (Phase 5; admin only, a viewer's subscription is accepted
but receives nothing). Later phases add e.g. `discovery:{job_id}`. Live video
itself uses separate media sockets (ADR 0002). Unknown topics are rejected with `error`.

### Delivery semantics
- At-most-once, best effort. The server holds a bounded broadcast buffer (1024
  events); a client that falls further behind gets `resync`.
- After any reconnect the client refetches its REST queries, since events may
  have been missed while disconnected. Reconnect uses exponential backoff from
  1 s to 30 s with ±20% jitter.
- Every event passes through one authorization hook (`may_see`) before being
  sent. In the MVP all signed-in users see all cameras; per-group access
  (SPEC §18 Q4) plugs in there.

## Consequences
- REST stays the source of truth; the socket only keeps caches fresh. A client
  that never connects still works, just without live updates.
- `camera_status` is sent on every check, not only on state changes, so the UI
  can show live response times and "last contact". At the target scale (250
  cameras, 10 s interval) that is ~25 small messages/s per client on the LAN,
  and no extra camera traffic.
