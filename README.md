# CCANH

Crane Camera Automated Navigation Hub: a single Windows executable that manages AXIS
IP cameras from the browser, with shared upstream streams to keep camera-network
bandwidth low.

- Specification: [docs/SPEC.md](docs/SPEC.md)
- Development guide: [docs/development.md](docs/development.md)
- Decisions: [docs/adr/](docs/adr/)

## Quick start (development)

```bash
cargo run -p crane -- --data-dir data
```

Then open http://127.0.0.1:8080 (API docs at `/api/docs`). For UI work, run
`npm run dev` in `frontend/` as well. See the development guide.
