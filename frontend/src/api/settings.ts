import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type MonitoringSettings = components["schemas"]["MonitoringSettings"];
export type MonitoringPatch = components["schemas"]["MonitoringPatch"];
export type StreamingSettings = components["schemas"]["StreamingSettings"];
export type StreamingPatch = components["schemas"]["StreamingPatch"];
export type DiscoverySettings = components["schemas"]["DiscoverySettings"];
export type DiscoveryPatch = components["schemas"]["DiscoveryPatch"];
export type RetentionSettings = components["schemas"]["RetentionSettings"];
export type RetentionPatch = components["schemas"]["RetentionPatch"];

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

export function useUpdateRetention() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: RetentionPatch) => unwrap(api.PATCH("/api/v1/settings/retention", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetRetention() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.DELETE("/api/v1/settings/retention")),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export type CameraDefaults = components["schemas"]["CameraDefaults"];
export type CameraDefaultsPatch = components["schemas"]["CameraDefaultsPatch"];

/** What a new camera gets when the server's defaults can't be read yet (ADR 0011). */
export const SHIPPED_CAMERA_DEFAULTS: CameraDefaults = {
  scheme: "http",
  allow_basic_over_http: true,
  tls_mode: "insecure_skip_verify",
};

export function useUpdateCameraDefaults() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CameraDefaultsPatch) =>
      unwrap(api.PATCH("/api/v1/settings/camera-defaults", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useResetCameraDefaults() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.DELETE("/api/v1/settings/camera-defaults")),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
  });
}
