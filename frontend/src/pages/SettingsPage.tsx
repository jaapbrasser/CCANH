import { useState, type FormEvent } from "react";
import { errorMessage } from "../api/client";
import {
  useResetCameraDefaults,
  useUpdateCameraDefaults,
  type CameraDefaults,
  useResetRetention,
  useUpdateRetention,
  type RetentionSettings,
  useResetDiscovery,
  useUpdateDiscovery,
  type DiscoverySettings,
  useResetMonitoring,
  useResetStreaming,
  useSettings,
  useUpdateMonitoring,
  useUpdateStreaming,
  type MonitoringSettings,
  type StreamingSettings,
} from "../api/settings";
import { showToast } from "../components/toasts";
import { Alert, Button, Field, PageHeader } from "../components/ui";
import { usePageTitle } from "../components/pageTitle";

type Key = keyof MonitoringSettings;

const FIELDS: { key: Key; label: string; unit: string; hint: string }[] = [
  { key: "interval_secs", label: "Check interval", unit: "seconds", hint: "How often each monitored camera is checked (2–3600)." },
  { key: "jitter_percent", label: "Jitter", unit: "%", hint: "Randomizes each interval so cameras aren't checked in lockstep (0–50)." },
  { key: "failure_threshold", label: "Failures before offline", unit: "checks", hint: "Consecutive failed checks before a camera is shown offline (1–20)." },
  { key: "timeout_secs", label: "Request timeout", unit: "seconds", hint: "Per camera request (1–60)." },
  { key: "retries", label: "Retries per check", unit: "attempts", hint: "Extra attempts for transient errors. Never used for authentication failures (0–5)." },
  { key: "offline_max_interval_secs", label: "Offline check interval (max)", unit: "seconds", hint: "Offline cameras are checked less often, backing off up to this (≥ interval, ≤ 3600)." },
  { key: "max_concurrent", label: "Concurrent checks", unit: "checks", hint: "Limit on simultaneous camera requests (1–256)." },
  { key: "identity_refresh_minutes", label: "Credential re-check", unit: "minutes", hint: "How often online cameras' identity and credentials are re-verified (1–1440)." },
];

export function SettingsPage() {
  usePageTitle("General settings");
  const settings = useSettings();
  const current = settings.data?.current.monitoring;
  const stats = settings.data?.monitor;

  return (
    <div className="max-w-3xl">
      <PageHeader title="General" />
      {settings.isError && <Alert>{errorMessage(settings.error)}</Alert>}

      {stats && (
        <section aria-label="Monitor status" className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
          {[
            ["Cameras monitored", stats.cameras_monitored],
            ["Checks in flight", stats.checks_in_flight],
            ["Checks", stats.checks_total],
            ["Failed checks", stats.failures_total],
          ].map(([label, value]) => (
            <div key={label} className="rounded border border-slate-800 p-3">
              <p className="text-xs text-slate-400">{label}</p>
              <p className="text-xl font-semibold">{value}</p>
            </div>
          ))}
        </section>
      )}

      <h2 className="mb-1 text-lg font-semibold">Camera monitoring</h2>
      <p className="mb-4 text-sm text-slate-400">
        Changes apply within a second, without a restart, and are recorded in the audit log.
      </p>
      {current && settings.data && (
        // Re-key on server values so a save or reset re-initializes the form.
        <MonitoringForm
          key={JSON.stringify(current)}
          current={current}
          defaults={settings.data.defaults.monitoring}
        />
      )}

      <h2 className="mb-1 mt-10 text-lg font-semibold">Live video</h2>
      <p className="mb-4 text-sm text-slate-400">Video is buffered in memory only; nothing is recorded.</p>
      {settings.data && (
        <StreamingForm
          key={JSON.stringify(settings.data.current.streaming)}
          current={settings.data.current.streaming}
          defaults={settings.data.defaults.streaming}
        />
      )}

      <h2 className="mb-1 mt-10 text-lg font-semibold">Discovery</h2>
      <p className="mb-4 text-sm text-slate-400">Limits for scanning address ranges for cameras.</p>
      {settings.data && (
        <DiscoveryForm
          key={JSON.stringify(settings.data.current.discovery)}
          current={settings.data.current.discovery}
          defaults={settings.data.defaults.discovery}
        />
      )}

      <h2 className="mb-1 mt-10 text-lg font-semibold">New camera defaults</h2>
      <p className="mb-4 text-sm text-slate-400">
        What a new camera gets unless you choose otherwise when adding or registering it. Changing these doesn't
        affect cameras you already have.
      </p>
      {settings.data && (
        <CameraDefaultsForm
          key={JSON.stringify(settings.data.current.camera_defaults)}
          current={settings.data.current.camera_defaults}
        />
      )}

      <h2 className="mb-1 mt-10 text-lg font-semibold">History retention</h2>
      <p className="mb-4 text-sm text-slate-400">Older history is removed by a daily cleanup.</p>
      {settings.data && (
        <RetentionForm
          key={JSON.stringify(settings.data.current.retention)}
          current={settings.data.current.retention}
          defaults={settings.data.defaults.retention}
        />
      )}
    </div>
  );
}

