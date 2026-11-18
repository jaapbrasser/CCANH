import { screen } from "@testing-library/react";
import { HealthBadge } from "./HealthBadge";
import { mockFetchJson, renderWithQuery } from "../test/render";

const health = {
  status: "ok",
  version: "0.1.0",
  mode: "production",
  uptime_seconds: 5,
  checks: { database: { status: "ok" } },
};

describe("HealthBadge", () => {
  it("shows healthy status and version", async () => {
    mockFetchJson(200, health);
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server healthy · v0.1.0");
  });

  it("shows degraded status from a 503 body", async () => {
    mockFetchJson(503, { ...health, status: "degraded" });
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server degraded");
  });

  it("shows unreachable when the request fails", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("network down"));
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server unreachable");
  });
});
