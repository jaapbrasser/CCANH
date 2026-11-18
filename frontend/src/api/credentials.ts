import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type CredentialSet = components["schemas"]["CredentialSetSummary"];
export type CredentialInput = components["schemas"]["CredentialInput"];
export type TestConnectionRequest = components["schemas"]["TestConnectionRequest"];
export type TestConnectionResult = components["schemas"]["TestConnectionResult"];

const setsKey = ["credential-sets"] as const;

export function useCredentialSets(enabled = true) {
  return useQuery({
    queryKey: setsKey,
    queryFn: () => unwrap(api.GET("/api/v1/credential-sets")),
    enabled,
  });
}

export function useCreateCredentialSet() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: components["schemas"]["NewCredentialSet"]) =>
      unwrap(api.POST("/api/v1/credential-sets", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: setsKey }),
  });
}

export function useUpdateCredentialSet() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & components["schemas"]["CredentialSetPatch"]) =>
      unwrap(api.PATCH("/api/v1/credential-sets/{id}", { params: { path: { id } }, body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: setsKey }),
  });
}

export function useDeleteCredentialSet() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      unwrap(api.DELETE("/api/v1/credential-sets/{id}", { params: { path: { id } } })),
    onSuccess: () => qc.invalidateQueries({ queryKey: setsKey }),
  });
}

export function useTestConnection() {
  return useMutation({
    mutationFn: (body: TestConnectionRequest) =>
      unwrap(api.POST("/api/v1/cameras/test-connection", { body })),
  });
}
