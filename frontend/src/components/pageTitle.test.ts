import { describe, expect, it } from "vitest";
import { formatTitle } from "./pageTitle";

describe("formatTitle", () => {
  it("joins section and name", () => {
    expect(formatTitle("Dashboard", "North crane")).toBe(
      "Dashboard North crane - Crane Camera Automated Navigation Hub",
    );
  });
  it("shows only the section without a name", () => {
    expect(formatTitle("Cameras")).toBe("Cameras - Crane Camera Automated Navigation Hub");
    expect(formatTitle("Camera", "  ")).toBe("Camera - Crane Camera Automated Navigation Hub");
  });
  it("marks demo mode", () => {
    expect(formatTitle("Cameras", null, true)).toBe("[DEMO] Cameras - Crane Camera Automated Navigation Hub");
  });
});
