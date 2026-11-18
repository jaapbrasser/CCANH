import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components, operations } from "./schema";

export type CameraSummary = components["schemas"]["CameraSummary"];
export type CameraDetail = components["schemas"]["CameraDetail"];
export type CameraStatus = components["schemas"]["CameraStatusView"];
export type CameraState = components["schemas"]["CameraState"];
export type Capability = components["schemas"]["Capability"];
export type CapabilityGroup = components["schemas"]["CapabilityGroup"];
export type NewCamera = components["schemas"]["NewCamera"];
export type CameraPatch = components["schemas"]["CameraPatch"];
export type CameraListQuery = NonNullable<operations["list_cameras"]["parameters"]["query"]>;
export type CameraList = components["schemas"]["CameraList"];

/** Query keys. Live updates patch these caches (see live.ts). */
export const cameraKeys = {
  all: ["cameras"] as const,
  lists: () => [...cameraKeys.all, "list"] as const,
  list: (q: CameraListQuery) => [...cameraKeys.lists(), q] as const,
  detail: (id: string) => [...cameraKeys.all, "detail", id] as const,
  history: (id: string) => [...cameraKeys.all, "history", id] as const,
};

export function useCameras(query: CameraListQuery) {
  return useQuery({
    queryKey: cameraKeys.list(query),
    queryFn: () => unwrap(api.GET("/api/v1/cameras", { params: { query } })),
    placeholderData: keepPreviousData,
  });
}

export function useCamera(id: string) {
  return useQuery({
    queryKey: cameraKeys.detail(id),
    queryFn: () => unwrap(api.GET("/api/v1/cameras/{id}", { params: { path: { id } } })),
  });
}

export function useStatusHistory(id: string) {
  return useQuery({
    queryKey: cameraKeys.history(id),
    queryFn: () =>
      unwrap(
        api.GET("/api/v1/cameras/{id}/status-history", {
          params: { path: { id }, query: { page_size: 20 } },
        }),
      ),
  });
}

export function useCreateCamera() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: NewCamera) => unwrap(api.POST("/api/v1/cameras", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: cameraKeys.lists() }),
  });
}

export function useUpdateCamera(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CameraPatch) =>
      unwrap(api.PATCH("/api/v1/cameras/{id}", { params: { path: { id } }, body })),
    onSuccess: (camera) => {
      qc.setQueryData(cameraKeys.detail(id), camera);
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
    },
  });
}

export type StreamProfileOverrides = components["schemas"]["StreamProfileOverrides"];

/** Set which profiles this camera uses for grid tiles and the detail view. */
export function useSetStreamProfiles(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: StreamProfileOverrides) =>
      unwrap(api.PUT("/api/v1/cameras/{id}/stream-profiles", { params: { path: { id } }, body })),
    onSuccess: (camera) => qc.setQueryData(cameraKeys.detail(id), camera),
  });
}

export function useDeleteCamera() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      unwrap(api.DELETE("/api/v1/cameras/{id}", { params: { path: { id } } })),
    onSuccess: (_, id) => {
      qc.removeQueries({ queryKey: cameraKeys.detail(id) });
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
    },
  });
}

export function useRefreshCamera(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () =>
      unwrap(api.POST("/api/v1/cameras/{id}/refresh", { params: { path: { id } } })),
    onSuccess: (camera) => qc.setQueryData(cameraKeys.detail(id), camera),
  });
}
