import { dismissToast, useToasts } from "./toasts";

const STYLES = {
  success: "border-emerald-700 bg-emerald-950 text-emerald-100",
  error: "border-red-700 bg-red-950 text-red-100",
  info: "border-slate-600 bg-slate-900 text-slate-100",
} as const;

export function Toaster() {
  const toasts = useToasts();
  return (
    <div aria-live="polite" className="fixed bottom-4 right-4 z-20 flex w-80 flex-col gap-2">
      {toasts.map((t) => (
        <div key={t.id} role="status" className={`flex items-start gap-2 rounded border p-3 text-sm shadow-lg ${STYLES[t.kind]}`}>
          <span className="flex-1">{t.message}</span>
          <button type="button" aria-label="Dismiss" className="opacity-70 hover:opacity-100" onClick={() => dismissToast(t.id)}>
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
