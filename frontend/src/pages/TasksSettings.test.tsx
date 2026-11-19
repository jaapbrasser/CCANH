import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, live, problem, server, signedInAs } from "../test/server";

const NOW = new Date().toISOString();
const CAMERA_DEFAULTS = { scheme: "http", allow_basic_over_http: true, tls_mode: "insecure_skip_verify" };
const MONITORING = {
  interval_secs: 10,
  jitter_percent: 20,
  failure_threshold: 3,
  timeout_secs: 5,
  max_concurrent: 32,
  retries: 1,
  offline_max_interval_secs: 60,
  identity_refresh_minutes: 15,
};

const STREAMING = {
  grace_period_secs: 15,
  stream_buffer_mb: 4,
  global_buffer_mb: 256,
  default_grid_profile: null,
  default_detail_profile: null,
  max_upstreams: 0,
  detail_reserve: 2,
  max_inbound_mbps: 0,
};

const RETENTION = { status_history_days: 30, tasks_days: 30, audit_days: 0 };
const DISCOVERY = { max_addresses: 4096, concurrency: 32, connect_timeout_ms: 1000, http_timeout_ms: 3000 };

function task(over: Record<string, unknown> = {}) {
  return {
    id: "00000000-0000-7000-8000-0000000000f2",
    kind: "test_connection",
    created_by: ADMIN.id,
    created_by_username: "admin",
    status: "running",
    progress_current: 1,
    progress_total: 4,
    message: "Verifying credentials",
    result: null,
    created_at: NOW,
    ...over,
  };
}

describe("settings page", () => {
  beforeEach(() => {
    signedInAs(ADMIN);
    server.use(
      http.get("/api/v1/settings", () =>
        HttpResponse.json({
          current: { monitoring: MONITORING, streaming: STREAMING, discovery: DISCOVERY, retention: RETENTION, camera_defaults: CAMERA_DEFAULTS },
          defaults: { monitoring: MONITORING, streaming: STREAMING, discovery: DISCOVERY, retention: RETENTION, camera_defaults: CAMERA_DEFAULTS },
          monitor: {
            checks_total: 120,
            failures_total: 7,
            retries_total: 3,
            identity_checks_total: 5,
            checks_in_flight: 2,
            cameras_monitored: 24,
            last_tick_unix: 0,
          },
        }),
      ),
    );
  });

  it("shows monitor counters and saves only changed values", async () => {
    let patch: unknown;
    server.use(
      http.patch("/api/v1/settings/monitoring", async ({ request }) => {
        patch = await request.json();
        return HttpResponse.json({ ...MONITORING, interval_secs: 30 });
      }),
    );
    renderApp("/settings");
    const status = await screen.findByRole("region", { name: "Monitor status" });
    expect(status).toHaveTextContent("24");
    expect(status).toHaveTextContent("120");

    const interval = screen.getByLabelText("Check interval (seconds)");
    await userEvent.clear(interval);
    await userEvent.type(interval, "30");
    expect(screen.getByText(/Default: 10\./)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(patch).toEqual({ interval_secs: 30 }));
    expect(await screen.findByText("Settings saved and applied.")).toBeInTheDocument();
  });

  it("shows the server's validation message", async () => {
    server.use(
      http.patch("/api/v1/settings/monitoring", () =>
        problem(400, "bad_request", "interval_secs must be between 2 and 3600"),
      ),
    );
    renderApp("/settings");
    const interval = await screen.findByLabelText("Check interval (seconds)");
    await userEvent.clear(interval);
    await userEvent.type(interval, "1");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("between 2 and 3600");
  });

  it("saves live video settings separately", async () => {
    let patch: unknown;
    server.use(
      http.patch("/api/v1/settings/streaming", async ({ request }) => {
        patch = await request.json();
        return HttpResponse.json({ ...STREAMING, grace_period_secs: 60 });
      }),
    );
    renderApp("/settings");
    const form = await screen.findByRole("form", { name: "Live video settings" });
    const grace = within(form).getByLabelText("Grace period (seconds)");
    await userEvent.clear(grace);
    await userEvent.type(grace, "60");
    await userEvent.type(within(form).getByLabelText("Grid profile name"), "grid");
    await userEvent.click(within(form).getByRole("button", { name: "Save live video settings" }));
    await waitFor(() => expect(patch).toEqual({ grace_period_secs: 60, default_grid_profile: "grid" }));
  });
});

describe("retention settings", () => {
  it("saves changed values", async () => {
    signedInAs(ADMIN);
    let patch: unknown;
    server.use(
      http.get("/api/v1/settings", () =>
        HttpResponse.json({
          current: { monitoring: MONITORING, streaming: STREAMING, discovery: DISCOVERY, retention: RETENTION, camera_defaults: CAMERA_DEFAULTS },
          defaults: { monitoring: MONITORING, streaming: STREAMING, discovery: DISCOVERY, retention: RETENTION, camera_defaults: CAMERA_DEFAULTS },
          monitor: {
            checks_total: 0,
            failures_total: 0,
            retries_total: 0,
            identity_checks_total: 0,
            checks_in_flight: 0,
            cameras_monitored: 0,
            last_tick_unix: 0,
          },
        }),
      ),
      http.patch("/api/v1/settings/retention", async ({ request }) => {
        patch = await request.json();
        return HttpResponse.json({ ...RETENTION, audit_days: 365 });
      }),
    );
    renderApp("/settings");
    const form = await screen.findByRole("form", { name: "Retention settings" });
    const audit = within(form).getByLabelText("Audit log (days)");
    await userEvent.clear(audit);
    await userEvent.type(audit, "365");
    await userEvent.click(within(form).getByRole("button", { name: "Save retention settings" }));
    await waitFor(() => expect(patch).toEqual({ audit_days: 365 }));
  });
});

