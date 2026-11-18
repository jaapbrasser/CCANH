# ADR 0002: Streaming transport to the browser

- Status: accepted
- Date: 2026-11-18
- Phase: 3 (spike)
- Spike code: branch `spike/streaming` (throwaway, not merged)

## Context

Browsers can't play RTSP. SPEC §7 requires **one upstream connection per
(camera, profile) no matter how many viewers**, no transcoding in the default
path, H.264 support (H.265 treated as unsupported in the MVP), playback that
starts immediately from the latest keyframe, and slow viewers that never stall
the others. §7.3 lists four candidates: (A) fMP4 over WebSocket into MSE,
(B) WebRTC, (C) MJPEG over HTTP, (D) HLS / LL-HLS.

No AXIS camera was available (SPEC §18 Q1), so the spike used a local RTSP
source (§15.5): MediaMTX 1.21 fed by ffmpeg 9.0 `testsrc2`, H.264 Main
1280×720 @ 25 fps at 2 Mbit/s CBR plus 640×360 @ 15 fps at 400 kbit/s, 2 s GOP,
no B-frames. Server and browser ran on one Apple M3 Pro (macOS 27) with the
Chromium-based browser pane, using a release build of the spike.

## What the spike built (option A)

`retina` (RTSP client) → H.264 access units passed through unchanged → a
~150-line hand-written fMP4 muxer (init segment from `retina`'s `avc1` sample
entry, then one `moof`+`mdat` per frame) → a per-profile hub (single upstream
session, GOP cache, `tokio::sync::broadcast` fan-out) → a WebSocket per viewer →
MSE `SourceBuffer` in the browser, which jumps to the live edge after the GOP
replay.

## Measurements

| What | Result |
|---|---|
| Upstream sessions per profile | **1**, with 2 browser tabs + 50 load-generator viewers on `main` and a third tab on `grid` |
| Startup, warm (page load → `playing`, upstream already open) | **264–274 ms** |
| Startup, cold (first viewer: RTSP setup + wait for a keyframe) | **1.26 s** (bounded by RTSP setup + ≤ 1 GOP) |
| Latency, server ingest → on screen | **150–230 ms** (browser playback buffer ≈ 200–250 ms) |
| Server CPU / RSS, 1 viewer | 0.5 % of one core, 8.8 MB |
| 10 viewers | 0.9 %, 11.3 MB |
| 50 viewers (105 Mbit/s out) | **2.8 %**, 19.0 MB |
| Bandwidth, camera side | Stream bitrate once per profile (2.0 Mbit/s for `main`), independent of viewers |
| Bandwidth, per viewer | ≈ 2.1 Mbit/s for a 2 Mbit/s stream (fMP4 + framing overhead ≈ 3–5 %) |
| Slow viewer (reads 8 of 25 frames/s) | Resynced at keyframes (6 times in 45 s); the other 5 viewers kept full rate |
| MJPEG for the same 720p25 source (ffmpeg `-q:v` 10 → 3) | **6.6–12.1 Mbit/s**, vs 2.0–2.7 Mbit/s H.264: 3–4.5× the bandwidth |

Latency is measured from when the server receives a frame to when the browser
shows it, using timestamps sent with each fragment. Camera encoding and the
camera → server hop are excluded; they are the same for every option.
`testsrc2` is synthetic, so absolute bitrates differ from real scenes, but the
ratios hold.

## Options considered

**A. fMP4 over WebSocket → MSE**: met every requirement in the spike.
Passthrough, so CPU is negligible and the egress is the camera's bitrate per
viewer. Instant start from the GOP cache. Slow-viewer isolation comes naturally
from per-viewer tasks plus keyframe resync. Sub-250 ms added latency on the LAN
is well inside the spec's expectation (0.5–2 s). Simple server, no ICE/STUN/DTLS.
Gaps: no MSE on iOS Safari (it has `ManagedMediaSource`, and mobile is a
non-goal); B-frames need composition offsets in the muxer.

