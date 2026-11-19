# Performance at target scale

SPEC §18 Q3 (default answer): up to **250 cameras, 25 concurrent users and 50
concurrent upstream streams**. `backend/tests/perf.rs` drives exactly that in
demo mode and asserts the results:

```bash
cargo test --release -p crane --test perf -- --ignored --nocapture
```

(`CRANE_PERF_SECS` changes the measured duration; default 60 s.)

## What the test does

- A file-backed SQLite database and the real HTTP server (`crane::serve`) on a
  local port: the same code paths as production. Demo providers simulate the
  cameras, without network access.
- **250 cameras** (the 24 demo cameras plus 226 more), all monitored at the
  default 10 s interval with jitter, identity refresh and offline backoff.
- **25 users** sign in. Each keeps a live-updates WebSocket (status, cameras,
  tasks) and requests a page of 50 cameras every second.
- **50 upstream streams**: 25 cameras × (detail profile at 1280×720/25 fps,
  grid profile at 640×360/15 fps), each watched by **2 viewers**, so 100
  media WebSockets.
- It asserts:
  - one upstream per (camera, profile);
  - every viewer starts at a keyframe and receives at least 80% of the frames
    the clip rate predicts;
  - no lag resyncs;
  - the API p95 is under 250 ms;
  - monitoring keeps at least 15 checks/s.

## Results

Run on 2026-11-19, release build, Apple M3 Pro (12 cores), macOS, 60 s:

| Measure | Result |
|---|---|
| Upstreams / viewers | 50 / 100 (2 each) |
| Status checks | 1,499 in 60 s (25.0/s, the full rate for 250 cameras at 10 s) |
| Live-update messages to 25 users | 41,300 |
| Frames delivered vs. expected | min 1.03, median 1.03 (all viewers kept up) |
| First fragment a keyframe | 100 / 100 |
| Lag resyncs / reconnects | 0 / 0 |
| `GET /api/v1/cameras?page_size=50` (1,489 requests) | p50 12.2 ms, p95 34.0 ms, max 46.6 ms |
| Inbound / outbound video | 14.6 / 30.9 Mbit/s (outbound ≈ 2 × inbound, as expected with 2 viewers per stream) |
| Process memory (server + all load clients) | 89 MiB RSS |
| CPU time (server + load clients, incl. setup) | 31 s over ~63 s (about half a core) |

"Frames delivered / expected" is a little above 1 because each viewer first
gets the cached GOP (up to 2 s of frames) at once.

## Caveats

- Demo clips are low-bitrate test patterns (about 0.3 Mbit/s per stream).
  Real AXIS streams are often 2–8 Mbit/s at 1080p. The server only copies
  bytes (no decoding or transcoding), so CPU scales with bitrate roughly
  linearly. The camera-network bandwidth lever is the grid profile and the
  connection limits (⚙ Settings → General → Live video).
- Server and load clients share one process and machine, so the CPU and
  memory figures are upper bounds for the server alone.
- Not yet measured on Windows or against real cameras (SPEC §18 Q1). Re-run
  the test on the target Windows host and add a row here.
