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
| Probe: reachability + identity (brand, model `ProdNbr`, firmware `Version`, serial) | `POST /axis-cgi/basicdeviceinfo.cgi` `{"apiVersion":"1.0","method":"getAllUnrestrictedProperties"}` | Anonymous | [Basic device information](https://developer.axis.com/vapix/network-video/basic-device-information/) | API 1.x; AXIS OS 8.40+ | `basicdeviceinfo_unrestricted.json` |
| Status: readiness + uptime | `POST /axis-cgi/systemready.cgi` `{"apiVersion":"1.0","method":"systemready","params":{"timeout":1}}` | Anonymous | [Systemready API](https://developer.axis.com/vapix/network-video/systemready-api/) | AXIS OS 9.50+; older firmware falls back to the probe call | `systemready.json` |
| Capabilities: list of supported APIs and versions | `POST /axis-cgi/apidiscovery.cgi` `{"apiVersion":"1.0","method":"getApiList"}` | Anonymous (AXIS OS 9.80 and 10.12 require auth; the client retries with credentials on 401) | [API Discovery service](https://developer.axis.com/vapix/network-video/api-discovery-service/) | AXIS OS 8.50+; missing → only "relied-on APIs unsupported" reported | `apidiscovery_getapilist*.json` |
| Stream profiles; also verifies credentials | `POST /axis-cgi/streamprofile.cgi` `{"apiVersion":"1.0","method":"list","params":{"streamProfileName":[]}}` | Viewer | [Stream profiles](https://developer.axis.com/vapix/network-video/stream-profiles/) | API 1.0; `parameters` is `key=value&…` (`resolution`, `fps`, `videocodec` as used in [Video streaming](https://developer.axis.com/vapix/network-video/video-streaming/)) | `streamprofile_list.json`, `error_unsupported_method.json` |
| Network details: hostname, MAC, IPv4, gateway, DHCP | `POST /axis-cgi/network_settings.cgi` `{"apiVersion":"1.0","method":"getNetworkInfo"}` | **Administrator**; best-effort (skipped on 401/403) | [Network settings API](https://developer.axis.com/vapix/network-video/network-settings-api/) | AXIS OS 8.50+ | `network_getnetworkinfo.json` |

## Planned (not called yet)

| Purpose | Request | Docs | Phase |
|---|---|---|---|
| Live video (RTSP, H.264) | `rtsp://<host>/axis-media/media.amp?videocodec=h264[&streamprofile=<name>]` | [Video streaming](https://developer.axis.com/vapix/network-video/video-streaming/) | 5 |
| MJPEG fallback | to be verified under [Media stream over HTTP](https://developer.axis.com/vapix/network-video/media-stream-over-http/) | — | 5 |
| Discovery identification | the anonymous Basic device information call above | — | 8 |
