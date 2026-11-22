import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import type { DiscoveredDevice, DiscoveryJobDetail } from "../api/discovery";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, live, server, signedInAs } from "../test/server";

const NOW = new Date().toISOString();
const JOB = "00000000-0000-7000-8000-0000000000e1";
const CAM = "00000000-0000-7000-8000-000000000011";

function device(n: number, over: Partial<DiscoveredDevice> = {}): DiscoveredDevice {
  return {
    id: `00000000-0000-7000-8000-0000000001${String(n).padStart(2, "0")}`,
    job_id: JOB,
    ip: `192.0.2.${n}`,
    port: 443,
    scheme: "https",
    manufacturer: "AXIS",
    model: "AXIS P1468-LE",
    firmware_version: "11.8.64",
    serial_number: `ACCC8E0000${String(n).padStart(2, "0")}`,
    mac_address: null,
    hostname: null,
    device_name: null,
    tls_fingerprint_sha256: null,
    response_time_ms: 40,
    registered_camera_id: null,
    duplicate_of: null,
    found_at: NOW,
    ...over,
  };
}

function job(status: "running" | "succeeded", results: DiscoveredDevice[]): DiscoveryJobDetail {
  return {
    id: JOB,
    ranges: ["192.0.2.0/24"],
    address_count: 254,
    scanned: status === "running" ? 100 : 254,
    responsive: results.length,
    found: results.length,
    created_at: NOW,
    task: {
      id: JOB,
      kind: "discovery",
      status,
      progress_current: 100,
      progress_total: 254,
      created_at: NOW,
    },
    results,
  };
}