function toForm(m: MonitoringSettings): Record<Key, string> {
  return Object.fromEntries(FIELDS.map((f) => [f.key, String(m[f.key])])) as Record<Key, string>;
}

function MonitoringForm({ current, defaults }: { current: MonitoringSettings; defaults: MonitoringSettings }) {
  const update = useUpdateMonitoring();
  const reset = useResetMonitoring();
  const [form, setForm] = useState(() => toForm(current));

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const patch: Partial<Record<Key, number>> = {};
    for (const f of FIELDS) {
      const v = Number(form[f.key]);
      if (v !== current[f.key]) patch[f.key] = v;
    }
    update.mutate(patch, { onSuccess: () => showToast("success", "Settings saved and applied.") });
  };

  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-4">
      {(update.isError || reset.isError) && <Alert>{errorMessage(update.error ?? reset.error)}</Alert>}
      <div className="grid gap-4 sm:grid-cols-2">
        {FIELDS.map((f) => (
          <Field
            key={f.key}
            label={`${f.label} (${f.unit})`}
            inputMode="numeric"
            hint={`${f.hint}${defaults[f.key] !== Number(form[f.key]) ? ` Default: ${defaults[f.key]}.` : ""}`}
            value={form[f.key]}
            onChange={(e) => setForm({ ...form, [f.key]: e.target.value.replace(/\D/g, "") })}
          />
        ))}
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={update.isPending}>
          Save
        </Button>
        <Button
          variant="secondary"
          disabled={reset.isPending}
          onClick={() => reset.mutate(undefined, { onSuccess: () => showToast("success", "Settings reset to defaults.") })}
        >
          Reset to defaults
        </Button>
      </div>
    </form>
  );
}

type SKey =
  | "grace_period_secs"
  | "stream_buffer_mb"
  | "global_buffer_mb"
  | "max_upstreams"
  | "detail_reserve"
  | "max_inbound_mbps";
type NameKey = "default_grid_profile" | "default_detail_profile";
const NAME_FIELDS: { key: NameKey; label: string; hint: string }[] = [
  {
    key: "default_grid_profile",
    label: "Grid profile name",
    hint: "Used for grid tiles on cameras that have a profile with this name. Empty: the smallest H.264 profile.",
  },
  {
    key: "default_detail_profile",
    label: "Detail profile name",
    hint: "Used for the detail view on cameras that have it. Empty: the largest H.264 profile.",
  },
];
const STREAM_FIELDS: { key: SKey; label: string; hint: string }[] = [
  { key: "grace_period_secs", label: "Grace period (seconds)", hint: "How long a camera stream stays connected after its last viewer leaves (0–300)." },
  { key: "stream_buffer_mb", label: "Buffer per stream (MB)", hint: "Memory for the latest keyframe group, so new viewers start instantly (1–64)." },
  { key: "global_buffer_mb", label: "Total buffer (MB)", hint: "Memory cap across all streams (16–4096)." },
  {
    key: "max_upstreams",
    label: "Camera connection limit",
    hint: "Most camera streams open at once; 0 means no limit (0–1000).",
  },
  {
    key: "detail_reserve",
    label: "Kept for detail views",
    hint: "With a limit set, this many connections are only for detail views, never grid tiles (0–100).",
  },
  {
    key: "max_inbound_mbps",
    label: "Inbound video limit (Mbit/s)",
    hint: "No new camera streams while video from cameras is above this; 0 means no limit.",
  },
];

