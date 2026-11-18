import { useQuery } from "@tanstack/react-query";
import { api } from "./client";
import type { components } from "./schema";

export type HealthResponse = components["schemas"]["HealthResponse"];

/** Polls `/api/v1/health`. A 503 still carries a body, so it is returned, not thrown. */
export function useHealth() {
  return useQuery({
    queryKey: ["health"],
    queryFn: async (): Promise<HealthResponse> => {
      const { data, error, response } = await api.GET("/api/v1/health");
      const body = data ?? (error as HealthResponse | undefined);
      if (!body) throw new Error(`Health check failed (HTTP ${response.status})`);
      return body;
    },
    refetchInterval: 15_000,
  });
}