describe("discovery page", () => {
  let detail: DiscoveryJobDetail;

  beforeEach(() => {
    signedInAs(ADMIN);
    detail = job("running", [
      device(41),
      device(11, {
        duplicate_of: { reason: "serial_number", camera_id: CAM, camera_name: "A1 Crane 1 – Hook", result_id: null },
      }),
    ]);
    server.use(
      http.get("/api/v1/discovery/jobs", () => HttpResponse.json([detail])),
      http.get("/api/v1/discovery/jobs/:id", () => HttpResponse.json(detail)),
      http.post("/api/v1/discovery/preview", async ({ request }) => {
        const { ranges } = (await request.json()) as { ranges: string[] };
        const big = ranges.some((r) => r.endsWith("/16"));
        return HttpResponse.json({
          address_count: big ? 65534 : 254,
          max_addresses: 4096,
          needs_confirmation: big,
          too_large: false,
        });
      }),
    );
  });

  it("previews the address count and requires confirmation for large scans", async () => {
    let started: unknown;
    server.use(
      http.post("/api/v1/discovery/jobs", async ({ request }) => {
        started = await request.json();
        return HttpResponse.json(detail, { status: 202 });
      }),
    );
    renderApp("/cameras/discovery");
    const form = await screen.findByRole("form", { name: "New scan" });
    const start = within(form).getByRole("button", { name: "Start scan" });
    await userEvent.type(within(form).getByLabelText("Address ranges"), "10.0.0.0/16");
    expect(await within(form).findByText("65,534 addresses")).toBeInTheDocument();
    expect(start).toBeDisabled();
    await userEvent.click(within(form).getByRole("checkbox", { name: /more than the usual limit of 4,096/ }));
    await userEvent.click(start);
    await waitFor(() => expect(started).toEqual({ ranges: ["10.0.0.0/16"], confirm_large: true }));
  });

  it("shows results as they stream in and progress as it's reported", async () => {
    renderApp("/cameras/discovery");
    const panel = await screen.findByRole("region", { name: "Scan results" });
    expect(await within(panel).findByText("https://192.0.2.41")).toBeInTheDocument();
    expect(within(panel).getByText("New")).toBeInTheDocument();
    expect(within(panel).getByText(/Already registered as/)).toHaveTextContent(
      "A1 Crane 1 – Hook (same serial number)",
    );
    expect(within(panel).queryByLabelText("Select 192.0.2.11")).not.toBeInTheDocument();

    await screen.findByText("Live");
    act(() => {
      live.broadcast(JSON.stringify({ type: "discovery_found", job_id: JOB, result: device(42) }));
      live.broadcast(
        JSON.stringify({ type: "discovery_progress", job_id: JOB, scanned: 200, total: 254, responsive: 3, found: 3 }),
      );
    });
    expect(await within(panel).findByText("https://192.0.2.42")).toBeInTheDocument();
    expect(
      within(panel).getByText(/200 of 254 addresses checked, 3 responding, 3 AXIS devices found/),
    ).toBeInTheDocument();
  });

  it("cancels a running scan", async () => {
    let cancelled = false;
    server.use(
      http.post("/api/v1/tasks/:id/cancel", () => {
        cancelled = true;
        return HttpResponse.json(detail.task);
      }),
    );
    renderApp("/cameras/discovery");
    await userEvent.click(await screen.findByRole("button", { name: "Cancel scan" }));
    await waitFor(() => expect(cancelled).toBe(true));
  });

  it("registers selected new devices with groups", async () => {
    detail = job("succeeded", detail.results);
    let body: unknown;
    server.use(
      http.get("/api/v1/groups", () =>
        HttpResponse.json([
          {
            id: "00000000-0000-7000-8000-0000000000c1",
            name: "New site",
            parent_id: null,
            camera_count: 0,
            created_at: NOW,
            updated_at: NOW,
          },
        ]),
      ),
      http.post("/api/v1/discovery/jobs/:id/register", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ registered: [], skipped: [] });
      }),
    );
    renderApp("/cameras/discovery");
    const panel = await screen.findByRole("region", { name: "Scan results" });
    expect(await within(panel).findByText(/Finished\./)).toBeInTheDocument();
    await userEvent.click(within(panel).getByLabelText("Select 192.0.2.41"));
    await userEvent.click(within(panel).getByRole("button", { name: "Register 1 selected" }));
    const dialog = screen.getByRole("dialog", { name: "Register 1 device" });
    await userEvent.click(await within(dialog).findByRole("checkbox", { name: "New site" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Register 1" }));
    await waitFor(() =>
      expect(body).toEqual({
        result_ids: [device(41).id],
        credentials: { type: "none" },
        group_ids: ["00000000-0000-7000-8000-0000000000c1"],
        monitoring_enabled: true,
        allow_basic_over_http: true,
      }),
    );
  });

  it("tests credentials, allows Basic over HTTP and creates a group inline", async () => {
    detail = job("succeeded", [device(41), device(42, { scheme: "http", port: 80, tls_fingerprint_sha256: null })]);
    let tested: unknown;
    let registered: unknown;
    const groups: {
      id: string;
      name: string;
      parent_id: null;
      camera_count: number;
      created_at: string;
      updated_at: string;
    }[] = [];
    const TASK = "00000000-0000-7000-8000-0000000000f9";
    server.use(
      http.get("/api/v1/groups", () => HttpResponse.json(groups)),
      http.post("/api/v1/groups", async ({ request }) => {
        const { name } = (await request.json()) as { name: string };
        const group = {
          id: "00000000-0000-7000-8000-0000000000c2",
          name,
          parent_id: null,
          camera_count: 0,
          created_at: NOW,
          updated_at: NOW,
        };
        groups.push(group);
        return HttpResponse.json(group, { status: 201 });
      }),
      http.post("/api/v1/discovery/jobs/:id/test-credentials", async ({ request }) => {
        tested = await request.json();
        return HttpResponse.json({ id: TASK, kind: "test_credentials", status: "queued" }, { status: 202 });
      }),
      http.get(`/api/v1/tasks/${TASK}`, () =>
        HttpResponse.json({
          id: TASK,
          kind: "test_credentials",
          status: "succeeded",
          result: {
            results: [
              { result_id: device(41).id, ip: "192.0.2.41", ok: true, error_category: null, message: null },
              {
                result_id: device(42).id,
                ip: "192.0.2.42",
                ok: false,
                error_category: "auth_failed",
                message: "authentication failed: camera rejected the configured credentials",
              },
            ],
          },
        }),
      ),
      http.post("/api/v1/discovery/jobs/:id/register", async ({ request }) => {
        registered = await request.json();
        return HttpResponse.json({ registered: [], skipped: [] });
      }),
    );
    renderApp("/cameras/discovery");
    const panel = await screen.findByRole("region", { name: "Scan results" });
    expect(await within(panel).findByText(/Finished\./)).toBeInTheDocument();
    await userEvent.click(within(panel).getByLabelText("Select 192.0.2.41"));
    await userEvent.click(within(panel).getByLabelText("Select 192.0.2.42"));
    await userEvent.click(within(panel).getByRole("button", { name: "Register 2 selected" }));
    const dialog = screen.getByRole("dialog", { name: "Register 2 devices" });

    // One of the two was found on HTTP: the Basic option shows, on by default.
    const basic = within(dialog).getByLabelText(/Allow Basic authentication over HTTP/);
    expect(basic).toBeChecked();
    expect(within(dialog).getByText(/Applies to 1 of 2 selected devices/)).toBeInTheDocument();

    await userEvent.click(within(dialog).getByLabelText("Username and password for these cameras"));
    await userEvent.type(within(dialog).getByLabelText("Username"), "root");
    await userEvent.type(within(dialog).getByLabelText("Password"), "pw-123");
    await userEvent.click(within(dialog).getByRole("button", { name: "Test credentials" }));
    const result = await within(dialog).findByRole("region", { name: "Credential test result" });
    expect(result).toHaveTextContent("Credentials accepted by 1 of 2 devices");
    expect(result).toHaveTextContent("Authentication failed");
    expect(tested).toEqual({
      result_ids: [device(41).id, device(42).id],
      credentials: { type: "inline", username: "root", password: "pw-123" },
      allow_basic_over_http: true,
    });

    await userEvent.type(within(dialog).getByLabelText("New group name"), "Site B{Enter}");
    expect(await within(dialog).findByRole("checkbox", { name: "Site B" })).toBeChecked();

    await userEvent.click(within(dialog).getByRole("button", { name: "Register 2" }));
    await waitFor(() =>
      expect(registered).toMatchObject({
        group_ids: ["00000000-0000-7000-8000-0000000000c2"],
        allow_basic_over_http: true,
      }),
    );
  });

  it("is admin only", async () => {
    signedInAs(VIEWER);
    renderApp("/cameras/discovery");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Discover cameras" })).not.toBeInTheDocument());
  });
});
