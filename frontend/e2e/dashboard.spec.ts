import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

// The demo admin account is documented in docs/demo.md; it exists only in the
// throwaway demo database the test server starts with.
const ADMIN = { username: "admin", password: "crane-demo-admin" };

interface StreamInfo {
  camera_id: string;
  profile: string;
  state: string;
  subscribers: number;
  reconnects: number;
  started_at: string;
}

async function signIn(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Username").fill(ADMIN.username);
  await page.getByLabel("Password").fill(ADMIN.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/cameras/);
}

async function streams(request: APIRequestContext): Promise<StreamInfo[]> {
  const res = await request.get("/api/v1/streams");
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as StreamInfo[];
}

/** Streams someone is watching (ones left by an earlier page may still be in their grace period). */
async function watched(request: APIRequestContext): Promise<StreamInfo[]> {
  return (await streams(request)).filter((s) => s.subscribers > 0);
}

async function addCamera(page: Page, tile: number, search: string, name: string) {
  await page.getByRole("button", { name: `Add camera to tile ${tile}` }).click();
  const picker = page.getByRole("dialog", { name: "Add a camera" });
  await picker.getByLabel("Search cameras").fill(search);
  await picker.getByRole("button", { name: `Add ${name}` }).click();
  await expect(page.getByRole("gridcell", { name: `Tile ${tile}: ${name}` })).toBeVisible();
}

function saved(page: Page) {
  return page.waitForResponse((r) => r.url().includes("/api/v1/dashboards/") && r.request().method() === "PATCH");
}

/** Drag with real pointer moves (dnd-kit needs movement past its threshold). */
async function drag(page: Page, from: string, toTile: number) {
  const handle = await page.getByRole("button", { name: `Move ${from}`, exact: true }).boundingBox();
  const target = await page.getByRole("button", { name: `Add camera to tile ${toTile}` }).boundingBox();
  if (!handle || !target) throw new Error("drag endpoints not visible");
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x + handle.width / 2 + 10, handle.y + handle.height / 2 + 10, { steps: 5 });
  await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 15 });
  await page.mouse.up();
}

test("build a dashboard, rearrange it without restarting streams, and find it again", async ({ page, browser }) => {
  await signIn(page);
  await page.goto("/dashboards");
  // Demo mode seeds two dashboards; the first is the default.
  await expect(page.getByLabel("Dashboard", { exact: true }).locator("option:checked")).toHaveText(
    "Site A cranes (default)",
  );

  await page.getByRole("button", { name: "New dashboard" }).click();
  await page.getByLabel("Name").fill("E2E dashboard");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("grid", { name: "Dashboard grid, 2 by 2" })).toBeVisible();
  const dashboardUrl = page.url();

  const firstSave = saved(page);
  await addCamera(page, 1, "Gate North", "A Yard – Gate North");
  await addCamera(page, 2, "Overview", "A Yard – Overview");
  expect((await firstSave).ok()).toBeTruthy();

  // One upstream per tile, both running.
  await expect
    .poll(async () => (await watched(page.request)).filter((s) => s.state === "running").length, { timeout: 20_000 })
    .toBe(2);
  const before = await watched(page.request);

  // Move Gate North into the empty tile 4 and wait for the layout to save.
  const moveSave = saved(page);
  await drag(page, "A Yard – Gate North", 4);
  await expect(page.getByRole("gridcell", { name: "Tile 4: A Yard – Gate North" })).toBeVisible();
  expect((await moveSave).ok()).toBeTruthy();

  // SPEC §17 Phase 7: rearranging doesn't restart visible streams.
  const after = await watched(page.request);
  expect(after).toHaveLength(before.length);
  for (const s of before) {
    const now = after.find((a) => a.camera_id === s.camera_id && a.profile === s.profile);
    expect(now, `stream for ${s.camera_id} still exists`).toBeDefined();
    expect(now!.started_at).toBe(s.started_at);
    expect(now!.reconnects).toBe(0);
    expect(now!.subscribers).toBe(1);
  }

  // The layout persists across sessions: a fresh browser context sees it.
  const other = await browser.newContext();
  const page2 = await other.newPage();
  await signIn(page2);
  await page2.goto(dashboardUrl);
  await expect(page2.getByRole("gridcell", { name: "Tile 4: A Yard – Gate North" })).toBeVisible();
  await expect(page2.getByRole("gridcell", { name: "Tile 2: A Yard – Overview" })).toBeVisible();
  await other.close();
});

test("expanded view opens the detail stream and Esc returns", async ({ page }) => {
  await signIn(page);
  await page.goto("/dashboards");
  await page.getByRole("button", { name: "Expand A1 Crane 1 – Hook" }).click();
  const dialog = page.getByRole("dialog", { name: "A1 Crane 1 – Hook" });
  await expect(dialog.getByRole("complementary", { name: "Camera details" })).toContainText("AXIS");
  await expect
    .poll(async () => (await streams(page.request)).map((s) => `${s.profile}:${s.state}`).sort(), { timeout: 20_000 })
    .toContain("main:running");
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});
