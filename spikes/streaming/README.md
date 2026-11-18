# Streaming spike (throwaway)

Phase 3 prototype for SPEC §7.3, option A: RTSP (H.264 passthrough via `retina`)
→ fragmented MP4 → WebSocket → Media Source Extensions. One upstream session
per profile is shared by every viewer; new viewers start from a cached GOP.

**Not production code and never merged to `main`.** The decision and the
measurements are in `docs/adr/0002-streaming-transport.md` on `main`.

```bash
pwsh scripts/rtsp-test-source.ps1                  # terminal 1: MediaMTX + ffmpeg
cd spikes/streaming && cargo run --release         # terminal 2: spike on :9000
open "http://127.0.0.1:9000/?profile=main"         # any number of tabs
curl -s http://127.0.0.1:9000/stats                # upstream sessions, bytes, viewers
cargo run --release --bin load -- 50 18 slow       # 50 viewers + one slow one
```

Shortcuts taken: no B-frames (no composition offsets), one sample per
fragment, no auth, no grace period or teardown, fixed port, minimal errors.
