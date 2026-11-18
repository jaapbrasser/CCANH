import { useSyncExternalStore } from "react";

export interface Toast {
  id: number;
  kind: "success" | "error" | "info";
  message: string;
}

let toasts: Toast[] = [];
let nextId = 1;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function dismissToast(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

/** Show a toast for a few seconds. */
export function showToast(kind: Toast["kind"], message: string, ms = 6000) {
  const id = nextId++;
  toasts = [...toasts.slice(-4), { id, kind, message }];
  emit();
  setTimeout(() => dismissToast(id), ms);
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => toasts,
  );
}
