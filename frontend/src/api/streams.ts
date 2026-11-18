import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type StreamInfo = components["schemas"]["StreamInfo"];

export const streamsKey = ["streams"] as const;

export function useStreams() {
  return useQuery({
    queryKey: streamsKey,
    queryFn: () => unwrap(api.GET("/api/v1/streams")),
  });
}

export function useStopStream() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ cameraId, profile }: { cameraId: string; profile: string }) =>
      unwrap(
        api.DELETE("/api/v1/streams/{camera_id}", {
          params: { path: { camera_id: cameraId }, query: { profile } },
        }),
      ),
    onSuccess: () => qc.invalidateQueries({ queryKey: streamsKey }),
  });
}
