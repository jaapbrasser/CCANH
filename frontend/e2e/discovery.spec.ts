import { expect, test } from "@playwright/test";

// Demo admin from docs/demo.md; exists only in the throwaway demo database.
const ADMIN = { username: "admin", password: "crane-demo-admin" };

test("scan a /24, watch results arrive, cancel a large scan, register a device", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Username").fill(ADMIN.username);
  await page.getByLabel("Password").fill(ADMIN.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/cameras/);
  // Discovery lives under Cameras (0.3.0).
  await page.getByRole("button", { name: "Discover cameras" }).click();
  await expect(page).toHaveURL(/\/cameras\/discovery$/);

  const form = page.getByRole("form", { name: "New scan" });
  await form.getByLabel("Address ranges").fill("192.0.2.0/24");
  await expect(form.getByText("254 addresses")).toBeVisible();
  await form.getByRole("button", { name: "Start scan" }).click();

  const panel = page.getByRole("region", { name: "Scan results" });
  // The UI stays usable during the scan and results show up live.
  await expect(panel.getByText("https://192.0.2.41")).toBeVisible();
  await expect(panel.getByText(/Finished\. 254 of 254 addresses checked/)).toBeVisible({ timeout: 30_000 });
  // Registered demo cameras are flagged as duplicates; new devices aren't.
  await expect(panel.getByText(/Already registered as/).first()).toBeVisible();
  await expect(panel.getByText("New", { exact: true })).toHaveCount(6);

  await panel.getByLabel("Select 192.0.2.43").check();
  await panel.getByRole("button", { name: "Register 1 selected" }).click();
  await page.getByRole("dialog", { name: "Register 1 device" }).getByRole("button", { name: "Register 1" }).click();
  await expect(panel.getByRole("row", { name: /192\.0\.2\.43/ }).getByText("Registered")).toBeVisible();

  // A large scan needs confirmation, and can be cancelled.
  await form.getByLabel("Address ranges").fill("10.0.0.0/18");
  await expect(form.getByRole("button", { name: "Start scan" })).toBeDisabled();
  await form.getByRole("checkbox", { name: /more than the usual limit/ }).check();
  await form.getByRole("button", { name: "Start scan" }).click();
  await expect(panel.getByRole("heading", { name: "10.0.0.0/18" })).toBeVisible();
  await panel.getByRole("button", { name: "Cancel scan" }).click();
  await expect(panel.getByText(/^Cancelled\./)).toBeVisible();
});
