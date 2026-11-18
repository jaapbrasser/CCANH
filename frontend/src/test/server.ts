import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import type { components } from "../api/schema";

type Me = components["schemas"]["Me"];
type User = components["schemas"]["User"];

export const ADMIN: Me = { id: "00000000-0000-7000-8000-000000000001", username: "admin", role: "admin" };
export const VIEWER: Me = { id: "00000000-0000-7000-8000-000000000002", username: "viewer", role: "viewer" };

export function user(me: Me, extra: Partial<User> = {}): User {
  const now = "2026-11-18T12:00:00Z";
  return {
    ...me,
    disabled: false,
    created_at: now,
    updated_at: now,
    password_changed_at: now,
    ...extra,
  };
}

export function problem(status: number, code: string, detail: string, headers?: Record<string, string>) {
  return HttpResponse.json(
    { type: `urn:crane:problem:${code}`, title: code, status, code, detail },
    { status, headers: { "content-type": "application/problem+json", ...headers } },
  );
}

/** Signed in as `me` (or signed out when null). */
export function signedInAs(me: Me | null) {
  server.use(
    http.get("/api/v1/auth/me", () =>
      me ? HttpResponse.json(me) : problem(401, "unauthorized", "Sign in to continue."),
    ),
  );
}

export const server = setupServer(
  http.get("/api/v1/health", () =>
    HttpResponse.json({
      status: "ok",
      version: "0.1.0",
      mode: "production",
      uptime_seconds: 1,
      checks: { database: { status: "ok" } },
    }),
  ),
  http.get("/api/v1/setup", () => HttpResponse.json({ setup_required: false })),
  http.get("/api/v1/auth/me", () => problem(401, "unauthorized", "Sign in to continue.")),
);