function StreamingForm({ current, defaults }: { current: StreamingSettings; defaults: StreamingSettings }) {
  const update = useUpdateStreaming();
  const reset = useResetStreaming();
  const [form, setForm] = useState(
    () => Object.fromEntries(STREAM_FIELDS.map((f) => [f.key, String(current[f.key])])) as Record<SKey, string>,
  );
  const [names, setNames] = useState<Record<NameKey, string>>(() => ({
    default_grid_profile: current.default_grid_profile ?? "",
    default_detail_profile: current.default_detail_profile ?? "",
  }));
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const patch: Partial<Record<SKey, number> & Record<NameKey, string>> = {};
    for (const f of STREAM_FIELDS) {
      const v = Number(form[f.key]);
      if (v !== current[f.key]) patch[f.key] = v;
    }
    for (const f of NAME_FIELDS) {
      // "" clears the name on the server.
      if (names[f.key].trim() !== (current[f.key] ?? "")) patch[f.key] = names[f.key].trim();
    }
    update.mutate(patch, { onSuccess: () => showToast("success", "Live video settings saved.") });
  };
  return (
    <form onSubmit={onSubmit} aria-label="Live video settings" className="flex flex-col gap-4">
      {(update.isError || reset.isError) && <Alert>{errorMessage(update.error ?? reset.error)}</Alert>}
      <div className="grid gap-4 sm:grid-cols-3">
        {STREAM_FIELDS.map((f) => (
          <Field
            key={f.key}
            label={f.label}
            inputMode="numeric"
            hint={`${f.hint}${defaults[f.key] !== Number(form[f.key]) ? ` Default: ${defaults[f.key]}.` : ""}`}
            value={form[f.key]}
            onChange={(e) => setForm({ ...form, [f.key]: e.target.value.replace(/\D/g, "") })}
          />
        ))}
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        {NAME_FIELDS.map((f) => (
          <Field
            key={f.key}
            label={f.label}
            maxLength={64}
            hint={`${f.hint} Cameras can override this on their page.`}
            value={names[f.key]}
            onChange={(e) => setNames({ ...names, [f.key]: e.target.value })}
          />
        ))}
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={update.isPending}>
          Save live video settings
        </Button>
        <Button
          variant="secondary"
          disabled={reset.isPending}
          onClick={() => reset.mutate(undefined, { onSuccess: () => showToast("success", "Live video settings reset.") })}
        >
          Reset
        </Button>
      </div>
    </form>
  );
}

type DKey = keyof DiscoverySettings;
const DISCOVERY_FIELDS: { key: DKey; label: string; hint: string }[] = [
  { key: "max_addresses", label: "Address limit per scan", hint: "Larger scans need explicit confirmation (16–65536)." },
  { key: "concurrency", label: "Hosts at once", hint: "Addresses probed in parallel (1–256)." },
  { key: "connect_timeout_ms", label: "Port check timeout (ms)", hint: "Per TCP connection attempt (100–10000)." },
  { key: "http_timeout_ms", label: "Identification timeout (ms)", hint: "For the one request to a responding host (500–30000)." },
];

function DiscoveryForm({ current, defaults }: { current: DiscoverySettings; defaults: DiscoverySettings }) {
  const update = useUpdateDiscovery();
  const reset = useResetDiscovery();
  const [form, setForm] = useState(
    () => Object.fromEntries(DISCOVERY_FIELDS.map((f) => [f.key, String(current[f.key])])) as Record<DKey, string>,
  );
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const patch: Partial<Record<DKey, number>> = {};
    for (const f of DISCOVERY_FIELDS) {
      const v = Number(form[f.key]);
      if (v !== current[f.key]) patch[f.key] = v;
    }
    update.mutate(patch, { onSuccess: () => showToast("success", "Discovery settings saved.") });
  };
  return (
    <form onSubmit={onSubmit} aria-label="Discovery settings" className="flex flex-col gap-4">
      {(update.isError || reset.isError) && <Alert>{errorMessage(update.error ?? reset.error)}</Alert>}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {DISCOVERY_FIELDS.map((f) => (
          <Field
            key={f.key}
            label={f.label}
            inputMode="numeric"
            hint={`${f.hint}${defaults[f.key] !== Number(form[f.key]) ? ` Default: ${defaults[f.key]}.` : ""}`}
            value={form[f.key]}
            onChange={(e) => setForm({ ...form, [f.key]: e.target.value.replace(/\D/g, "") })}
          />
        ))}
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={update.isPending}>
          Save discovery settings
        </Button>
        <Button
          variant="secondary"
          disabled={reset.isPending}
          onClick={() => reset.mutate(undefined, { onSuccess: () => showToast("success", "Discovery settings reset.") })}
        >
          Reset
        </Button>
      </div>
    </form>
  );
}

