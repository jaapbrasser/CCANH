# ADR 0013: Camera control (restart and pan/tilt/zoom)

- Status: accepted
- Date: 2026-11-22
- Phase: 12 (site feedback, issues #31 and #33)

## Context

Until 0.4.0 CCANH only read from cameras: status, identity, streams. The
site asked to restart (or try to turn on) cameras from the dashboard and the
camera page (#33), and to pan, tilt and zoom: one control moving every PTZ
camera on a dashboard at once, plus controls for a single camera, never
covering any part of the video (#31). Both change the physical device, so
who may do it and how it's recorded needed deciding.

## Decisions

- **Who:** every signed-in user (viewer and admin), through the UI and the
  API. The owner manages access in CCANH's roles; every command uses the one
  account each camera is linked with, which the owner treats as fully
  privileged (AXIS `reboot` needs an Administrator account; PTZ needs a
  viewer with PTZ access). A camera that refuses shows as `auth_failed`.
- **Restart:** `POST /cameras/{id}/restart` runs a `restart_camera` task
  (ADR 0008) and returns 202. AXIS: the firmware management API `reboot`
  (`restart.cgi` is obsolete, `docs/vapix.md`). The task succeeds once the
  camera accepts; the camera then drops offline and the status monitor
  reports it as usual. A second restart while one is queued or running gets
  409. Audited as `camera.restart` with the outcome. The UI asks for
  confirmation. Restart can't reach a camera that is unreachable or powered
  off; switching PoE power is a separate backlog item (#36, admin only).
- **PTZ protocol:** `POST /cameras/{id}/ptz` and `POST /dashboards/{id}/ptz`
  (own dashboards only; cameras without PTZ are skipped) take continuous
  speeds `{pan, tilt, zoom}` from -100 to 100, all zero to stop. These are
  synchronous (a few hundred milliseconds per camera, sent in parallel with a
  3 s timeout each), not tasks: they're interactive. The UI sends the move on
  press, repeats it every 400 ms while held, and sends a stop on release,
  pointer cancel, blur or window blur. Commands are sent one at a time so a
  stop never overtakes its move.
- **Safety:** AXIS continuous moves don't stop by themselves, so
  `ptz::PtzControl` keeps a watchdog per moving camera and sends a stop when
  a move isn't repeated within 1.5 s (closed tab, dropped connection). Each
  user gets 20 commands of burst and 10 per second sustained (a held control
  uses 2.5); beyond that 429.
- **Audit:** one `camera.ptz` entry per gesture: a new or changed move. Its
  repeats and the stop aren't recorded, so holding a button is one entry.
  Details: the action in words ("pan left, zoom in"), the speeds, the cameras
  (id and name) and how many failed. The target is the camera or the
  dashboard.
- **Which cameras can move:** `CameraSummary.ptz` is true when the stored
  capabilities include a supported item in the `ptz` group (AXIS: API
  Discovery ids containing ptz/pan/tilt/zoom; demo: the Q6135 model). The UI
  shows controls only then.
- **Placement:** the dashboard pad sits in a column beside the grid (also in
  fullscreen); the camera page and the expanded tile put it next to the
  video. Nothing is drawn over video.

## Consequences

- The provider contract (`providers/contract.rs`) covers `restart` and
  `ptz_move`; DemoProvider implements both (a restarted demo camera is
  unreachable for 20 s; PTZ is accepted by the PTZ model but the demo clips
  don't move).
- Real movement could only be checked on site; the AXIS calls are tested
  against wiremock with the documented request and response shapes.
- Concurrent users moving the same camera interleave last-command-wins, as
  AXIS does. The Queuing API (PTZ control ownership) isn't used.
