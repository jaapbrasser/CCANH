import { expect, test } from "@playwright/test";

// Demo accounts from docs/demo.md; they exist only in the throwaway demo database.
const ADMIN = { username: "admin", password: "crane-demo-admin" };

test("settings cog, user menu and favorites from the selection", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Username").fill(ADMIN.username);
  await page.getByLabel("Password").fill(ADMIN.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/cameras/);

  // Only Dashboards and Cameras stay in the main navigation.
  const main = page.getByRole("navigation", { name: "Main" });
  await expect(main.getByRole("link")).toHaveText(["Dashboards", "Cameras"]);

  // The cog opens the settings area on General; the sidebar holds the rest.
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page).toHaveURL(/\/settings\/general$/);
  await expect(page.getByRole("heading", { name: "General" })).toBeVisible();
  await expect(page).toHaveTitle("[DEMO] General settings - Crane Camera Automated Navigation Hub");
  const sidebar = page.getByRole("navigation", { name: "Settings" });
  await sidebar.getByRole("link", { name: "Groups" }).click();
  await expect(page.getByRole("heading", { name: "Groups" })).toBeVisible();
  // Old addresses still work.
  await page.goto("/audit");
  await expect(page).toHaveURL(/\/settings\/audit$/);

  // Favorites from the selection, then the filter shows them.
  await page.goto("/cameras");
  await page.getByLabel("Select A Yard – Rail Spur").check();
  await page.getByLabel("Select A Yard – Overview").check();
  const bar = page.getByRole("region", { name: "Bulk actions" });
  await bar.getByLabel("Bulk action").selectOption("add_favorites");
  await bar.getByRole("button", { name: "Apply" }).click();
  await expect(page.getByText("2 cameras added to your favorites.")).toBeVisible();
  await page.getByLabel("Favorites only").check();
  await expect(page.getByRole("link", { name: "A Yard – Rail Spur" })).toBeVisible();

  // Hover the name for Account and Sign out.
  await page.getByRole("button", { name: /^admin/ }).hover();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login/);
});
