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
  - `B Storage – Aisle 4` rejects its credentials (auth failure).
- Cameras an admin adds in demo mode are simulated too: demo mode never opens a
  connection to a camera.

Live video (Phase 5), groups/tags/favorites (Phase 6), dashboards (Phase 7)
and discovery simulation (Phase 8) are added to demo mode as those phases land.
