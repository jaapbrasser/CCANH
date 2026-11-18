import type { components } from "../api/schema";

type ErrorCategory = components["schemas"]["ErrorCategory"];

/** "12 s ago", "5 min ago", "3 h ago", or a date for older times. */
export function relativeTime(iso: string | null | undefined, now: number = Date.now()): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s} s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return new Date(iso).toLocaleDateString();
}

const CATEGORY_LABELS: Record<ErrorCategory, string> = {
  unreachable: "Unreachable",
  timeout: "Timed out",
  auth_failed: "Authentication failed",
  tls_error: "TLS error",
  unsupported: "Not supported",
  protocol_error: "Protocol error",
  rate_limited: "Rate limited",
  internal: "Internal error",
};

export function errorCategoryLabel(c: ErrorCategory | null | undefined): string | null {
  return c ? CATEGORY_LABELS[c] : null;
}
