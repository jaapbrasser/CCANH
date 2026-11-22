import { useEffect, type ReactNode } from "react";
import { PtzPad } from "../../components/PtzPad";
import { moveCamera } from "../../api/ptz";
import { Link } from "react-router";
import { useCamera } from "../../api/cameras";
import { LivePlayer } from "../../components/LivePlayer";
import { StatusBadge } from "../../components/StatusBadge";
import { Button } from "../../components/ui";
import { relativeTime } from "../../utils/format";

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-slate-400">{label}</dt>
      <dd className="min-w-0 break-words">{children ?? "—"}</dd>
    </>
  );
}

/**
 * A tile expanded over the dashboard (SPEC §8.4): the detail-profile stream
 * plus what's known about the camera. Back button or Esc returns.
 */
export function ExpandedCamera({ cameraId, onClose }: { cameraId: string; onClose: () => void }) {
  const camera = useCamera(cameraId);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const c = camera.data;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={c ? c.name : "Camera"}
      className="fixed inset-0 z-20 flex flex-col bg-slate-950/95 p-4"
    >
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <Button variant="secondary" onClick={onClose}>
          ← Back to dashboard
        </Button>
        <span className="text-xs text-slate-500">or press Esc</span>
        {c && (
          <>
            <h2 className="text-xl font-semibold">{c.name}</h2>
            <StatusBadge status={c.status} />
            <Link to={`/cameras/${c.id}`} className="ml-auto text-sm text-sky-300 hover:underline">
              Open camera page
            </Link>
          </>
        )}
      </div>
      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[1fr_20rem]">
        <div className="flex min-h-0 items-center">
          <LivePlayer cameraId={cameraId} purpose="detail" label={c?.name ?? "camera"} />
        </div>
        {c && (
          <aside aria-label="Camera details" className="overflow-y-auto rounded border border-slate-800 p-4 text-sm">
            {c.ptz && (
              <div className="mb-4">
                <PtzPad title="Moves this camera" send={(mv) => moveCamera(c.id, mv)} />
              </div>
            )}
            <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1.5">
              <Fact label="Address">
                {c.host}:{c.port}
              </Fact>
              <Fact label="Model">{c.model}</Fact>
              <Fact label="Firmware">{c.firmware_version}</Fact>
              <Fact label="Serial">{c.serial_number}</Fact>
              <Fact label="MAC">{c.mac_address}</Fact>
              <Fact label="Last contact">{relativeTime(c.status.last_success_at)}</Fact>
              <Fact label="Response">
                {c.status.response_time_ms != null ? `${c.status.response_time_ms} ms` : null}
              </Fact>
              <Fact label="Groups">{c.groups.map((g) => g.name).join(", ") || null}</Fact>
            </dl>
            <h3 className="mb-1 mt-4 text-xs font-semibold uppercase tracking-wide text-slate-400">Capabilities</h3>
            <ul className="flex flex-wrap gap-1">
              {c.capabilities
                .filter((cap) => cap.supported)
                .map((cap) => (
                  <li key={cap.key} className="rounded bg-slate-800 px-1.5 py-0.5 text-xs">
                    {cap.label}
                  </li>
                ))}
            </ul>
          </aside>
        )}
      </div>
    </div>
  );
}
