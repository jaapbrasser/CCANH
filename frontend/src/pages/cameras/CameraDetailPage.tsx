import { useState, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMe } from "../../api/auth";
import {
  useCamera,
  useDeleteCamera,
  useRefreshCamera,
  useSetStreamProfiles,
  useStatusHistory,
  type CameraDetail,
  type Capability,
  type CapabilityGroup,
} from "../../api/cameras";
import { ApiError, errorMessage } from "../../api/client";
import { groupTree, useGroups } from "../../api/organization";
import { LivePlayer } from "../../components/LivePlayer";
import { Chips, FavoriteButton } from "../../components/organization";
import { StatusBadge } from "../../components/StatusBadge";
import { showToast } from "../../components/toasts";
import { Alert, Button, Dialog } from "../../components/ui";
import { errorCategoryLabel, relativeTime } from "../../utils/format";
import { CameraFormDialog } from "./CameraFormDialog";
import { usePageTitle } from "../../components/pageTitle";

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

/** Live video with a profile picker (H.264 profiles only). */
function LiveView({ camera }: { camera: CameraDetail }) {
  const playable = camera.stream_profiles.filter((p) => p.codec == null || p.codec === "h264");
  const [profile, setProfile] = useState<string | undefined>(undefined);
  return (
    <section aria-label="Live view" className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-400">Live view</h2>
        {playable.length > 1 && (
          <select
            aria-label="Stream profile"
            className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm"
            value={profile ?? ""}
            onChange={(e) => setProfile(e.target.value || undefined)}
          >
            <option value="">Default for this camera</option>
            {playable.map((p) => (
              <option key={p.id || "default"} value={p.id}>
                {p.name}
                {p.width && p.height ? ` (${p.width}×${p.height})` : ""}
              </option>
            ))}
          </select>
        )}
      </div>
      {camera.stream_profiles.length === 0 ? (
        <div className="flex aspect-video items-center justify-center rounded bg-slate-900 text-sm text-slate-500">
          Live view becomes available once the camera's stream profiles are known.
        </div>
      ) : playable.length === 0 ? (
        <div className="flex aspect-video items-center justify-center rounded bg-slate-900 p-4 text-center text-sm text-slate-400">
          Live view needs an H.264 stream profile. This camera only offers{" "}
          {[...new Set(camera.stream_profiles.map((p) => p.codec?.toUpperCase()))].join(", ")}, which browsers can't play
          live here.
        </div>
      ) : (
        <LivePlayer cameraId={camera.id} profile={profile} purpose="detail" label={camera.name} />
      )}
    </section>
  );
}

/** The camera's groups (as full paths) and tags. */
function Organization({ camera }: { camera: CameraDetail }) {
  const groups = useGroups();
  const paths = new Map(groupTree(groups.data ?? []).map((g) => [g.id, g.path]));
  return (
    <dl className="grid grid-cols-[8rem_1fr] gap-y-2 text-sm">
      <dt className="text-slate-400">Groups</dt>
      <dd>
        <Chips
          items={camera.groups.map((g) => ({ id: g.id, name: paths.get(g.id) ?? g.name }))}
          label="Camera groups"
        />
      </dd>
      <dt className="text-slate-400">Tags</dt>
      <dd>
        <Chips items={camera.tags} label="Camera tags" />
      </dd>
    </dl>
  );
}

const AUTO = "__auto__";

function profileLabel(camera: CameraDetail, id: string | null | undefined): string {
  if (id == null) return "Automatic";
  return camera.stream_profiles.find((p) => p.id === id)?.name ?? `${id || "camera default"} (not on camera)`;
}

/** Which profiles grid tiles and the detail view use (SPEC §7.4). */
function ProfileChoices({ camera, isAdmin }: { camera: CameraDetail; isAdmin: boolean }) {
  const playable = camera.stream_profiles.filter((p) => p.codec == null || p.codec === "h264");
  const [grid, setGrid] = useState(camera.grid_profile ?? AUTO);
  const [detail, setDetail] = useState(camera.detail_profile ?? AUTO);
  const save = useSetStreamProfiles(camera.id);
  if (!isAdmin || playable.length === 0) {
    return (
      <p className="mt-3 text-sm text-slate-400">
        Grid tiles: {profileLabel(camera, camera.grid_profile)} · Detail view:{" "}
        {profileLabel(camera, camera.detail_profile)}
      </p>
    );
  }
  const select = (label: string, value: string, onChange: (v: string) => void) => (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-slate-300">{label}</span>
      <select
        className="rounded border border-slate-700 bg-slate-900 px-2 py-1"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value={AUTO}>Automatic (global default or by size)</option>
        {playable.map((p) => (
          <option key={p.id || "default"} value={p.id}>
            {p.name}
            {p.width && p.height ? ` (${p.width}×${p.height})` : ""}
          </option>
        ))}
      </select>
    </label>
  );
  return (
    <form
      aria-label="Profile choices"
      className="mt-4 flex flex-col gap-3 border-t border-slate-800 pt-4"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate(
          { grid_profile: grid === AUTO ? null : grid, detail_profile: detail === AUTO ? null : detail },
          { onSuccess: () => showToast("success", "Profile choices saved. New viewers use them.") },
        );
      }}
    >
      {save.isError && <Alert>{errorMessage(save.error)}</Alert>}
      <div className="grid gap-3 sm:grid-cols-2">
        {select("Grid tiles", grid, setGrid)}
        {select("Detail view", detail, setDetail)}
      </div>
      <div>
        <Button type="submit" variant="secondary" disabled={save.isPending}>
          Save profile choices
        </Button>
      </div>
    </form>
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
  usePageTitle("Camera", camera.data?.name);
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
          <FavoriteButton cameraId={camera.id} name={camera.name} />
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
      {s.identity_error_category && (
        <Alert>
          {errorCategoryLabel(s.identity_error_category)}: {s.identity_error_message ?? "the last device check failed."}
          {s.identity_error_category === "auth_failed" && isAdmin && " Check the camera's credentials under Edit."}
        </Alert>
      )}
      <div className="max-w-4xl">
        <LiveView camera={camera} />
      </div>
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
        <Section title="Groups and tags">
          <Organization camera={camera} />
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
              [
                "Credentials",
                camera.credential_set
                  ? `Shared set: ${camera.credential_set.name}`
                  : camera.has_credentials
                    ? "Camera-specific"
                    : "None",
              ],
              ...(camera.scheme === "http"
                ? ([["Basic over HTTP", camera.allow_basic_over_http ? "Allowed" : "Not allowed"]] as [string, string][])
                : []),
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
                  <tr key={p.id || "default"}>
                    <td className="py-0.5">{p.name}</td>
                    <td className="py-0.5 uppercase">{p.codec ?? "default"}</td>
                    <td className="py-0.5">{p.width && p.height ? `${p.width}×${p.height}` : "default"}</td>
                    <td className="py-0.5">{p.fps ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="text-sm text-slate-500">Not reported yet.</p>
          )}
          {camera.stream_profiles.length > 0 && (
            // Re-key on server values so a save or live update re-initializes the form.
            <ProfileChoices
              key={`${camera.grid_profile}|${camera.detail_profile}`}
              camera={camera}
              isAdmin={isAdmin}
            />
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
