import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import type { CameraSummary } from "../api/cameras";
import { groupTree, subtreeIds, type Group } from "../api/organization";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, server, signedInAs } from "../test/server";

const NOW = new Date().toISOString();

function group(id: string, name: string, parent_id: string | null = null, camera_count = 0): Group {
  return { id, name, parent_id, camera_count, created_at: NOW, updated_at: NOW };
}

const SITE = group("00000000-0000-7000-8000-0000000000a1", "Site A");
const BUILDING = group("00000000-0000-7000-8000-0000000000a2", "Building 1", SITE.id, 2);
const YARD = group("00000000-0000-7000-8000-0000000000a3", "Yard", SITE.id);
const SITE_B = group("00000000-0000-7000-8000-0000000000b1", "Site B");
const GROUPS = [YARD, SITE_B, BUILDING, SITE];
const TAG = { id: "00000000-0000-7000-8000-0000000000c1", name: "Outdoor", camera_count: 1, created_at: NOW, updated_at: NOW };

function cam(id: string, name: string, extra: Partial<CameraSummary> = {}): CameraSummary {
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
    tags: [],
    ...extra,
  };
}

const HOOK = cam("00000000-0000-7000-8000-000000000011", "Crane Hook", {
  groups: [{ id: BUILDING.id, name: BUILDING.name }],
  tags: [{ id: TAG.id, name: TAG.name }],
});
const GATE = cam("00000000-0000-7000-8000-000000000012", "Gate North");

let lastQuery: URLSearchParams;

beforeEach(() => {
  lastQuery = new URLSearchParams();
  server.use(
    http.get("/api/v1/groups", () => HttpResponse.json(GROUPS)),
    http.get("/api/v1/tags", () => HttpResponse.json([TAG])),
    http.get("/api/v1/cameras", ({ request }) => {
      lastQuery = new URL(request.url).searchParams;
      return HttpResponse.json({ items: [HOOK, GATE], total: 2, page: 1, page_size: 50 });
    }),
  );
});

describe("group tree helpers", () => {
  it("orders parents before children and builds paths", () => {
    const tree = groupTree(GROUPS);
    expect(tree.map((g) => `${"-".repeat(g.depth)}${g.name}`)).toEqual(["Site A", "-Building 1", "-Yard", "Site B"]);
    expect(tree[1]!.path).toBe("Site A / Building 1");
    expect([...subtreeIds(GROUPS, SITE.id)].sort()).toEqual([SITE.id, BUILDING.id, YARD.id].sort());
  });
});

