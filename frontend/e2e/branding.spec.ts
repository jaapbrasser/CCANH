import { expect, test } from "@playwright/test";

// Demo admin (docs/demo.md); exists only in the throwaway demo database.
const ADMIN = { username: "admin", password: "crane-demo-admin" };

test("icons are served for the browser and the app", async ({ request }) => {
  for (const [path, type] of [
    ["/favicon.ico", /image\/(x-icon|vnd\.microsoft\.icon)/],
    ["/favicon.svg", /image\/svg\+xml/],
    ["/icon-192.png", /image\/png/],
    ["/manifest.webmanifest", /json/],
  ] as const) {
    const res = await request.get(path);
    expect(res.status(), path).toBe(200);
    expect(res.headers()["content-type"], path).toMatch(type);
  }
});

test("tab titles name the section and the selected item", async ({ page }) => {
  await page.goto("/login");
  await expect(page).toHaveTitle("[DEMO] Sign in - Crane Camera Automated Navigation Hub");
  await page.getByLabel("Username").fill(ADMIN.username);
  await page.getByLabel("Password").fill(ADMIN.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveTitle("[DEMO] Cameras - Crane Camera Automated Navigation Hub");

  await page.goto("/dashboards");
  // Demo mode seeds "Site A cranes" as the default dashboard.
  await expect(page).toHaveTitle("[DEMO] Dashboard Site A cranes - Crane Camera Automated Navigation Hub");
});
