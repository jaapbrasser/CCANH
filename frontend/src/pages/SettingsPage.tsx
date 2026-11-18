import { useState, type FormEvent } from "react";
import { errorMessage } from "../api/client";
import {
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
  const settings = useSettings();
  const current = settings.data?.current.monitoring;
  const stats = settings.data?.monitor;

  return (
    <div className="max-w-3xl">
      <PageHeader title="Settings" />
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

type SKey = keyof StreamingSettings;
const STREAM_FIELDS: { key: SKey; label: string; hint: string }[] = [
  { key: "grace_period_secs", label: "Grace period (seconds)", hint: "How long a camera stream stays connected after its last viewer leaves (0–300)." },
  { key: "stream_buffer_mb", label: "Buffer per stream (MB)", hint: "Memory for the latest keyframe group, so new viewers start instantly (1–64)." },
  { key: "global_buffer_mb", label: "Total buffer (MB)", hint: "Memory cap across all streams (16–4096)." },
];

function StreamingForm({ current, defaults }: { current: StreamingSettings; defaults: StreamingSettings }) {
  const update = useUpdateStreaming();
  const reset = useResetStreaming();
  const [form, setForm] = useState(
    () => Object.fromEntries(STREAM_FIELDS.map((f) => [f.key, String(current[f.key])])) as Record<SKey, string>,
  );
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const patch: Partial<Record<SKey, number>> = {};
    for (const f of STREAM_FIELDS) {
      const v = Number(form[f.key]);
      if (v !== current[f.key]) patch[f.key] = v;
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
