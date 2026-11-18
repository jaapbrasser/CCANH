import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components, operations } from "./schema";

export type AuditEntry = components["schemas"]["AuditEntry"];
export type AuditQuery = NonNullable<operations["list_audit_entries"]["parameters"]["query"]>;

export function useAudit(query: AuditQuery) {
  return useQuery({
    queryKey: ["audit", query],
    queryFn: () => unwrap(api.GET("/api/v1/audit", { params: { query } })),
    placeholderData: keepPreviousData,
  });
}
