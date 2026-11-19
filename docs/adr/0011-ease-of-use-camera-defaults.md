# ADR 0011: Ease-of-use camera defaults

- Status: accepted
- Date: 2026-11-19
- Phase: 10 (site feedback, issues #14, #17, #18)

## Context

SPEC §4.5 made Basic authentication over plain HTTP an explicit per-camera
admin opt-in and defaulted new cameras to HTTPS with strict certificate
checks. On the first site deployment most cameras only offer Basic over HTTP,
and admins had to tick the option for every camera, both when adding cameras
and (impossibly) when registering them from discovery. The owner's decision:
**ease of use over security** on this closed local network, with stricter
behaviour available as a configurable choice.

## Decisions

- New runtime settings group `camera_defaults` (⚙ Settings → General → New camera
  defaults; config section `[camera_defaults]`), audited like the others:
  - `scheme`: **`http`** (owner decision, #18).
  - `allow_basic_over_http`: **`true`**.
  - `tls_mode` for cameras added with HTTPS: **`insecure_skip_verify`**.
    `pinned` can't be a default (a pin is per camera). Discovery keeps pinning
    the certificate it saw during the scan, which is stricter and needs no
    clicks.
- The defaults pre-fill the add-camera form and discovery registration, and
  apply to `POST /cameras` and `POST /discovery/jobs/{id}/register` fields the
  request leaves out. Explicit values always win. Changing a default never
  touches existing cameras (#19's bulk edit does that).
- Certificate settings don't apply to HTTP cameras (#17): the API reports
  `tls_mode: null` for them, saving an HTTP camera clears any pin, and the UI
  shows "Not applicable (HTTP)". The column stays `NOT NULL` (storing the
  neutral `strict`) to avoid rebuilding the `cameras` table in SQLite.
- SPEC §4.5's rule becomes: Basic over HTTP is allowed per camera; whether new
  cameras get it is the `camera_defaults.allow_basic_over_http` setting,
  shipped on. Digest is still preferred whenever a camera offers it, and
  Basic is still only used when the camera's challenge offers nothing else.

## Consequences

- With the shipped defaults, camera passwords of Basic-only cameras cross the
  network readable, and HTTPS cameras added by hand aren't verified. Sites
  that need more set `scheme = "https"`, `allow_basic_over_http = false` and
  `tls_mode = "strict"` once, in Settings or `config.toml`.
- The defaults are visible and editable in one place, and every change is in
  the audit log.