describe("camera defaults", () => {
  it("can be tightened", async () => {
    signedInAs(ADMIN);
    let patch: unknown;
    const all = { monitoring: MONITORING, streaming: STREAMING, discovery: DISCOVERY, retention: RETENTION, camera_defaults: CAMERA_DEFAULTS };
    server.use(
      http.get("/api/v1/settings", () =>
        HttpResponse.json({
          current: all,
          defaults: all,
          monitor: {
            checks_total: 0,
            failures_total: 0,
            retries_total: 0,
            identity_checks_total: 0,
            checks_in_flight: 0,
            cameras_monitored: 0,
            last_tick_unix: 0,
          },
        }),
      ),
      http.patch("/api/v1/settings/camera-defaults", async ({ request }) => {
        patch = await request.json();
        return HttpResponse.json(patch as Record<string, unknown>);
      }),
    );
    renderApp("/settings");
    const form = await screen.findByRole("form", { name: "Camera defaults" });
    expect(within(form).getByLabelText("Protocol")).toHaveValue("http");
    await userEvent.selectOptions(within(form).getByLabelText("Protocol"), "https");
    await userEvent.selectOptions(within(form).getByLabelText("Certificate check (HTTPS cameras)"), "strict");
    await userEvent.click(within(form).getByLabelText(/Allow Basic authentication over HTTP/));
    await userEvent.click(within(form).getByRole("button", { name: "Save camera defaults" }));
    await waitFor(() =>
      expect(patch).toEqual({ scheme: "https", allow_basic_over_http: false, tls_mode: "strict" }),
    );
  });
});

describe("activity page", () => {
  it("lists tasks and cancels a running one", async () => {
    signedInAs(ADMIN);
    let cancelled: string | undefined;
    server.use(
      http.get("/api/v1/tasks", () =>
        HttpResponse.json({
          items: [
            task(),
            task({
              id: "00000000-0000-7000-8000-0000000000f3",
              status: "failed",
              error_category: "timeout",
              error_message: "no answer",
            }),
          ],
          total: 2,
          page: 1,
          page_size: 25,
        }),
      ),
      http.post("/api/v1/tasks/:id/cancel", ({ params }) => {
        cancelled = params.id as string;
        return new HttpResponse(null, { status: 202 });
      }),
    );
    renderApp("/activity");
    const rows = await screen.findAllByRole("row");
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(3));
    const running = screen.getByText("Verifying credentials (1/4)").closest("tr")!;
    expect(screen.getByText(/Timed out: no answer/)).toBeInTheDocument();
    expect(rows.length).toBeGreaterThan(0);
    await userEvent.click(within(running).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(cancelled).toBe("00000000-0000-7000-8000-0000000000f2"));
  });
});

describe("task notifications", () => {
  it("toasts when my task finishes, not for others' tasks", async () => {
    signedInAs(ADMIN);
    renderApp("/cameras");
    await screen.findByText("Live");
    act(() => {
      live.broadcast(JSON.stringify({ type: "task_updated", task: task({ id: "00000000-0000-7000-8000-0000000000f9", created_by: "someone-else", status: "succeeded", result: { ok: true } }) }));
      live.broadcast(JSON.stringify({ type: "task_updated", task: task({ status: "succeeded", result: { ok: false } }) }));
    });
    expect(await screen.findByText("Connection test finished with problems.")).toBeInTheDocument();
    expect(screen.queryByText("Connection test finished.")).not.toBeInTheDocument();
    // A repeated terminal update for the same task doesn't toast twice.
    act(() => {
      live.broadcast(JSON.stringify({ type: "task_updated", task: task({ status: "succeeded", result: { ok: false } }) }));
    });
    expect(screen.getAllByText("Connection test finished with problems.")).toHaveLength(1);
  });
});

describe("test connection progress", () => {
  it("shows progress and can cancel the running test", async () => {
    signedInAs(ADMIN);
    let cancelled = false;
    server.use(
      http.post("/api/v1/cameras/test-connection", () => HttpResponse.json(task({ status: "queued", message: null }), { status: 202 })),
      http.get("/api/v1/tasks/:id", () => HttpResponse.json(cancelled ? task({ status: "cancelled", message: "cancelled" }) : task())),
      http.post("/api/v1/tasks/:id/cancel", () => {
        cancelled = true;
        return new HttpResponse(null, { status: 202 });
      }),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera" }));
    const dialog = screen.getByRole("dialog", { name: "Add camera" });
    await userEvent.type(within(dialog).getByLabelText("Host"), "192.0.2.90");
    await userEvent.click(within(dialog).getByRole("button", { name: "Test connection" }));
    const progress = await within(dialog).findByRole("status", { name: "Connection test progress" });
    await waitFor(() => expect(progress).toHaveTextContent("Verifying credentials (1/4)"));
    await userEvent.click(within(progress).getByRole("button", { name: "Cancel test" }));
    expect(await within(dialog).findByText("The connection test was cancelled.")).toBeInTheDocument();
  });
});
