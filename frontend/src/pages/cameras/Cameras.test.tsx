import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import type { CameraDetail, CameraSummary } from "../../api/cameras";
import { renderApp } from "../../test/render";
import { ADMIN, VIEWER, live, liveReceived, problem, server, signedInAs } from "../../test/server";

const NOW = new Date().toISOString();

function summary(id: string, name: string, extra: Partial<CameraSummary> = {}): CameraSummary {
  return {
    id,
    name,
    host: `192.0.2.${id.slice(-2)}`,
    port: 443,
    scheme: "https",
    manufacturer: "AXIS",
    model: "AXIS Q6135-LE",
    firmware_version: "10.12.220",
    monitoring_enabled: true,
    status: {
      state: "online",
      last_success_at: NOW,
      last_change_at: NOW,
      last_checked_at: NOW,
      last_failure_at: null,
      response_time_ms: 42,
      consecutive_failures: 0,
      error_category: null,
      error_message: null,
      identity_error_category: null,
      identity_error_message: null,
    },
    groups: [],
    tags: [],
    ...extra,
  };
}

const HOOK = summary("00000000-0000-7000-8000-000000000011", "A1 Crane 1 – Hook");
const RAIL = summary("00000000-0000-7000-8000-000000000034", "A Yard – Rail Spur", {
  model: "AXIS F2105-RE",
  status: { ...HOOK.status, state: "offline", error_category: "unreachable", response_time_ms: null },
});

function detail(s: CameraSummary): CameraDetail {
  return {
    ...s,
    tls_mode: "strict",
    tls_pinned_sha256: null,
    has_credentials: true,
    credential_set: { id: "00000000-0000-7000-8000-0000000000c1", name: "Site A" },
    allow_basic_over_http: false,
    serial_number: "ACCC8E000011",
    mac_address: "AC:CC:8E:00:00:11",
    device_name: s.name,
    device_hostname: "axis-accc8e000011",
    network: { ipv4_address: s.host, subnet_mask: "255.255.255.0", gateway: null, dhcp: false },
    capabilities: [
      { key: "video.h264", group: "video", label: "H.264", supported: true, version: null, detail: null },
      { key: "ptz.control", group: "ptz", label: "Pan/tilt/zoom control", supported: true, version: "3.0", detail: null },
      { key: "audio.input", group: "audio", label: "Audio input", supported: false, version: null, detail: "Not available on this model" },
    ],
    stream_profiles: [
      { id: "main", name: "Main (1080p)", codec: "h264", width: 1920, height: 1080, fps: 30, bitrate_kbps: 4000 },
    ],
    grid_profile: null,
    detail_profile: null,
    identity_updated_at: NOW,
    created_at: NOW,
    updated_at: NOW,
  };
}

function listHandler() {
  return http.get("/api/v1/cameras", ({ request }) => {
    const url = new URL(request.url);
    const q = url.searchParams.get("q")?.toLowerCase();
    const status = url.searchParams.get("status");
    const items = [HOOK, RAIL].filter(
      (c) => (!q || c.name.toLowerCase().includes(q)) && (!status || c.status.state === status),
    );
    return HttpResponse.json({ items, total: items.length, page: 1, page_size: 50 });
  });
}

describe("camera list", () => {
  beforeEach(() => {
    signedInAs(VIEWER);
    server.use(listHandler());
  });

  it("is the landing page and shows status", async () => {
    renderApp("/");
    await screen.findByRole("heading", { name: "Cameras" });
    const row = (await screen.findByRole("link", { name: "A Yard – Rail Spur" })).closest("tr")!;
    expect(row).toHaveTextContent("Offline");
    expect(row).toHaveTextContent("Unreachable");
    expect(screen.queryByRole("button", { name: "Add camera" })).not.toBeInTheDocument();
  });

  it("filters by status and searches", async () => {
    renderApp("/cameras");
    await screen.findByRole("link", { name: "A1 Crane 1 – Hook" });
    await userEvent.selectOptions(screen.getByLabelText("Filter by status"), "offline");
    await waitFor(() =>
      expect(screen.queryByRole("link", { name: "A1 Crane 1 – Hook" })).not.toBeInTheDocument(),
    );
    await userEvent.selectOptions(screen.getByLabelText("Filter by status"), "");
    await userEvent.type(screen.getByLabelText("Search cameras"), "hook");
    await waitFor(() =>
      expect(screen.queryByRole("link", { name: "A Yard – Rail Spur" })).not.toBeInTheDocument(),
    );
  });

  it("subscribes to live updates and applies status changes", async () => {
    renderApp("/cameras");
    const link = await screen.findByRole("link", { name: "A1 Crane 1 – Hook" });
    await waitFor(() =>
      expect(liveReceived).toContainEqual({ type: "subscribe", topics: ["camera_status", "cameras", "tasks"] }),
    );
    await screen.findByText("Live");

    act(() => {
      live.broadcast(
        JSON.stringify({
          type: "camera_status",
          camera_id: HOOK.id,
          status: { ...HOOK.status, state: "offline", error_category: "timeout", response_time_ms: null },
        }),
      );
    });
    const row = link.closest("tr")!;
    await waitFor(() => expect(row).toHaveTextContent("Timed out"));
    expect(row).toHaveTextContent("Offline");
  });
});