describe("camera list organization", () => {
  it("filters by group, tag and favorites and shows memberships", async () => {
    signedInAs(VIEWER);
    renderApp("/cameras");
    const table = await screen.findByRole("table");
    expect(await within(table).findByRole("list", { name: "Groups of Crane Hook" })).toHaveTextContent("Building 1");
    expect(within(table).getByRole("list", { name: "Tags of Crane Hook" })).toHaveTextContent("Outdoor");
    expect(screen.queryByLabelText("Select Crane Hook")).not.toBeInTheDocument();

    await screen.findByRole("option", { name: /Building 1/ });
    await userEvent.selectOptions(screen.getByLabelText("Filter by group"), SITE.id);
    await waitFor(() => expect(lastQuery.get("group_id")).toBe(SITE.id));
    await userEvent.selectOptions(screen.getByLabelText("Filter by tag"), TAG.id);
    await waitFor(() => expect(lastQuery.get("tag_id")).toBe(TAG.id));
    await userEvent.click(screen.getByLabelText("Favorites only"));
    await waitFor(() => expect(lastQuery.get("favorites")).toBe("true"));
  });

  it("toggles a favorite", async () => {
    signedInAs(VIEWER);
    let added: string | undefined;
    server.use(
      http.get("/api/v1/favorites", () => HttpResponse.json(added ? [added] : [])),
      http.put("/api/v1/favorites/:id", ({ params }) => {
        added = params.id as string;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/cameras");
    const star = await screen.findByRole("button", { name: "Add Gate North to favorites" });
    await userEvent.click(star);
    await waitFor(() => expect(added).toBe(GATE.id));
    expect(await screen.findByRole("button", { name: "Remove Gate North from favorites" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("favorites view asks for the user's favorites", async () => {
    signedInAs(VIEWER);
    renderApp("/favorites");
    expect(await screen.findByRole("heading", { name: "Favorites" })).toBeInTheDocument();
    await waitFor(() => expect(lastQuery.get("favorites")).toBe("true"));
  });

  it("bulk-tags selected cameras and confirms bulk delete", async () => {
    signedInAs(ADMIN);
    const bodies: unknown[] = [];
    server.use(
      http.post("/api/v1/cameras/bulk", async ({ request }) => {
        bodies.push(await request.json());
        return HttpResponse.json(
          {
            id: "00000000-0000-7000-8000-0000000000f9",
            kind: "bulk_cameras",
            status: "queued",
            progress_current: 0,
            created_at: NOW,
          },
          { status: 202 },
        );
      }),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByLabelText("Select all cameras on this page"));
    const bar = screen.getByRole("region", { name: "Bulk actions" });
    expect(bar).toHaveTextContent("2 selected");
    await userEvent.selectOptions(within(bar).getByLabelText("Bulk action"), "add_tags");
    await userEvent.selectOptions(within(bar).getByLabelText("Tag"), TAG.id);
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(bodies[0]).toEqual({ camera_ids: [HOOK.id, GATE.id], action: { type: "add_tags", tag_ids: [TAG.id] } }),
    );

    await userEvent.click(screen.getByLabelText("Select Gate North"));
    const bar2 = screen.getByRole("region", { name: "Bulk actions" });
    await userEvent.selectOptions(within(bar2).getByLabelText("Bulk action"), "delete");
    await userEvent.click(within(bar2).getByRole("button", { name: "Apply" }));
    const dialog = screen.getByRole("dialog", { name: "Delete 1 camera?" });
    expect(bodies).toHaveLength(1);
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete 1 camera" }));
    await waitFor(() => expect(bodies[1]).toEqual({ camera_ids: [GATE.id], action: { type: "delete" } }));
  });

  it("assigns groups and tags when adding a camera", async () => {
    signedInAs(ADMIN);
    let body: Record<string, unknown> | undefined;
    server.use(
      http.post("/api/v1/cameras", async ({ request }) => {
        body = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json({ ...HOOK, id: "00000000-0000-7000-8000-000000000099" }, { status: 201 });
      }),
      http.get("/api/v1/cameras/:id", () => new HttpResponse(null, { status: 404 })),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera" }));
    const dialog = screen.getByRole("dialog", { name: "Add camera" });
    await userEvent.type(within(dialog).getByLabelText("Name"), "New");
    await userEvent.type(within(dialog).getByLabelText(/Host/), "192.0.2.50");
    await userEvent.click(await within(dialog).findByRole("checkbox", { name: "Yard" }));
    await userEvent.click(within(dialog).getByRole("checkbox", { name: "Outdoor" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Add camera" }));
    await waitFor(() => expect(body).toMatchObject({ group_ids: [YARD.id], tag_ids: [TAG.id] }));
  });
});

describe("bulk camera settings", () => {
  const SET = { id: "00000000-0000-7000-8000-0000000000d1", name: "Site viewer", camera_count: 0, created_at: NOW, updated_at: NOW };
  function capture(bodies: unknown[]) {
    return [
      http.get("/api/v1/credential-sets", () => HttpResponse.json([SET])),
      http.post("/api/v1/cameras/bulk", async ({ request }) => {
        bodies.push(await request.json());
        return HttpResponse.json(
          { id: "00000000-0000-7000-8000-0000000000fa", kind: "bulk_cameras", status: "queued", progress_current: 0, created_at: NOW },
          { status: 202 },
        );
      }),
    ];
  }

  it("changes only the ticked settings for selected cameras", async () => {
    signedInAs(ADMIN);
    const bodies: unknown[] = [];
    server.use(...capture(bodies));
    renderApp("/cameras");
    await userEvent.click(await screen.findByLabelText("Select all cameras on this page"));
    const bar = screen.getByRole("region", { name: "Bulk actions" });
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    const dialog = screen.getByRole("dialog", { name: "Edit camera settings" });
    expect(within(dialog).getByRole("button", { name: "Apply to 2 selected cameras" })).toBeDisabled();
    await userEvent.click(within(dialog).getByLabelText("Change credentials"));
    await userEvent.selectOptions(await within(dialog).findByLabelText("Credential set"), SET.id);
    await userEvent.click(within(dialog).getByLabelText("Change basic authentication over http"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Apply to 2 selected cameras" }));
    await waitFor(() =>
      expect(bodies[0]).toEqual({
        camera_ids: [HOOK.id, GATE.id],
        action: {
          type: "update_settings",
          settings: { credentials: { type: "set", id: SET.id }, allow_basic_over_http: true },
        },
      }),
    );
  });

  it("applies to a whole group or tag from the organization page", async () => {
    signedInAs(ADMIN);
    const bodies: unknown[] = [];
    server.use(...capture(bodies));
    renderApp("/organization");
    await userEvent.click(await screen.findByRole("button", { name: "Camera settings for Site A" }));
    let dialog = screen.getByRole("dialog", { name: "Edit camera settings" });
    await userEvent.click(within(dialog).getByLabelText("Change certificate check"));
    await userEvent.selectOptions(within(dialog).getByLabelText("Certificate check"), "strict");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Apply to cameras in Site A/ }));
    await waitFor(() =>
      expect(bodies[0]).toEqual({
        group_id: SITE.id,
        include_subgroups: true,
        action: { type: "update_settings", settings: { tls_mode: "strict" } },
      }),
    );

    await userEvent.click(await screen.findByRole("button", { name: "Camera settings for Outdoor" }));
    dialog = screen.getByRole("dialog", { name: "Edit camera settings" });
    await userEvent.click(within(dialog).getByLabelText("Change protocol and port"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Apply to cameras tagged Outdoor" }));
    await waitFor(() =>
      expect(bodies[1]).toEqual({
        tag_id: TAG.id,
        action: { type: "update_settings", settings: { scheme: "http", port: 80 } },
      }),
    );
  });
});

describe("groups and tags page", () => {
  it("creates a subgroup and deletes a group with the chosen mode", async () => {
    signedInAs(ADMIN);
    let created: unknown;
    let deleted: string | null = null;
    server.use(
      http.post("/api/v1/groups", async ({ request }) => {
        created = await request.json();
        return HttpResponse.json(group("00000000-0000-7000-8000-0000000000a9", "Building 2", SITE.id), { status: 201 });
      }),
      http.delete("/api/v1/groups/:id", ({ request }) => {
        deleted = new URL(request.url).searchParams.get("mode");
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/organization");
    const groups = await screen.findByRole("region", { name: "Groups" });
    await within(groups).findByRole("button", { name: "Edit Site A / Building 1" });
    const form = within(groups).getByRole("form", { name: "New group" });
    await userEvent.type(within(form).getByLabelText("New group"), "Building 2");
    await userEvent.selectOptions(within(form).getByLabelText("Parent group"), SITE.id);
    await userEvent.click(within(form).getByRole("button", { name: "Add group" }));
    await waitFor(() => expect(created).toEqual({ name: "Building 2", parent_id: SITE.id }));

    await userEvent.click(within(groups).getByRole("button", { name: "Delete Site A / Building 1" }));
    const dialog = screen.getByRole("dialog", { name: "Delete Building 1?" });
    expect(dialog).toHaveTextContent('cameras join "Site A"');
    await userEvent.click(within(dialog).getByLabelText(/Detach/));
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete group" }));
    await waitFor(() => expect(deleted).toBe("detach"));
  });

  it("won't offer a group's own subtree as its new parent", async () => {
    signedInAs(ADMIN);
    renderApp("/organization");
    const groups = await screen.findByRole("region", { name: "Groups" });
    await userEvent.click(await within(groups).findByRole("button", { name: "Edit Site A" }));
    const dialog = screen.getByRole("dialog", { name: "Edit Site A" });
    const options = within(within(dialog).getByLabelText("Move under"))
      .getAllByRole("option")
      .map((o) => o.textContent?.trim());
    expect(options).toEqual(["Top level", "Site B"]);
  });

  it("is admin only", async () => {
    signedInAs(VIEWER);
    renderApp("/organization");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Groups & tags" })).not.toBeInTheDocument());
  });
});
