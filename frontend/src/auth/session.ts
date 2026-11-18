import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { meKey } from "../api/auth";
import { UNAUTHORIZED_EVENT } from "../api/client";

/** Refetch the session whenever any API call reports 401. */
export function useUnauthorizedListener() {
  const qc = useQueryClient();
  useEffect(() => {
    const handler = () => qc.invalidateQueries({ queryKey: meKey });
    window.addEventListener(UNAUTHORIZED_EVENT, handler);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, handler);
  }, [qc]);
}

/** Only allow same-site relative paths as post-login redirects. */
export function safeNext(next: string | null): string {
  if (!next || !next.startsWith("/") || next.startsWith("//") || next.startsWith("/\\")) {
    return "/";
  }
  return next;
}
