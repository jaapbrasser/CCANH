import { useState, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMe } from "../../api/auth";
import {
  useCamera,
  useDeleteCamera,
  useRefreshCamera,
  useStatusHistory,
  type CameraDetail,
  type Capability,
  type CapabilityGroup,
} from "../../api/cameras";
import { ApiError, errorMessage } from "../../api/client";
import { StatusBadge } from "../../components/StatusBadge";
import { Alert, Button, Dialog } from "../../components/ui";
import { errorCategoryLabel, relativeTime } from "../../utils/format";
import { CameraFormDialog } from "./CameraFormDialog";

const GROUP_LABELS: Record<CapabilityGroup, string> = {
  device: "Device",
  network: "Network",
  video: "Video",
  audio: "Audio",
  ptz: "Pan / tilt / zoom",
  io: "I/O",
  events: "Events",
  storage: "Storage",
  analytics: "Analytics",
};

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section aria-label={title} className="rounded border border-slate-800 p-4">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-400">{title}</h2>
      {children}
    </section>
  );
}

function Facts({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[10rem_1fr] gap-x-4 gap-y-1.5 text-sm">
      {rows.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-slate-400">{k}</dt>
          <dd className="break-all">{v ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}

/** One section per capability group present, so new groups need no redesign. */
function CapabilitySections({ capabilities }: { capabilities: Capability[] }) {
  const groups = new Map<CapabilityGroup, Capability[]>();
  for (const c of capabilities) groups.set(c.group, [...(groups.get(c.group) ?? []), c]);
  return (
    <>
      {[...groups.entries()].map(([group, caps]) => (
        <Section key={group} title={GROUP_LABELS[group] ?? group}>
          <ul className="flex flex-col gap-1.5 text-sm">
            {caps.map((c) => (
              <li key={c.key} className="flex items-baseline gap-2">
                <span
                  aria-label={c.supported ? "Supported" : "Not supported"}
                  className={c.supported ? "text-emerald-400" : "text-slate-600"}
                >
                  {c.supported ? "✓" : "✕"}
                </span>
                <span className={c.supported ? "" : "text-slate-500"}>{c.label}</span>
                {c.version && <span className="text-xs text-slate-500">v{c.version}</span>}
                {!c.supported && c.detail && <span className="text-xs text-slate-600">· {c.detail}</span>}
              </li>
            ))}
          </ul>
        </Section>
      ))}
    </>
  );
}

function History({ id }: { id: string }) {
  const history = useStatusHistory(id);
  if (!history.data?.items.length) return <p className="text-sm text-slate-500">No status changes yet.</p>;
  return (
    <ol className="flex flex-col gap-1.5 text-sm">
      {history.data.items.map((t) => (
        <li key={t.id} className="flex gap-3">
          <span className="w-44 shrink-0 text-slate-500">{new Date(t.occurred_at).toLocaleString()}</span>
          <span>
            {t.from_state} → <strong>{t.to_state}</strong>
            {t.error_category && <span className="text-slate-400"> ({errorCategoryLabel(t.error_category)})</span>}
          </span>
        </li>
      ))}
    </ol>
  );
}

export function CameraDetailPage() {
  const { id = "" } = useParams();
  const camera = useCamera(id);
  const { data: me } = useMe();
  const isAdmin = me?.role === "admin";

  if (camera.isPending) return <p className="text-slate-400">Loading…</p>;
  if (camera.isError) {
    const missing = camera.error instanceof ApiError && camera.error.status === 404;
    return (
      <div>
        <Link to="/cameras" className="text-sm text-slate-400 hover:text-white">
          ← Cameras
        </Link>
        <h1 className="mt-4 text-xl font-semibold">{missing ? "Camera not found" : "Couldn't load camera"}</h1>
        {!missing && <Alert>{errorMessage(camera.error)}</Alert>}
      </div>
    );
  }
  return <Detail camera={camera.data} isAdmin={isAdmin} />;
}

function Detail({ camera, isAdmin }: { camera: CameraDetail; isAdmin: boolean }) {
  const navigate = useNavigate();
  const refresh = useRefreshCamera(camera.id);
  const remove = useDeleteCamera();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const s = camera.status;
  const net = camera.network;

  return (
    <div className="flex flex-col gap-4">
      <Link to="/cameras" className="text-sm text-slate-400 hover:text-white">
        ← Cameras
      </Link>
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <h1 className="text-2xl font-semibold">{camera.name}</h1>
          <StatusBadge status={s} />
        </div>
        {isAdmin && (
          <div className="flex gap-2">
            <Button variant="secondary" disabled={refresh.isPending} onClick={() => refresh.mutate()}>
              {refresh.isPending ? "Refreshing…" : "Refresh details"}
            </Button>
            <Button variant="secondary" onClick={() => setEditing(true)}>
              Edit
            </Button>
            <Button variant="danger" onClick={() => setDeleting(true)}>
              Remove
            </Button>
          </div>
        )}
      </div>
      {refresh.isError && <Alert>{errorMessage(refresh.error)}</Alert>}
      {!camera.identity_updated_at && (
        <p className="text-sm text-slate-400">
          Device details appear after the first successful contact with the camera.
        </p>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <Section title="Connectivity">
          <Facts
            rows={[
              ["Status", <StatusBadge key="s" status={s} />],
              ["Last contact", relativeTime(s.last_success_at)],
              ["Last failure", relativeTime(s.last_failure_at)],
              ["Last change", relativeTime(s.last_change_at)],
              ["Response time", s.response_time_ms != null ? `${s.response_time_ms} ms` : "—"],
              ["Failed checks", String(s.consecutive_failures)],
              ["Last error", s.error_message ?? "—"],
              ["Monitoring", camera.monitoring_enabled ? "On" : "Off"],
            ]}
          />
        </Section>
        <Section title="Identity">
          <Facts
            rows={[
              ["Manufacturer", camera.manufacturer],
              ["Model", camera.model],
              ["Firmware", camera.firmware_version],
              ["Serial number", camera.serial_number],
              ["MAC address", camera.mac_address],
              ["Device name", camera.device_name],
              ["Hostname", camera.device_hostname],
              ["Details updated", relativeTime(camera.identity_updated_at)],
            ]}
          />
        </Section>
        <Section title="Connection">
          <Facts
            rows={[
              ["Address", <span key="a" className="font-mono">{`${camera.scheme}://${camera.host}:${camera.port}`}</span>],
              [
                "Certificate check",
                camera.tls_mode === "insecure_skip_verify" ? (
                  <span key="t" className="text-amber-300">Not verified (insecure)</span>
                ) : camera.tls_mode === "pinned" ? (
                  <span key="t">Pinned · <span className="font-mono text-xs">{camera.tls_pinned_sha256}</span></span>
                ) : (
                  "Strict"
                ),
              ],
              ["IPv4", net?.ipv4_address],
              ["Subnet mask", net?.subnet_mask],
              ["Gateway", net?.gateway],
              ["DHCP", net?.dhcp == null ? "—" : net.dhcp ? "Yes" : "No"],
            ]}
          />
        </Section>
        <Section title="Stream profiles">
          {camera.stream_profiles.length ? (
            <table className="w-full text-left text-sm">
              <thead className="text-slate-400">
                <tr>
                  <th className="pb-1 font-medium">Profile</th>
                  <th className="pb-1 font-medium">Codec</th>
                  <th className="pb-1 font-medium">Resolution</th>
                  <th className="pb-1 font-medium">FPS</th>
                </tr>
              </thead>
              <tbody>
                {camera.stream_profiles.map((p) => (
                  <tr key={p.id}>
                    <td className="py-0.5">{p.name}</td>
                    <td className="py-0.5 uppercase">{p.codec}</td>
                    <td className="py-0.5">{`${p.width}×${p.height}`}</td>
                    <td className="py-0.5">{p.fps ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="text-sm text-slate-500">Not reported yet.</p>
          )}
        </Section>
        <CapabilitySections capabilities={camera.capabilities} />
        <Section title="Status history">
          <History id={camera.id} />
        </Section>
      </div>

      {editing && (
        <CameraFormDialog camera={camera} onClose={() => setEditing(false)} onSaved={() => setEditing(false)} />
      )}
      {deleting && (
        <Dialog title={`Remove ${camera.name}?`} onClose={() => setDeleting(false)}>
          <p className="mb-4 text-sm text-slate-300">
            This removes the camera, its status and its history. It can't be undone.
          </p>
          {remove.isError && (
            <div className="mb-4">
              <Alert>{errorMessage(remove.error)}</Alert>
            </div>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setDeleting(false)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              disabled={remove.isPending}
              onClick={() => remove.mutate(camera.id, { onSuccess: () => navigate("/cameras") })}
            >
              Remove camera
            </Button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
