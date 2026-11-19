import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { cameraKeys } from "./cameras";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type DiscoveryJob = components["schemas"]["DiscoveryJob"];
export type DiscoveryJobDetail = components["schemas"]["DiscoveryJobDetail"];
export type DiscoveredDevice = components["schemas"]["DiscoveredDevice"];
export type DiscoveryPreview = components["schemas"]["DiscoveryPreview"];
export type RegisterDiscovered = components["schemas"]["RegisterDiscovered"];
export type RegisterOutcome = components["schemas"]["RegisterOutcome"];

export const discoveryKeys = {
  all: ["discovery"] as const,
  jobs: () => [...discoveryKeys.all, "jobs"] as const,
  job: (id: string) => [...discoveryKeys.all, "job", id] as const,
};

/** Split the ranges box into entries (the server splits further). */
export function rangeEntries(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Address count and caps for what's typed, debounced. */
export function useDiscoveryPreview(text: string) {
  const [debounced, setDebounced] = useState(text);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(text), 300);
    return () => clearTimeout(t);
  }, [text]);
  const ranges = rangeEntries(debounced);
  return useQuery({
    queryKey: [...discoveryKeys.all, "preview", ranges],
    queryFn: () => unwrap(api.POST("/api/v1/discovery/preview", { body: { ranges } })),
    enabled: ranges.length > 0,
    retry: false,
  });
}

export function useDiscoveryJobs() {
  return useQuery({ queryKey: discoveryKeys.jobs(), queryFn: () => unwrap(api.GET("/api/v1/discovery/jobs")) });
}

export function useDiscoveryJob(id: string | null) {
  return useQuery({
    queryKey: discoveryKeys.job(id ?? ""),
    queryFn: () => unwrap(api.GET("/api/v1/discovery/jobs/{id}", { params: { path: { id: id! } } })),
    enabled: Boolean(id),
  });
}

export function useStartDiscovery() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { ranges: string[]; confirm_large: boolean }) =>
      unwrap(api.POST("/api/v1/discovery/jobs", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: discoveryKeys.jobs() }),
  });
}

export function useRegisterDiscovered(jobId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: RegisterDiscovered) =>
      unwrap(api.POST("/api/v1/discovery/jobs/{id}/register", { params: { path: { id: jobId } }, body })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: discoveryKeys.job(jobId) });
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
    },
  });
}

/** Apply a `discovery_found` message: add the result to its job, once. */
export function applyFound(qc: QueryClient, jobId: string, result: DiscoveredDevice) {
  qc.setQueryData<DiscoveryJobDetail>(discoveryKeys.job(jobId), (job) =>
    job && !job.results.some((r) => r.id === result.id) ? { ...job, results: [...job.results, result] } : job,
  );
}

/** Apply a `discovery_progress` message to the job and the job list. */
export function applyProgress(
  qc: QueryClient,
  jobId: string,
  counters: { scanned: number; responsive: number; found: number },
) {
  qc.setQueryData<DiscoveryJobDetail>(discoveryKeys.job(jobId), (job) => job && { ...job, ...counters });
  qc.setQueryData<DiscoveryJob[]>(discoveryKeys.jobs(), (jobs) =>
    jobs?.map((j) => (j.id === jobId ? { ...j, ...counters } : j)),
  );
}
