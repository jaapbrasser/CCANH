import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components, operations } from "./schema";

export type Task = components["schemas"]["Task"];
export type TaskStatus = components["schemas"]["TaskStatus"];
export type TaskListQuery = NonNullable<operations["list_tasks"]["parameters"]["query"]>;

export const taskKeys = {
  all: ["tasks"] as const,
  lists: () => [...taskKeys.all, "list"] as const,
  list: (q: TaskListQuery) => [...taskKeys.lists(), q] as const,
  detail: (id: string) => [...taskKeys.all, "detail", id] as const,
};

export function isTerminal(status: TaskStatus | undefined): boolean {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

/**
 * Follow one task. Live WebSocket updates patch the cache; polling is a
 * fallback while the task is still running.
 */
export function useTask(id: string | null | undefined) {
  return useQuery({
    queryKey: taskKeys.detail(id ?? ""),
    queryFn: () => unwrap(api.GET("/api/v1/tasks/{id}", { params: { path: { id: id! } } })),
    enabled: Boolean(id),
    refetchInterval: (q) => (isTerminal(q.state.data?.status) ? false : 500),
  });
}

export function useTasks(query: TaskListQuery) {
  return useQuery({
    queryKey: taskKeys.list(query),
    queryFn: () => unwrap(api.GET("/api/v1/tasks", { params: { query } })),
    placeholderData: keepPreviousData,
  });
}

export function useCancelTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      unwrap(api.POST("/api/v1/tasks/{id}/cancel", { params: { path: { id } } })),
    onSuccess: (_, id) => {
      qc.invalidateQueries({ queryKey: taskKeys.detail(id) });
      qc.invalidateQueries({ queryKey: taskKeys.lists() });
    },
  });
}
