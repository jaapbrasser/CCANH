import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type MonitoringSettings = components["schemas"]["MonitoringSettings"];
export type MonitoringPatch = components["schemas"]["MonitoringPatch"];
export type StreamingSettings = components["schemas"]["StreamingSettings"];
export type StreamingPatch = components["schemas"]["StreamingPatch"];
export type DiscoverySettings = components["schemas"]["DiscoverySettings"];
export type DiscoveryPatch = components["schemas"]["DiscoveryPatch"];

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

export function useUpdateStreaming() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: StreamingPatch) => unwrap(api.PATCH("/api/v1/settings/streaming", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetStreaming() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.DELETE("/api/v1/settings/streaming")),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useUpdateDiscovery() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: DiscoveryPatch) => unwrap(api.PATCH("/api/v1/settings/discovery", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetDiscovery() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.DELETE("/api/v1/settings/discovery")),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}
