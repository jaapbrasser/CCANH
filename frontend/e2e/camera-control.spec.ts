import { expect, test } from "@playwright/test";

// Demo accounts from docs/demo.md; they exist only in the throwaway demo database.
const VIEWER = { username: "viewer", password: "crane-demo-viewer" };

test("0.4.0: camera count, PTZ pad, 2×2/3×3 grids and restart as a viewer", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Username").fill(VIEWER.username);
  await page.getByLabel("Password").fill(VIEWER.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/cameras/);

  // Top-bar count: unknown counts as offline, so the numbers always add up.
  await expect(page.getByRole("status", { name: /cameras online/ })).toHaveText(/^\d+ 🟢 \/ \d+ 🔴 \(\d+% online\)$/);

  // The default dashboard has the two PTZ hook cameras; the pad sits beside the grid.
  await page.goto("/dashboards");
  const pad = page.getByRole("complementary").getByRole("region", { name: "Camera controls" });
  await expect(pad).toContainText("Moves all 2 PTZ cameras on this dashboard", { timeout: 15_000 });
  await expect(page.getByLabel("Grid size").locator("option")).toHaveText(["4 (2×2)", "9 (3×3)"]);
  const moved = page.waitForResponse((r) => r.url().includes("/ptz") && r.request().method() === "POST");
  const left = pad.getByRole("button", { name: "Pan left" });
  await left.hover();
  await page.mouse.down();
  expect((await moved).status()).toBe(200);
  const stopped = page.waitForResponse(
    (r) => r.url().includes("/ptz") && r.request().postData() === JSON.stringify({ pan: 0, tilt: 0, zoom: 0 }),
  );
  await page.mouse.up();
  expect((await stopped).status()).toBe(200);

  // Restart from the camera page (a camera the other specs don't use).
  await page.goto("/cameras");
  await page.getByRole("link", { name: "B Storage – Overview" }).click();
  await page.getByRole("button", { name: "Restart camera" }).click();
  const dialog = page.getByRole("dialog", { name: "Restart B Storage – Overview?" });
  const restarted = page.waitForResponse((r) => r.url().endsWith("/restart"));
  await dialog.getByRole("button", { name: "Restart camera" }).click();
  expect((await restarted).status()).toBe(202);
  await expect(page.getByText("The camera accepted the restart.", { exact: false })).toBeVisible();
});
