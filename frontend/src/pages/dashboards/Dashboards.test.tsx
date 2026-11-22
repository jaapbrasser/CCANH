import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import type { CameraSummary } from "../../api/cameras";
import { fitTiles, moveTile, type Dashboard } from "../../api/dashboards";
import { LivePlayer } from "../../components/LivePlayer";
import { PAUSE_DELAY_MS } from "../../streaming/visibility";
import { renderApp } from "../../test/render";
import { VIEWER, problem, server, signedInAs } from "../../test/server";

const NOW = new Date().toISOString();
const A = "00000000-0000-7000-8000-000000000011";
const B = "00000000-0000-7000-8000-000000000012";
const C = "00000000-0000-7000-8000-000000000013";

function cam(id: string, name: string): CameraSummary {
  return {
    id,
    name,
    host: "192.0.2.1",
    port: 443,
    scheme: "https",
    manufacturer: "AXIS",
    model: "M1",
    firmware_version: null,
    monitoring_enabled: true,
    status: {
      state: "online",
      last_success_at: NOW,
      last_change_at: NOW,
      last_checked_at: NOW,
      last_failure_at: null,
      response_time_ms: 10,
      consecutive_failures: 0,
      error_category: null,
      error_message: null,
      identity_error_category: null,
      identity_error_message: null,
    },
    groups: [],
  };
}

function dash(over: Partial<Dashboard> = {}): Dashboard {
  return {
    id: "00000000-0000-7000-8000-0000000000d1",
    name: "Cranes",
    rows: 2,
    cols: 2,
    tiles: [
      { position: 0, camera_id: A },
      { position: 1, camera_id: B },
    ],
    is_default: true,
    created_at: NOW,
    updated_at: NOW,
    ...over,
  };
}

describe("layout helpers", () => {
  it("moves into empty cells and swaps with occupied ones", () => {
    const tiles = dash().tiles;
    expect(moveTile(tiles, 0, 3)).toEqual([
      { position: 1, camera_id: B },
      { position: 3, camera_id: A },
    ]);
    expect(moveTile(tiles, 0, 1)).toEqual([
      { position: 0, camera_id: B },
      { position: 1, camera_id: A },
    ]);
  });

  it("re-packs tiles when the grid shrinks and refuses when they can't fit", () => {
    const tiles = [
      { position: 0, camera_id: A },
      { position: 8, camera_id: B },
    ];
    expect(fitTiles(tiles, 1, 2)).toEqual([
      { position: 0, camera_id: A },
      { position: 1, camera_id: B },
    ]);
    expect(fitTiles(tiles, 1, 1)).toBeNull();
  });
});

