# API

The OpenAPI document is generated from code (`/api/openapi.json`, Swagger UI at
`/api/docs`, both admin-only unless `server.public_api_docs`; or offline via
`crane --print-openapi`). This page covers conventions the document can't.

- Base path `/api/v1/`, JSON, `snake_case`, RFC 3339 UTC timestamps, UUIDv7 IDs.
- Errors: RFC 9457 `application/problem+json` with `type = urn:crane:problem:<code>`
  and the same `code` as a field. Codes so far: `bad_request`, `unauthorized`,
  `invalid_credentials`, `forbidden`, `csrf_rejected`, `not_found`, `conflict`,
  `rate_limited` (with `Retry-After`), `unavailable`, `internal`.
- Pagination: `?page=&page_size=` (max 200); responses include `items`, `total`,
  `page`, `page_size`. Sorting: `?sort=&order=asc|desc` where supported.
- Auth: `POST /api/v1/auth/login` sets the `crane_session` cookie. Every operation
  states `Required role: none|viewer|admin`.
- CSRF: non-GET requests need an `Origin` header matching the server (ADR 0004).
