import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "./schema";

export type Problem = components["schemas"]["Problem"];

/** An RFC 9457 problem returned by the API, thrown by {@link unwrap}. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly detail?: string | null;
  readonly retryAfter?: number;

  constructor(status: number, problem: Partial<Problem> | undefined, retryAfter?: number) {
    super(problem?.detail ?? problem?.title ?? `Request failed (HTTP ${status})`);
    this.name = "ApiError";
    this.status = status;
    this.code = problem?.code ?? "unknown";
    this.detail = problem?.detail;
    this.retryAfter = retryAfter;
  }
}

/** Fired when any request (other than the session probe) gets a 401. */
export const UNAUTHORIZED_EVENT = "crane:unauthorized";

const notifyUnauthorized: Middleware = {
  onResponse({ request, response }) {
    const path = new URL(request.url).pathname;
    const isAuthProbe = path === "/api/v1/auth/me" || path === "/api/v1/auth/login";
    if (response.status === 401 && !isAuthProbe) {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    }
    return response;
  },
};

/**
 * Typed API client generated from the backend's OpenAPI document.
 *
 * - Absolute origin: `Request` outside browsers (tests) rejects relative URLs.
 *   Same-origin in production and behind the Vite proxy in dev, so the browser
 *   sends a matching `Origin` header, which the server's CSRF check requires.
 * - `fetch` is looked up per call rather than captured at import time, so tests
 *   (and MSW) can replace it.
 */
export const api = createClient<paths>({
  baseUrl: window.location.origin,
  fetch: (request) => globalThis.fetch(request),
  credentials: "same-origin",
});
api.use(notifyUnauthorized);

interface FetchResult<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

/** Return `data` or throw an {@link ApiError}. */
export async function unwrap<T>(promise: Promise<FetchResult<T>>): Promise<T> {
  const { data, error, response } = await promise;
  if (response.ok) return data as T;
  const retry = Number(response.headers.get("retry-after"));
  throw new ApiError(
    response.status,
    error as Partial<Problem> | undefined,
    Number.isFinite(retry) && retry > 0 ? retry : undefined,
  );
}

/** A user-facing message for any thrown value. */
export function errorMessage(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.code === "rate_limited" && err.retryAfter) {
      return `Too many attempts. Try again in ${formatSeconds(err.retryAfter)}.`;
    }
    return err.message;
  }
  if (err instanceof Error) return err.message;
  return "Something went wrong.";
}

function formatSeconds(s: number): string {
  if (s < 60) return `${s} second${s === 1 ? "" : "s"}`;
  const m = Math.ceil(s / 60);
  return `${m} minute${m === 1 ? "" : "s"}`;
}