describe("dashboards page", () => {
  let patches: unknown[];
  let failSave: boolean;

  beforeEach(() => {
    signedInAs(VIEWER);
    patches = [];
    failSave = false;
    server.use(
      http.get("/api/v1/dashboards", () =>
        HttpResponse.json([
          dash(),
          dash({ id: "00000000-0000-7000-8000-0000000000d2", name: "Gates", is_default: false, tiles: [] }),
        ]),
      ),
      http.get("/api/v1/cameras", () =>
        HttpResponse.json({
          items: [cam(A, "Hook 1"), cam(B, "Hook 2"), cam(C, "Gate North")],
          total: 3,
          page: 1,
          page_size: 200,
        }),
      ),
      http.patch("/api/v1/dashboards/:id", async ({ request }) => {
        const body = (await request.json()) as Partial<Dashboard>;
        patches.push(body);
        if (failSave) return problem(500, "internal", "Something went wrong.");
        return HttpResponse.json(dash(body));
      }),
    );
  });

  it("shows the default dashboard's cameras", async () => {
    renderApp("/dashboards");
    const grid = await screen.findByRole("grid", { name: "Dashboard grid, 2 by 2" });
    expect(await within(grid).findByRole("gridcell", { name: "Tile 1: Hook 1" })).toBeInTheDocument();
    expect(within(grid).getByRole("gridcell", { name: "Tile 2: Hook 2" })).toBeInTheDocument();
    expect(within(grid).getAllByRole("button", { name: /Add camera to tile/ })).toHaveLength(2);
    expect(screen.getByLabelText("Dashboard")).toHaveDisplayValue("Cranes (default)");
  });

  it("adds a camera from the picker and saves the layout after a pause", async () => {
    renderApp("/dashboards");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera to tile 3" }));
    const picker = screen.getByRole("dialog", { name: "Add a camera" });
    expect(await within(picker).findAllByText("On this dashboard")).toHaveLength(2);
    await userEvent.click(within(picker).getByRole("button", { name: "Add Gate North" }));
    expect(await screen.findByRole("gridcell", { name: "Tile 3: Gate North" })).toBeInTheDocument();
    await waitFor(() => expect(patches).toHaveLength(1), { timeout: 3000 });
    expect(patches[0]).toEqual({
      rows: 2,
      cols: 2,
      tiles: [
        { position: 0, camera_id: A },
        { position: 1, camera_id: B },
        { position: 2, camera_id: C },
      ],
    });
  });

  it("rolls back a layout that fails to save", async () => {
    failSave = true;
    renderApp("/dashboards");
    await userEvent.click(await screen.findByRole("button", { name: "Remove Hook 2 from dashboard" }));
    expect(screen.queryByRole("gridcell", { name: "Tile 2: Hook 2" })).not.toBeInTheDocument();
    await waitFor(() => expect(patches).toHaveLength(1), { timeout: 3000 });
    expect(await screen.findByRole("gridcell", { name: "Tile 2: Hook 2" })).toBeInTheDocument();
    expect(await screen.findByText(/Layout not saved/)).toBeInTheDocument();
  });

  it("changes the grid size, keeping cameras", async () => {
    renderApp("/dashboards");
    await screen.findByRole("gridcell", { name: "Tile 1: Hook 1" });
    await userEvent.selectOptions(screen.getByLabelText("Grid size"), "1 (1×1)");
    expect(await screen.findByText(/room for 1 cameras/)).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Grid size"), "9 (3×3)");
    expect(await screen.findByRole("grid", { name: "Dashboard grid, 3 by 3" })).toBeInTheDocument();
    await waitFor(() => expect(patches.at(-1)).toMatchObject({ rows: 3, cols: 3 }), { timeout: 3000 });
  });

  it("expands a tile and returns with Esc", async () => {
    server.use(http.get("/api/v1/cameras/:id", () => new HttpResponse(null, { status: 404 })));
    renderApp("/dashboards");
    await userEvent.click(await screen.findByRole("button", { name: "Expand Hook 1" }));
    expect(screen.getByRole("button", { name: "← Back to dashboard" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("button", { name: "← Back to dashboard" })).not.toBeInTheDocument();
  });

  it("offers to create the first dashboard", async () => {
    let created: unknown;
    server.use(
      http.get("/api/v1/dashboards", () => HttpResponse.json([])),
      http.post("/api/v1/dashboards", async ({ request }) => {
        created = await request.json();
        return HttpResponse.json(dash({ tiles: [] }), { status: 201 });
      }),
    );
    renderApp("/dashboards");
    await userEvent.click(await screen.findByRole("button", { name: "Create a dashboard" }));
    await userEvent.type(screen.getByLabelText("Name"), "Yard");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(created).toEqual({ name: "Yard", rows: 2, cols: 2 }));
  });
});

describe("pausing hidden players", () => {
  afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  });

  it("pauses after a delay while the tab is hidden and resumes when shown", () => {
    vi.useFakeTimers();
    render(<LivePlayer cameraId={A} label="Hook 1" pauseWhenHidden />);
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
      vi.advanceTimersByTime(PAUSE_DELAY_MS - 1);
    });
    expect(screen.queryByText("Paused while not visible")).not.toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByText("Paused while not visible")).toBeInTheDocument();
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(screen.queryByText("Paused while not visible")).not.toBeInTheDocument();
  });
});
