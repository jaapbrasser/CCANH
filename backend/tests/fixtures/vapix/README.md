# VAPIX response fixtures

Used by the AxisProvider tests (`backend/src/providers/axis/tests.rs`) with
`wiremock`. **No physical camera was available (SPEC §18 Q1)**, so these are
built from the example responses in the official VAPIX documentation, not
recorded from a device. Each file notes its source below. Re-record against a
real AXIS camera when one is available and update `docs/vapix.md`.

| File | Source |
|---|---|
| `basicdeviceinfo_unrestricted.json` | Basic device information, `getAllUnrestrictedProperties`: documented property names; values illustrative |
| `apidiscovery_getapilist.json` | API Discovery service, `getApiList`: documented example response (verbatim structure and values) |
| `apidiscovery_getapilist_full.json` | Same structure, extended with `stream-profiles` and other APIs to exercise grouping |
| `systemready.json` | Systemready API, `systemready`: documented example response (verbatim) |
| `streamprofile_list.json` | Stream profiles, `list`: documented structure; profile names from the docs, parameters illustrative |
| `network_getnetworkinfo.json` | Network settings API, `getNetworkInfo`: documented example, trimmed to the fields we read |
| `error_unsupported_method.json` | Documented JSON error envelope, code 2005 (Unsupported Method) |