describe("camera detail", () => {
  beforeEach(() => {
    server.use(
      listHandler(),
      http.get("/api/v1/cameras/:id", ({ params }) =>
        params.id === HOOK.id ? HttpResponse.json(detail(HOOK)) : problem(404, "not_found", "Not found"),
      ),
      http.get("/api/v1/cameras/:id/status-history", () =>
        HttpResponse.json({
          items: [{ id: 1, occurred_at: NOW, from_state: "unknown", to_state: "online", error_category: null }],
          total: 1,
          page: 1,
          page_size: 20,
        }),
      ),
    );
  });

  it("shows sections driven by capability groups", async () => {
    signedInAs(VIEWER);
    renderApp(`/cameras/${HOOK.id}`);
    await screen.findByRole("heading", { name: "A1 Crane 1 – Hook" });
    const ptz = screen.getByRole("region", { name: "Pan / tilt / zoom" });
    expect(within(ptz).getByText("Pan/tilt/zoom control")).toBeInTheDocument();
    const audio = screen.getByRole("region", { name: "Audio" });
    expect(within(audio).getByLabelText("Not supported")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Identity" })).toHaveTextContent("ACCC8E000011");
    expect(screen.getByRole("region", { name: "Stream profiles" })).toHaveTextContent("1920×1080");
    expect(screen.getByRole("region", { name: "Stream profiles" })).toHaveTextContent(
      "Grid tiles: Automatic · Detail view: Automatic",
    );
    const history = screen.getByRole("region", { name: "Status history" });
    await waitFor(() => expect(history).toHaveTextContent("unknown → online"));
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("shows a live view that explains when the browser can't play video", async () => {
    signedInAs(VIEWER);
    renderApp(`/cameras/${HOOK.id}`);
    // jsdom has no Media Source Extensions, so no stream is opened.
    const liveView = await screen.findByRole("region", { name: "Live view" });
    expect(within(liveView).getByLabelText("Live video: A1 Crane 1 – Hook")).toBeInTheDocument();
    expect(within(liveView).getByRole("status")).toHaveTextContent("Live video not supported");
  });

  it("lets admins choose grid and detail profiles; viewers see the choice", async () => {
    signedInAs(ADMIN);
    const profiles = [
      { id: "main", name: "Main (1080p)", codec: "h264", width: 1920, height: 1080, fps: 30, bitrate_kbps: 4000 },
      { id: "grid", name: "Grid (360p)", codec: "h264", width: 640, height: 360, fps: 15, bitrate_kbps: 400 },
    ];
    let body: unknown;
    server.use(
      http.get("/api/v1/cameras/:id", () => HttpResponse.json({ ...detail(HOOK), stream_profiles: profiles })),
      http.put("/api/v1/cameras/:id/stream-profiles", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ...detail(HOOK), stream_profiles: profiles, grid_profile: "grid" });
      }),
    );
    renderApp(`/cameras/${HOOK.id}`);
    const form = await screen.findByRole("form", { name: "Profile choices" });
    await userEvent.selectOptions(within(form).getByLabelText("Grid tiles"), "grid");
    await userEvent.click(within(form).getByRole("button", { name: "Save profile choices" }));
    await waitFor(() => expect(body).toEqual({ grid_profile: "grid", detail_profile: null }));
    await waitFor(() =>
      expect(within(screen.getByRole("form", { name: "Profile choices" })).getByLabelText("Grid tiles")).toHaveValue("grid"),
    );
  });

  it("explains when a camera has no H.264 profile", async () => {
    signedInAs(VIEWER);
    server.use(
      http.get("/api/v1/cameras/:id", () =>
        HttpResponse.json({
          ...detail(HOOK),
          stream_profiles: [{ id: "main", name: "Main", codec: "h265", width: 1920, height: 1080, fps: 30, bitrate_kbps: 4000 }],
        }),
      ),
    );
    renderApp(`/cameras/${HOOK.id}`);
    const liveView = await screen.findByRole("region", { name: "Live view" });
    expect(liveView).toHaveTextContent("only offers H265");
    expect(within(liveView).queryByLabelText(/Live video:/)).not.toBeInTheDocument();
  });

  it("warns about rejected credentials on an online camera", async () => {
    signedInAs(ADMIN);
    server.use(
      http.get("/api/v1/cameras/:id", () =>
        HttpResponse.json(
          detail({
            ...HOOK,
            status: {
              ...HOOK.status,
              identity_error_category: "auth_failed",
              identity_error_message: "authentication failed: the camera rejected the configured credentials",
            },
          }),
        ),
      ),
    );
    renderApp(`/cameras/${HOOK.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("Authentication failed");
    expect(screen.getByRole("alert")).toHaveTextContent("Check the camera's credentials under Edit.");
    expect(screen.getByRole("region", { name: "Connection" })).toHaveTextContent("Shared set: Site A");
  });

  it("reports a missing camera", async () => {
    signedInAs(VIEWER);
    renderApp("/cameras/00000000-0000-7000-8000-00000000dead");
    await screen.findByRole("heading", { name: "Camera not found" });
  });

  it("lets admins refresh and see camera errors", async () => {
    signedInAs(ADMIN);
    server.use(
      http.post("/api/v1/cameras/:id/refresh", () =>
        HttpResponse.json(
          {
            type: "urn:crane:problem:camera_error",
            title: "Camera Error",
            status: 502,
            code: "camera_error",
            detail: "authentication failed: camera rejected the configured credentials",
            error_category: "auth_failed",
          },
          { status: 502, headers: { "content-type": "application/problem+json" } },
        ),
      ),
    );
    renderApp(`/cameras/${HOOK.id}`);
    await userEvent.click(await screen.findByRole("button", { name: "Refresh details" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("authentication failed");
  });

  it("asks before removing", async () => {
    signedInAs(ADMIN);
    let removed = false;
    server.use(
      http.delete("/api/v1/cameras/:id", () => {
        removed = true;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp(`/cameras/${HOOK.id}`);
    await userEvent.click(await screen.findByRole("button", { name: "Remove" }));
    expect(removed).toBe(false);
    const dialog = screen.getByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove camera" }));
    await screen.findByRole("heading", { name: "Cameras" });
    expect(removed).toBe(true);
  });
});

describe("adding a camera", () => {
  it("posts the form and opens the new camera", async () => {
    signedInAs(ADMIN);
    let body: Record<string, unknown> = {};
    server.use(
      listHandler(),
      http.post("/api/v1/cameras", async ({ request }) => {
        body = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json(detail(HOOK), { status: 201 });
      }),
      http.get("/api/v1/cameras/:id", () => HttpResponse.json(detail(HOOK))),
      http.get("/api/v1/cameras/:id/status-history", () =>
        HttpResponse.json({ items: [], total: 0, page: 1, page_size: 20 }),
      ),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera" }));
    const dialog = screen.getByRole("dialog", { name: "Add camera" });
    await userEvent.type(within(dialog).getByLabelText("Name"), "Gate cam");
    await userEvent.type(within(dialog).getByLabelText("Host"), "192.0.2.77");
    await userEvent.selectOptions(within(dialog).getByLabelText("Certificate check"), "insecure_skip_verify");
    expect(within(dialog).getByRole("alert")).toHaveTextContent("won't be checked");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add camera" }));
    await screen.findByRole("heading", { name: "A1 Crane 1 – Hook" });
    expect(body).toMatchObject({
      name: "Gate cam",
      host: "192.0.2.77",
      port: null,
      scheme: "https",
      tls_mode: "insecure_skip_verify",
      tls_pinned_sha256: null,
      monitoring_enabled: true,
    });
  });
});

describe("demo mode", () => {
  it("shows the banner and demo accounts on the login page", async () => {
    signedInAs(null);
    server.use(
      http.get("/api/v1/health", () =>
        HttpResponse.json({
          status: "ok",
          version: "0.1.0",
          mode: "demo",
          uptime_seconds: 1,
          checks: { database: { status: "ok" } },
          services: {},
          demo_accounts: [{ username: "admin", password: "crane-demo-admin", role: "admin" }],
        }),
      ),
    );
    renderApp("/login");
    expect(await screen.findByText(/DEMO MODE/)).toBeInTheDocument();
    expect(screen.getByText("crane-demo-admin")).toBeInTheDocument();
  });
});
