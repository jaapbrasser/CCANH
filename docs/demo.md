# Demo mode

Run the full application with simulated cameras and no camera network:

```bash
cargo run -p crane -- --demo --data-dir data
```

or `mode = "demo"` in `config.toml`. The UI shows a **DEMO MODE** banner.

## What you get

- A separate database, `<data_dir>/demo.db`, recreated on every start. Pass
  `--demo-keep-state` to keep changes between runs. Demo data never touches
  `crane.db`.
- Demo accounts (they exist only in `demo.db`, and the login page lists them in
  demo mode):

  | Username | Password | Role |
  |---|---|---|
  | `admin` | `crane-demo-admin` | admin |
  | `viewer` | `crane-demo-viewer` | viewer |

- 24 cameras across Site A (Buildings 1–2, Yard) and Site B (Loading, Storage,
  Perimeter), using RFC 5737 documentation addresses that never route.
- Six simulated AXIS models with different capability sets (one PTZ, one older
  device without API discovery, HTTPS or H.265), so "not supported" paths show up.
- Behaviour, deterministic per camera:
  - most cameras are healthy with 15–90 ms response jitter;
  - `A Yard – Overview` is slow (450–900 ms);
  - `A Yard – Rail Spur` and `B Perimeter – Fence West` are always offline;
  - `B Loading – Dock 3` flaps online/offline every 45 s;
  - `B Storage – Aisle 4` rejects its credentials (auth failure);
  - any demo camera rejects the password `wrong`, so the discovery credential
    test and camera test connection can show a rejection.
- Discovery finds a few unregistered devices at `192.0.2.41–46` and
  `198.51.100.41–46`; `.42` and `.44` answer on HTTP only (to try Basic over
  HTTP), the rest on HTTPS.
- The SPEC §9 group tree (Site A → Building 1/2 → Crane 1–4, Yard; Site B →
  Loading Area, Storage Area, Perimeter) and a few favorites for each demo
  account.
- Two sample dashboards for each demo account: "Site A cranes" (3×3, the
  default) and "Gates and docks" (3×3). The PTZ pad beside the grid moves
  the two hook cameras (AXIS Q6135-LE); the demo video doesn't actually move.
- Restarting a demo camera makes it unreachable for 20 s.
- Cameras an admin adds in demo mode are simulated too: demo mode never opens a
  connection to a camera.
- Live video: every online camera with an H.264 profile plays one of four
  bundled test patterns (`assets/demo/`, regenerated with
  `scripts/gen-demo-clips.ps1`): 1280×720 at 25 fps for full-size profiles,
  640×360 at 15 fps for `grid`. Clips go through the real stream manager, so
  the **Streams** page shows true viewer counts and bandwidth: open one camera
  in three tabs and it shows one camera connection with three viewers. Offline
  cameras fail to stream; H.265-only models show why they can't play.

- Discovery: scanning `192.0.2.0/24` or `198.51.100.0/24` finds the online
  demo cameras (flagged as already registered) and 6 unregistered demo devices
  in each range (`.41`–`.46`), with realistic timing. `.1` answers like a
  router that isn't a camera. No socket is opened.
