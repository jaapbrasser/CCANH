# API

The OpenAPI document is generated from code (`/api/openapi.json`, Swagger UI at
`/api/docs`, both admin-only unless `server.public_api_docs`; or offline via
`crane --print-openapi`). This page covers conventions the document can't.

- Base path `/api/v1/`, JSON, `snake_case`, RFC 3339 UTC timestamps, UUIDv7 IDs.
- Errors: RFC 9457 `application/problem+json` with `type = urn:crane:problem:<code>`
  and the same `code` as a field. Codes so far: `bad_request`, `unauthorized`,
  `invalid_credentials`, `forbidden`, `csrf_rejected`, `not_found`, `conflict`,
  `rate_limited` (with `Retry-After`), `camera_error` (502, with
  `error_category`), `unavailable`, `internal`.
- Pagination: `?page=&page_size=` (max 200); responses include `items`, `total`,
  `page`, `page_size`. Sorting: `?sort=&order=asc|desc` where supported.
- Auth: `POST /api/v1/auth/login` sets the `crane_session` cookie. Every operation
  states `Required role: none|viewer|admin`.
- CSRF: non-GET requests need an `Origin` header matching the server (ADR 0004).
- Live updates: one WebSocket at `/api/v1/ws` (ADR 0007).
- Organization (ADR 0009): `GET /cameras` filters by `group_id` (includes
  subgroups), `tag_id` and `favorites=true` (the caller's own). Bulk changes go
  through `POST /cameras/bulk` and return 202 with a `bulk_cameras` task; the
  selection is `camera_ids` and/or `group_id` (+ `include_subgroups`) and/or
  `tag_id`, and `update_settings` changes only the given connection and
  security settings (skipped cameras are listed in the result).
- Camera defaults: `POST /cameras` (and discovery registration) take omitted
  `scheme`, `tls_mode` and `allow_basic_over_http` from the `camera_defaults`
  setting (`PATCH|DELETE /settings/camera-defaults`, ADR 0011). `name` is
  optional; `name_pending` says the address stands in until the camera's own
  name arrives. HTTP cameras report `tls_mode: null`.
- Discovery (`/discovery/*`, admin): `POST /discovery/preview` counts addresses
  and checks the caps; `POST /discovery/jobs` returns 202 with the job (its id
  is the task id, so cancel with `POST /tasks/{id}/cancel`); results stream on
  the `discovery` topic; `POST /discovery/jobs/{id}/test-credentials` checks
  credentials against up to 20 results (a `test_credentials` task, nothing
  stored); `POST /discovery/jobs/{id}/register` registers approved results.
  Nothing is registered automatically.
- Dashboards (`/dashboards`, ADR 0005) belong to the caller; another user's
  dashboard id answers 404, for admins too.
- Stream limits (settings `streaming.max_upstreams`, `detail_reserve`,
  `max_inbound_mbps`; off by default): a refused stream's media socket opens,
  sends `{"type":"state","state":"failed","message":…}` with the reason, and
  closes.
- Live video: a media WebSocket per stream at
  `/api/v1/streams/{camera_id}/ws?profile=|purpose=`, carrying fMP4 for MSE
  (ADR 0002). It isn't in the OpenAPI document beyond the upgrade route.
- Long-running operations return **202 with a task**, which you follow via
  `GET /api/v1/tasks/{id}` or the `tasks` topic. The task's `result` holds the
  outcome (ADR 0008). `POST /api/v1/cameras/test-connection` works this way.
