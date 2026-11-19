import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cameraKeys } from "./cameras";
import { api, unwrap } from "./client";
import type { components, operations } from "./schema";

export type Group = components["schemas"]["Group"];
export type Tag = components["schemas"]["Tag"];
export type GroupPatch = components["schemas"]["GroupPatch"];
export type DeleteMode = operations["delete_group"]["parameters"]["query"]["mode"];
export type BulkAction = components["schemas"]["BulkAction"];

export const orgKeys = {
  groups: ["groups"] as const,
  tags: ["tags"] as const,
  favorites: ["favorites"] as const,
};

export function useGroups() {
  return useQuery({ queryKey: orgKeys.groups, queryFn: () => unwrap(api.GET("/api/v1/groups")) });
}

export function useTags() {
  return useQuery({ queryKey: orgKeys.tags, queryFn: () => unwrap(api.GET("/api/v1/tags")) });
}

export function useFavorites() {
  return useQuery({ queryKey: orgKeys.favorites, queryFn: () => unwrap(api.GET("/api/v1/favorites")) });
}

/** A group with its depth and full path, in tree order. */
export interface GroupNode extends Group {
  depth: number;
  path: string;
}

/** Order groups as a tree (parents before children, siblings by name). */
export function groupTree(groups: Group[]): GroupNode[] {
  const children = new Map<string | null, Group[]>();
  for (const g of groups) {
    const key = g.parent_id ?? null;
    children.set(key, [...(children.get(key) ?? []), g]);
  }
  const out: GroupNode[] = [];
  const visit = (parent: string | null, depth: number, prefix: string) => {
    const list = [...(children.get(parent) ?? [])].sort((a, b) => a.name.localeCompare(b.name));
    for (const g of list) {
      const path = prefix ? `${prefix} / ${g.name}` : g.name;
      out.push({ ...g, depth, path });
      visit(g.id, depth + 1, path);
    }
  };
  visit(null, 0, "");
  return out;
}

/** Ids of a group and all its descendants. */
export function subtreeIds(groups: Group[], id: string): Set<string> {
  const ids = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const g of groups) {
      if (g.parent_id && ids.has(g.parent_id) && !ids.has(g.id)) {
        ids.add(g.id);
        grew = true;
      }
    }
  }
  return ids;
}

function useOrgMutation<TVars, TData>(fn: (v: TVars) => Promise<TData>, key: readonly string[]) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: key });
      qc.invalidateQueries({ queryKey: cameraKeys.all });
    },
  });
}

export function useCreateGroup() {
  return useOrgMutation(
    (body: { name: string; parent_id?: string | null }) => unwrap(api.POST("/api/v1/groups", { body })),
    orgKeys.groups,
  );
}

export function useUpdateGroup() {
  return useOrgMutation(
    ({ id, ...body }: GroupPatch & { id: string }) =>
      unwrap(api.PATCH("/api/v1/groups/{id}", { params: { path: { id } }, body })),
    orgKeys.groups,
  );
}

export function useDeleteGroup() {
  return useOrgMutation(
    ({ id, mode }: { id: string; mode: DeleteMode }) =>
      unwrap(api.DELETE("/api/v1/groups/{id}", { params: { path: { id }, query: { mode } } })),
    orgKeys.groups,
  );
}

export function useCreateTag() {
  return useOrgMutation((name: string) => unwrap(api.POST("/api/v1/tags", { body: { name } })), orgKeys.tags);
}

export function useRenameTag() {
  return useOrgMutation(
    ({ id, name }: { id: string; name: string }) =>
      unwrap(api.PATCH("/api/v1/tags/{id}", { params: { path: { id } }, body: { name } })),
    orgKeys.tags,
  );
}

export function useDeleteTag() {
  return useOrgMutation(
    (id: string) => unwrap(api.DELETE("/api/v1/tags/{id}", { params: { path: { id } } })),
    orgKeys.tags,
  );
}

/** Add or remove a favorite, updating the cached list right away. */
export function useToggleFavorite() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ cameraId, favorite }: { cameraId: string; favorite: boolean }) =>
      favorite
        ? unwrap(api.PUT("/api/v1/favorites/{camera_id}", { params: { path: { camera_id: cameraId } } }))
        : unwrap(api.DELETE("/api/v1/favorites/{camera_id}", { params: { path: { camera_id: cameraId } } })),
    onMutate: ({ cameraId, favorite }) => {
      const prev = qc.getQueryData<string[]>(orgKeys.favorites);
      qc.setQueryData<string[]>(orgKeys.favorites, (ids = []) =>
        favorite ? [cameraId, ...ids.filter((i) => i !== cameraId)] : ids.filter((i) => i !== cameraId),
      );
      return { prev };
    },
    onError: (_e, _v, ctx) => qc.setQueryData(orgKeys.favorites, ctx?.prev),
    onSettled: () => {
      qc.invalidateQueries({ queryKey: orgKeys.favorites });
      // The favorites filter depends on it.
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
    },
  });
}

export type BulkSettings = components["schemas"]["BulkSettings"];
export type BulkResult = components["schemas"]["BulkResult"];
/** Which cameras a bulk operation applies to: ids, a group and/or a tag (a union). */
export interface BulkSelection {
  camera_ids?: string[];
  group_id?: string;
  include_subgroups?: boolean;
  tag_id?: string;
}

/** Start a bulk operation; returns the task to follow. */
export function useBulkCameras() {
  return useMutation({
    mutationFn: (body: BulkSelection & { action: BulkAction }) =>
      unwrap(api.POST("/api/v1/cameras/bulk", { body })),
  });
}