type RKey = keyof RetentionSettings;
const RETENTION_FIELDS: { key: RKey; label: string; hint: string }[] = [
  { key: "status_history_days", label: "Status history (days)", hint: "Camera online/offline changes (1–3650)." },
  { key: "tasks_days", label: "Tasks and scans (days)", hint: "Finished tasks, including discovery scans and their results (1–3650)." },
  { key: "audit_days", label: "Audit log (days)", hint: "0 keeps it forever; otherwise at least 90. Removing entries is itself audited." },
];

function RetentionForm({ current, defaults }: { current: RetentionSettings; defaults: RetentionSettings }) {
  const update = useUpdateRetention();
  const reset = useResetRetention();
  const [form, setForm] = useState(
    () => Object.fromEntries(RETENTION_FIELDS.map((f) => [f.key, String(current[f.key])])) as Record<RKey, string>,
  );
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const patch: Partial<Record<RKey, number>> = {};
    for (const f of RETENTION_FIELDS) {
      const v = Number(form[f.key]);
      if (v !== current[f.key]) patch[f.key] = v;
    }
    update.mutate(patch, { onSuccess: () => showToast("success", "Retention settings saved.") });
  };
  return (
    <form onSubmit={onSubmit} aria-label="Retention settings" className="flex flex-col gap-4">
      {(update.isError || reset.isError) && <Alert>{errorMessage(update.error ?? reset.error)}</Alert>}
      <div className="grid gap-4 sm:grid-cols-3">
        {RETENTION_FIELDS.map((f) => (
          <Field
            key={f.key}
            label={f.label}
            inputMode="numeric"
            hint={`${f.hint}${defaults[f.key] !== Number(form[f.key]) ? ` Default: ${defaults[f.key]}.` : ""}`}
            value={form[f.key]}
            onChange={(e) => setForm({ ...form, [f.key]: e.target.value.replace(/\D/g, "") })}
          />
        ))}
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={update.isPending}>
          Save retention settings
        </Button>
        <Button
          variant="secondary"
          disabled={reset.isPending}
          onClick={() => reset.mutate(undefined, { onSuccess: () => showToast("success", "Retention settings reset.") })}
        >
          Reset
        </Button>
      </div>
    </form>
  );
}

function CameraDefaultsForm({ current }: { current: CameraDefaults }) {
  const update = useUpdateCameraDefaults();
  const reset = useResetCameraDefaults();
  const [form, setForm] = useState(current);
  const select = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100";
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    update.mutate(form, { onSuccess: () => showToast("success", "Camera defaults saved.") });
  };
  return (
    <form onSubmit={onSubmit} aria-label="Camera defaults" className="flex flex-col gap-4">
      {(update.isError || reset.isError) && <Alert>{errorMessage(update.error ?? reset.error)}</Alert>}
      <div className="grid gap-4 sm:grid-cols-2">
        <label className="flex flex-col gap-1 text-sm text-slate-300">
          Protocol
          <select
            className={select}
            value={form.scheme}
            onChange={(e) => setForm({ ...form, scheme: e.target.value as CameraDefaults["scheme"] })}
          >
            <option value="http">HTTP</option>
            <option value="https">HTTPS</option>
          </select>
        </label>
        <label className="flex flex-col gap-1 text-sm text-slate-300">
          Certificate check (HTTPS cameras)
          <select
            className={select}
            value={form.tls_mode}
            onChange={(e) => setForm({ ...form, tls_mode: e.target.value as CameraDefaults["tls_mode"] })}
          >
            <option value="insecure_skip_verify">Don't verify (insecure)</option>
            <option value="strict">Strict</option>
          </select>
        </label>
      </div>
      <label className="flex items-start gap-2 text-sm text-slate-300">
        <input
          type="checkbox"
          className="mt-1"
          checked={form.allow_basic_over_http}
          onChange={(e) => setForm({ ...form, allow_basic_over_http: e.target.checked })}
        />
        <span>
          Allow Basic authentication over HTTP
          <span className="block text-xs text-slate-500">
            Needed for cameras that only offer Basic. The password then crosses the network readable.
          </span>
        </span>
      </label>
      <div className="flex gap-2">
        <Button type="submit" disabled={update.isPending}>
          Save camera defaults
        </Button>
        <Button
          variant="secondary"
          disabled={reset.isPending}
          onClick={() => reset.mutate(undefined, { onSuccess: () => showToast("success", "Camera defaults reset.") })}
        >
          Reset
        </Button>
      </div>
    </form>
  );
}
