import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { HealthBadge } from "./HealthBadge";
import { renderWithQuery } from "../test/render";
import { server } from "../test/server";

const health = {
  status: "ok",
  version: "0.1.0",
  mode: "production",
  uptime_seconds: 5,
  checks: { database: { status: "ok" } },
};

describe("HealthBadge", () => {
  it("shows healthy status and version", async () => {
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server healthy · v0.1.0");
  });

  it("shows degraded status from a 503 body", async () => {
    server.use(
      http.get("/api/v1/health", () =>
        HttpResponse.json({ ...health, status: "degraded" }, { status: 503 }),
      ),
    );
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server degraded");
  });

  it("shows unreachable when the request fails", async () => {
    server.use(http.get("/api/v1/health", () => HttpResponse.error()));
    renderWithQuery(<HealthBadge />);
    expect(await screen.findByRole("status")).toHaveTextContent("Server unreachable");
  });
});
