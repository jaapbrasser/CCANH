# ADR 0004: Sessions, CSRF and authentication hardening

- Status: accepted
- Date: 2026-11-18
- Phase: 1

## Context

SPEC §14 requires local accounts with Argon2id, server-side sessions in SQLite,
`HttpOnly`/`SameSite=Strict`/`Secure` cookies, idle (8 h) and absolute (7 d)
timeouts, CSRF protection, per-username and per-IP login rate limiting, a
first-run bootstrap without default credentials, and role checks in one place.

## Decisions

### Passwords
- Argon2id v19 with the `argon2` crate defaults (m = 19 MiB, t = 2, p = 1, the
  OWASP minimum). Hashing runs on Tokio's blocking pool.
- Policy: 12–1024 characters, no composition rules (NIST SP 800-63B).
- Unknown usernames are verified against a dummy hash so failed logins take the
  same time and return the same body whether or not the user exists.
- Secrets are `secrecy::SecretString`: redacted `Debug`, no `Serialize`, zeroed on
  drop. A test captures TRACE-level logs for a full set of credential operations
  and asserts no password, hash, setup token or cookie appears.

### Sessions
- 256-bit random token (OS RNG), base64url, in cookie `crane_session`
  (`HttpOnly`, `SameSite=Strict`, `Path=/`, `Max-Age` = absolute timeout, `Secure`
  when TLS is on).
- The database stores only `SHA-256(token)`, so a database copy can't be replayed.
- Each request validates idle and absolute expiry and the user's `disabled` flag.
  `last_seen_at` is written at most once a minute. A background task purges
  expired rows every 10 minutes.
- Sessions end on logout, password change (other sessions only), admin password
  reset, disable and delete (cascade).
- Roles are read from the `users` row on every request, so role changes apply
  immediately to existing sessions.

### CSRF
Chosen: **SameSite=Strict plus an Origin/Referer check**, not a double-submit token.

- Options considered: (a) synchronizer/double-submit token, (b) SameSite + Origin
  check, (c) custom header requirement.
- (b) needs no client-side token plumbing, works for every same-origin `fetch`,
  and is what OWASP lists as a sufficient defence in depth alongside SameSite.
- Rule: any non-safe method under `/api/` must carry an `Origin` (or, failing
  that, `Referer`) equal to the server's own origin (`http(s)://<Host header>`,
  scheme from the TLS setting, default ports normalized) or an entry in
  `server.allowed_origins`. Missing or `null` origins are rejected with
  problem `csrf_rejected` (403). The login and setup endpoints are covered too
  (login CSRF).
- Consequence: non-browser API clients must send an `Origin` header.

### Login rate limiting
- In memory, keyed by lowercase username and by client IP. After 5 failures per
  username (20 per IP) further attempts are refused with 429 + `Retry-After` for
  1 s, doubling per failure, capped at 15 min; history is forgotten after an hour
  without failures. Success clears the username key only.
- Lockout applies even when the next password is correct, so guessing can't
  continue at full speed. The cap bounds how long an attacker can lock out a
  known username.
- Blocked attempts are logged but not audited, so a flood can't fill the audit
  table. Failed attempts are audited with a reason (`unknown_user`,
  `bad_password`, `disabled`), visible to admins only.
- Client IP is the TCP peer address; no proxy headers are trusted (LAN only).

### Bootstrap
- If the database has no users at startup, a one-time setup token is generated,
  printed to stderr and the log, and its hash kept in memory. `POST /api/v1/setup`
  with the token creates the first admin, atomically only if no users exist,
  and signs them in. Token guesses are rate limited.
- `crane --create-admin <username>` prompts for a password twice and creates an
  admin. It works even when users exist: console access already implies control
  of the host, and it is the recovery path for lost admin passwords.

### Authorization
- `AuthUser` (any signed-in user) and `AdminUser` (403 for viewers) extractors are
  the only place roles are checked. Handlers take one as their first argument.
- Each OpenAPI operation states `Required role: none|viewer|admin`. A test walks
  the generated route table and asserts anonymous → 401 for viewer/admin routes and
  viewer → 403 for admin routes, so new routes are covered automatically.
- The last enabled admin can't be deleted, disabled or demoted. Enforced inside a
  single SQL statement per change, so concurrent requests can't race past it.
  Admins can't delete their own account.
- OpenAPI JSON and Swagger UI are admin-only unless `server.public_api_docs`.

### Server TLS
- rustls with the `ring` provider (no CMake/NASM on Windows, unlike aws-lc-rs),
  served through `axum-server`. Modes: `off`, `files` (PEM chain + key),
  `self_signed` (rcgen, stored in `<data_dir>/tls/`, key file `0600` on Unix;
  SHA-256 fingerprint logged at startup). ALPN offers h2 and http/1.1.

## Consequences

- No client-side CSRF token handling; the frontend just uses same-origin `fetch`.
- Rate-limit state resets on restart (acceptable on a LAN service).
- Per-group camera permissions (SPEC §18 Q4) can be added as another extractor or
  a check that takes `AuthUser` plus a resource, without touching handlers'
  authentication.
