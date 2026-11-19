import { defineConfig, devices } from "@playwright/test";
import os from "node:os";
import path from "node:path";

// End-to-end tests run against the release binary in demo mode (SPEC §15):
// build it first with `pwsh scripts/build.ps1`. CRANE_E2E_EXE overrides the
// binary path. Each run starts a fresh demo database in a temp directory.
const PORT = Number(process.env.CRANE_E2E_PORT ?? 18080);
const exe =
  process.env.CRANE_E2E_EXE ??
  path.resolve(import.meta.dirname, "../target/release", process.platform === "win32" ? "crane.exe" : "crane");
const dataDir = path.join(os.tmpdir(), `crane-e2e-${process.pid}`);

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: "retain-on-failure",
    viewport: { width: 1400, height: 900 },
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1400, height: 900 } } }],
  webServer: {
    command: `"${exe}" --demo --data-dir "${dataDir}" --bind 127.0.0.1 --port ${PORT}`,
    url: `http://127.0.0.1:${PORT}/api/v1/health`,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
