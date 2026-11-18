import type { CameraStatus } from "../api/cameras";
import { errorCategoryLabel } from "../utils/format";

const STYLES = {
  online: "bg-emerald-900/60 text-emerald-200",
  offline: "bg-red-900/60 text-red-200",
  unknown: "bg-slate-800 text-slate-300",
} as const;

const LABELS = { online: "Online", offline: "Offline", unknown: "Unknown" } as const;

export function StatusBadge({ status }: { status: CameraStatus }) {
  const reason = status.state !== "online" ? errorCategoryLabel(status.error_category) : null;
  return (
    <span className="inline-flex items-center gap-2">
      <span className={`rounded px-2 py-0.5 text-xs font-medium ${STYLES[status.state]}`}>
        {LABELS[status.state]}
      </span>
      {reason && <span className="text-xs text-slate-400">{reason}</span>}
    </span>
  );
}
