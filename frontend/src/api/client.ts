import createClient from "openapi-fetch";
import type { paths } from "./schema";

/**
 * Typed API client generated from the backend's OpenAPI document.
 *
 * - Absolute origin: `Request` outside browsers (tests) rejects relative URLs.
 *   Same-origin in production and behind the Vite proxy in dev.
 * - `fetch` is looked up per call rather than captured at import time, so tests
 *   (and MSW later) can replace it.
 */
export const api = createClient<paths>({
  baseUrl: window.location.origin,
  fetch: (request) => globalThis.fetch(request),
});
