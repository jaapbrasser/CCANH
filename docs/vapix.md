# VAPIX endpoints used

Every AXIS endpoint the application calls, checked against the official VAPIX
documentation (SPEC §4.4). **Never add a call that isn't listed here with its
documentation link.**

All JSON endpoints are `POST` with `Content-Type: application/json` and return
either `{"data": …}` or a JSON error envelope `{"error": {"code", "message"}}`
with HTTP 200. Codes 2000 (unsupported version) and 2005 (unsupported method)
are treated as `Unsupported`; others as `protocol_error`. HTTP 404 means the API
doesn't exist on this firmware (`Unsupported`), 401 triggers authentication,
and 403 means the account lacks privileges.

Authentication follows "Authentication"
(<https://developer.axis.com/vapix/authentication/>): the first request goes
without credentials. On `401` the scheme comes from `WWW-Authenticate`.
Digest (MD5/SHA-256, `qop=auth`) is preferred; Basic is used over HTTPS only,
unless an admin enables it per camera.

Verified 2026-11-19. **No physical camera was available** (SPEC §18 Q1). Test
fixtures are built from the documented examples (see
`backend/tests/fixtures/vapix/README.md`) and should be re-recorded against a
real device.

| Purpose | Request | Access | Docs | Version / firmware | Fixture |
|---|---|---|---|---|---|
| Probe: reachability + identity (brand, model `ProdNbr`, firmware `Version`, serial); also **discovery identification** (Phase 8) | `POST /axis-cgi/basicdeviceinfo.cgi` `{"apiVersion":"1.0","method":"getAllUnrestrictedProperties"}` | Anonymous | [Basic device information](https://developer.axis.com/vapix/network-video/basic-device-information/) | API 1.x; AXIS OS 8.40+ | `basicdeviceinfo_unrestricted.json` |
| Status: readiness + uptime | `POST /axis-cgi/systemready.cgi` `{"apiVersion":"1.0","method":"systemready","params":{"timeout":1}}` | Anonymous | [Systemready API](https://developer.axis.com/vapix/network-video/systemready-api/) | AXIS OS 9.50+; older firmware falls back to the probe call | `systemready.json` |
| Capabilities: list of supported APIs and versions | `POST /axis-cgi/apidiscovery.cgi` `{"apiVersion":"1.0","method":"getApiList"}` | Anonymous (AXIS OS 9.80 and 10.12 require auth; the client retries with credentials on 401) | [API Discovery service](https://developer.axis.com/vapix/network-video/api-discovery-service/) | AXIS OS 8.50+; missing → only "relied-on APIs unsupported" reported | `apidiscovery_getapilist*.json` |
| Stream profiles; also verifies credentials | `POST /axis-cgi/streamprofile.cgi` `{"apiVersion":"1.0","method":"list","params":{"streamProfileName":[]}}` | Viewer | [Stream profiles](https://developer.axis.com/vapix/network-video/stream-profiles/) | API 1.0; `parameters` is `key=value&…` (`resolution`, `fps`, `videocodec` as used in [Video streaming](https://developer.axis.com/vapix/network-video/video-streaming/)) | `streamprofile_list.json`, `error_unsupported_method.json` |
| Network details: hostname, MAC, IPv4, gateway, DHCP | `POST /axis-cgi/network_settings.cgi` `{"apiVersion":"1.0","method":"getNetworkInfo"}` | **Administrator**; best-effort (skipped on 401/403) | [Network settings API](https://developer.axis.com/vapix/network-video/network-settings-api/) | AXIS OS 8.50+ | `network_getnetworkinfo.json` |
| Live video (Phase 5b) | RTSP `DESCRIBE`/`SETUP`/`PLAY` on `rtsp://<host>:554/axis-media/media.amp?videocodec=h264&audio=0[&streamprofile=<name>]`; RTP over the RTSP TCP connection (`RTP/AVP/TCP;unicast`) | Viewer (RTSP Digest, or whatever the camera challenges with) | [Video streaming](https://developer.axis.com/vapix/network-video/video-streaming/) (RTSP URL, `videocodec`, `streamprofile`, `audio`, port 554, TCP transport) | RTSP API (`Properties.API.RTSP.RTSPAuth=yes` for auth) | `rtsp_describe_h264.sdp` (fake RTSP server in `providers/axis/rtsp_tests.rs`) |

Live video notes:

- The empty profile id (the camera default stream) omits `streamprofile`.
  Values are URL-encoded; profile names come from `streamprofile.cgi list`.
- Credentials go only in RTSP `Authorization` headers (retina answers the
  camera's challenge), never in the URL, so URLs are safe to log. RTSP is not
  encrypted; the deployment is LAN-only (owner decision, Phase 5).
  `allow_basic_over_http` doesn't apply: retina uses Digest when the camera
  offers it, which AXIS does by default.
- RTSP status mapping when opening: 401/403 → `auth_failed` (not retried);
  400/404/406/415/451/457/461 → `unsupported` (unknown profile or parameters;
  not retried); 453/503 → `rate_limited` (camera busy; retried); other
  statuses → `protocol_error`; connection failures → `unreachable`. Once
  playing, any interruption is retried with backoff.
- The RTSP port is fixed at the documented default, 554.

Discovery notes (Phase 8, SPEC §5.1):

- Before any HTTP request, discovery makes a TCP connect check on 443 and 80
  (at the same time, so a silent address costs one timeout). A responsive host
  then gets **exactly one** request: the anonymous `getAllUnrestrictedProperties`
  call above, which the docs list as needing no credentials. HTTPS is preferred.
- The certificate can't be verified during a scan (no pin exists yet), so the
  scan reads only unrestricted data and records the fingerprint it saw.
  Registering a result pins that fingerprint, once an admin has approved it.
- An error or non-AXIS answer counts as "responding, unidentified". No
  further requests are made to that host.

## Planned (not called yet)

| Purpose | Request | Docs | Phase |
|---|---|---|---|
| MJPEG fallback | to be verified under [Media stream over HTTP](https://developer.axis.com/vapix/network-video/media-stream-over-http/) | — | deferred (ADR 0002) |