**B. WebRTC** (evaluated, not prototyped: A already met the bar). Lower latency
(typically 50–150 ms), but much more machinery: SDP signaling, ICE (host
candidates suffice on a LAN), DTLS-SRTP with per-viewer encryption cost,
RTP re-packetization and RTCP feedback handling. It also fits the shared
upstream model poorly: a viewer can't be handed a cached GOP as a burst
(jitter buffers drop late frames), so joins wait for the next keyframe (up to
one GOP). PLI/FIR keyframe requests can't be forwarded per viewer without
disturbing every other viewer of that upstream. The latency gain isn't worth
the complexity for the MVP.

**C. MJPEG over HTTP**: trivial and universally playable, but 3–4.5× the
bandwidth for the same picture. It also needs the camera to produce a separate
MJPEG stream, adding camera CPU load and an extra upstream connection, which
works against the project's main goal (§1).

**D. HLS / LL-HLS**: very compatible but adds seconds of latency and
segment/playlist management. No advantage over A on desktop browsers.

## Decision

1. **Default: option A**, fMP4 over a dedicated media WebSocket into MSE, with
   H.264 passed through and never transcoded.
2. **Fallback: option C** (MJPEG), only for clients without MSE/H.264 support
   or cameras without H.264. It's served through the same stream manager as a
   separate `(camera, "mjpeg")` key, so it's still shared. The VAPIX MJPEG
   endpoint must be verified and logged in `docs/vapix.md` before use.
3. **Later upgrade path: option B** behind the same subscriber interface, if a
   real need for lower latency appears.
4. **Codecs:** H.264 required. H.265 is reported as `Unsupported` for live view
   in the MVP: browser MSE support for HEVC varies by browser, OS and hardware.
   Cameras with B-frames are supported by writing composition offsets (`trun`
   sample composition time offsets) in the production muxer.

## Shape for Phase 5

`MediaSource` (SPEC §4.3) yields codec parameters and timestamped access units,
independent of transport:

```rust
pub struct VideoParams {
    pub codec: VideoCodec,          // H264 in the MVP
    pub rfc6381: String,            // e.g. "avc1.4D401F", for MSE
    pub width: u32,
    pub height: u32,
    pub sample_entry: Bytes,        // complete `avc1` box (avcC inside)
}

pub enum MediaEvent {
    /// First event, and again whenever SPS/PPS change.
    Params(VideoParams),
    /// One access unit, AVC (length-prefixed) format.
    Frame { pts: i64 /* 90 kHz */, dts: i64, keyframe: bool, data: Bytes },
}

#[async_trait]
pub trait MediaSource: Send {
    async fn next(&mut self) -> Result<Option<MediaEvent>, ProviderError>;
}
```

- **AxisProvider** wraps a `retina` session (RTSP over TCP, credentials
  never in URLs).
- **DemoProvider** parses the bundled H.264 clips (SPEC §12.3) into
  `Params` + `Frame`s at startup and replays them in real time, offsetting
  timestamps on each loop so they stay monotonic, with a keyframe at each loop
  start. Because it yields the same events, it feeds the same stream manager,
  GOP cache, muxer and egress, and subscriber counts and bandwidth metrics are
  real in demo mode.
- **Stream manager** owns one hub per `(camera, profile)` as in SPEC §7.2. It
  creates the hub in `Starting` state atomically (the spike used a mutex around
  get-or-create), and adds the grace period, reconnect with backoff, global
  buffer cap and metrics.
- **Egress:** `GET /api/v1/streams/{camera_id}/{profile}` upgrades to a media
  WebSocket, separate from the control socket (ADR 0007) and with the same
  cookie auth and origin check. It sends a text `{"type":"init","codec":…}`, a
  binary init segment, the cached GOP, then one binary fragment per frame,
  re-sending init on parameter changes. The frontend jumps to the live edge when
  more than ~1 s behind and trims the buffer.

## Consequences

- No native dependencies: `retina` and the muxer are pure Rust, so the Windows
  build stays simple.
- Camera-side bandwidth is one stream per active profile; viewer-side bandwidth
  is the stream bitrate per viewer, so grid views must use the low-resolution
  profile (§7.4).
- Live view needs MSE with H.264: current desktop Chrome, Edge, Firefox and
  Safari qualify.
- The numbers come from a local synthetic source on one machine. When a real
  AXIS camera is available (Phase 4/5), repeat the measurements against it and
  over the actual camera network, and add the results here.
