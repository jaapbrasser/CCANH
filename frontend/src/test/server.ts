import { http, HttpResponse, ws } from "msw";
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

/** The app's live-update socket. Tests can broadcast server messages through it. */
export const live = ws.link("ws://localhost:3000/api/v1/ws");

/** Messages the live socket received from the app, as parsed JSON. */
export const liveReceived: unknown[] = [];

const liveHandler = live.addEventListener("connection", ({ client }) => {
  client.send(JSON.stringify({ type: "hello", username: "test", role: "viewer", heartbeat_secs: 20 }));
  client.addEventListener("message", (event) => {
    const msg = JSON.parse(String(event.data));
    liveReceived.push(msg);
    if (msg.type === "subscribe") {
      client.send(JSON.stringify({ type: "subscribed", topics: msg.topics }));
    }
  });
});

export const server = setupServer(
  liveHandler,
  http.get("/api/v1/cameras", () => HttpResponse.json({ items: [], total: 0, page: 1, page_size: 50 })),
  http.get("/api/v1/cameras/summary", () => HttpResponse.json({ online: 0, offline: 0, unknown: 0 })),
  http.get("/api/v1/credential-sets", () => HttpResponse.json([])),
  http.get("/api/v1/groups", () => HttpResponse.json([])),
  http.get("/api/v1/favorites", () => HttpResponse.json([])),
  http.get("/api/v1/dashboards", () => HttpResponse.json([])),
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
