import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, unwrap } from "./client";
import type { components } from "./schema";

export type Dashboard = components["schemas"]["Dashboard"];
export type Tile = components["schemas"]["Tile"];
export type DashboardPatch = components["schemas"]["DashboardPatch"];

export const dashboardKeys = { all: ["dashboards"] as const };

export function useDashboards() {
  return useQuery({ queryKey: dashboardKeys.all, queryFn: () => unwrap(api.GET("/api/v1/dashboards")) });
}

export function useCreateDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { name: string; rows?: number; cols?: number }) =>
      unwrap(api.POST("/api/v1/dashboards", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: dashboardKeys.all }),
  });
}

function patchDashboard(id: string, body: DashboardPatch) {
  return unwrap(api.PATCH("/api/v1/dashboards/{id}", { params: { path: { id } }, body }));
}

/** Rename or make default (not the layout: see `useLayoutAutosave`). */
export function useUpdateDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: DashboardPatch & { id: string }) => patchDashboard(id, body),
    onSuccess: () => qc.invalidateQueries({ queryKey: dashboardKeys.all }),
  });
}

export function useDeleteDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE("/api/v1/dashboards/{id}", { params: { path: { id } } })),
    onSuccess: () => qc.invalidateQueries({ queryKey: dashboardKeys.all }),
  });
}

export interface Layout {
  rows: number;
  cols: number;
  tiles: Tile[];
}

function pickLayout(d: Layout): Layout {
  return { rows: d.rows, cols: d.cols, tiles: d.tiles };
}

/** Debounce for layout saves. */
export const SAVE_DELAY_MS = 800;

/**
 * Local, optimistic layout state for one dashboard. Changes apply at once and
 * save after a short pause; a failed save rolls back to the last saved layout
 * and calls `onError`.
 */
export function useLayoutAutosave(dashboard: Dashboard, onError: (e: unknown) => void) {
  const qc = useQueryClient();
  const saved = useRef<Layout>(pickLayout(dashboard));
  const [layout, setLayout] = useState<Layout>(() => pickLayout(dashboard));
  const [saving, setSaving] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pending = useRef<Layout | null>(null);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });

  const flush = useCallback(async () => {
    const next = pending.current;
    pending.current = null;
    if (!next) return;
    setSaving(true);
    try {
      const result = await patchDashboard(dashboard.id, pickLayout(next));
      saved.current = pickLayout(result);
      qc.setQueryData<Dashboard[]>(dashboardKeys.all, (list) => list?.map((d) => (d.id === result.id ? result : d)));
    } catch (e) {
      // Roll back unless a newer change is already waiting to be saved.
      if (!pending.current) setLayout(saved.current);
      onErrorRef.current(e);
    } finally {
      setSaving(false);
    }
  }, [dashboard.id, qc]);

  const change = useCallback(
    (next: Layout) => {
      setLayout(next);
      pending.current = next;
      clearTimeout(timer.current);
      timer.current = setTimeout(() => void flush(), SAVE_DELAY_MS);
    },
    [flush],
  );

  // Save what's pending when leaving the dashboard.
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      void flush();
    },
    [flush],
  );

  return { layout, change, saving };
}

/** Grid presets (SPEC §8.2). */
export const PRESETS: { label: string; rows: number; cols: number }[] = [
  { label: "1 (1×1)", rows: 1, cols: 1 },
  { label: "2 (1×2)", rows: 1, cols: 2 },
  { label: "4 (2×2)", rows: 2, cols: 2 },
  { label: "6 (2×3)", rows: 2, cols: 3 },
  { label: "9 (3×3)", rows: 3, cols: 3 },
  { label: "12 (3×4)", rows: 3, cols: 4 },
  { label: "16 (4×4)", rows: 4, cols: 4 },
];

/** Move the camera at `from` to `to`, swapping with whatever is there. */
export function moveTile(tiles: Tile[], from: number, to: number): Tile[] {
  if (from === to) return tiles;
  return tiles
    .map((t) => (t.position === from ? { ...t, position: to } : t.position === to ? { ...t, position: from } : t))
    .sort((a, b) => a.position - b.position);
}

/**
 * Fit tiles into a rows × cols grid: tiles that still fit keep their place,
 * the rest move to free cells. Returns `null` if there are more cameras than
 * cells.
 */
export function fitTiles(tiles: Tile[], rows: number, cols: number): Tile[] | null {
  const cells = rows * cols;
  if (tiles.length > cells) return null;
  const kept = tiles.filter((t) => t.position < cells);
  const used = new Set(kept.map((t) => t.position));
  const free = Array.from({ length: cells }, (_, i) => i).filter((i) => !used.has(i));
  const moved = tiles.filter((t) => t.position >= cells).map((t, i) => ({ ...t, position: free[i]! }));
  return [...kept, ...moved].sort((a, b) => a.position - b.position);
}
