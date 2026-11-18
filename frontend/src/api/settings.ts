import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type MonitoringSettings = components["schemas"]["MonitoringSettings"];
export type MonitoringPatch = components["schemas"]["MonitoringPatch"];

const key = ["settings"] as const;

export function useSettings() {
  return useQuery({
    queryKey: key,
    queryFn: () => unwrap(api.GET("/api/v1/settings")),
    refetchInterval: 5_000, // monitor counters
  });
}

export function useUpdateMonitoring() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: MonitoringPatch) =>
      unwrap(api.PATCH("/api/v1/settings/monitoring", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetMonitoring() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.DELETE("/api/v1/settings/monitoring")),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}
