import { useState, type FormEvent } from "react";
import { errorMessage } from "../../api/client";
import {
  useCreateCamera,
  useUpdateCamera,
  type CameraDetail,
  type NewCamera,
} from "../../api/cameras";
import {
  useCredentialSets,
  useTestConnection,
  type CredentialInput,
  type TestConnectionResult,
} from "../../api/credentials";
import type { components } from "../../api/schema";
import { isTerminal, useCancelTask, useTask } from "../../api/tasks";
import { Alert, Button, Dialog, Field } from "../../components/ui";
import { errorCategoryLabel } from "../../utils/format";

type Scheme = components["schemas"]["Scheme"];
type TlsMode = components["schemas"]["TlsMode"];
type CredMode = "keep" | "none" | "set" | "inline";

interface FormState {
  name: string;
  host: string;
  port: string;
  scheme: Scheme;
  tls_mode: TlsMode;
  tls_pinned_sha256: string;
  monitoring_enabled: boolean;
  allow_basic_over_http: boolean;
  credMode: CredMode;
  setId: string;
  username: string;
  password: string;
}

function initial(camera?: CameraDetail): FormState {
  return {
    name: camera?.name ?? "",
    host: camera?.host ?? "",
    port: camera ? String(camera.port) : "",
    scheme: camera?.scheme ?? "https",
    tls_mode: camera?.tls_mode ?? "strict",
    tls_pinned_sha256: camera?.tls_pinned_sha256 ?? "",
    monitoring_enabled: camera?.monitoring_enabled ?? true,
    allow_basic_over_http: camera?.allow_basic_over_http ?? false,
    credMode: camera ? "keep" : "none",
    setId: camera?.credential_set?.id ?? "",
    username: "",
    password: "",
  };
}

/** The credential choice to send, or `undefined` to keep the current one. */
function credentialInput(f: FormState): CredentialInput | undefined {
  switch (f.credMode) {
    case "keep":
      return undefined;
    case "none":
      return { type: "none" };
    case "set":
      return { type: "set", id: f.setId };
    case "inline":
      return { type: "inline", username: f.username, password: f.password };
  }
}

function connectionFields(f: FormState) {
  return {
    host: f.host.trim(),
    port: f.port.trim() ? Number(f.port) : null,
    scheme: f.scheme,
    tls_mode: f.tls_mode,
    tls_pinned_sha256: f.tls_mode === "pinned" ? f.tls_pinned_sha256.trim() : null,
    allow_basic_over_http: f.scheme === "http" && f.allow_basic_over_http,
  };
}

function toBody(f: FormState): NewCamera {
  return {
    name: f.name.trim(),
    ...connectionFields(f),
    monitoring_enabled: f.monitoring_enabled,
    credentials: credentialInput(f) ?? null,
  };
}

const STEP_LABELS: Record<string, string> = {
  reachability: "Reachable and identified",
  credentials: "Credentials accepted",
  capabilities: "Capabilities read",
  device_info: "Device details read",
};

function TestResult({
  result,
  pinned,
  onPin,
}: {
  result: TestConnectionResult;
  pinned: boolean;
  onPin: (fp: string) => void;
}) {
  return (
    <div role="region" aria-label="Connection test result" className="rounded border border-slate-700 p-3 text-sm">
      <p className={`mb-2 font-medium ${result.ok ? "text-emerald-300" : "text-amber-300"}`}>
        {result.ok ? "Connection test passed" : "Connection test found problems"}
      </p>
      <ul className="mb-2 flex flex-col gap-1">
        {result.steps.map((s) => (
          <li key={s.name} className="flex gap-2">
            <span aria-hidden className={s.ok ? "text-emerald-400" : s.skipped ? "text-slate-500" : "text-red-400"}>
              {s.ok ? "✓" : s.skipped ? "–" : "✕"}
            </span>
            <span>
              {STEP_LABELS[s.name] ?? s.name}
              {!s.ok && !s.skipped && s.error_category && (
                <span className="text-slate-400"> · {errorCategoryLabel(s.error_category)}</span>
              )}
              {s.message && !s.ok && <span className="block text-xs text-slate-500">{s.message}</span>}
            </span>
          </li>
        ))}
      </ul>
      {result.device && (
        <p className="text-slate-300">
          {[result.device.model, result.device.firmware_version && `firmware ${result.device.firmware_version}`]
            .filter(Boolean)
            .join(", ")}
          {result.response_time_ms != null && <span className="text-slate-500"> · {result.response_time_ms} ms</span>}
        </p>
      )}
      {result.tls_fingerprint_sha256 && (
        <div className="mt-2">
          <p className="text-xs text-slate-400">Certificate SHA-256</p>
          <p className="break-all font-mono text-xs">{result.tls_fingerprint_sha256}</p>
          {!pinned && (
            <Button variant="secondary" className="mt-1" onClick={() => onPin(result.tls_fingerprint_sha256!)}>
              Pin this certificate
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

/** Add a camera, or edit one when `camera` is given. */
export function CameraFormDialog({
  camera,
  onClose,
  onSaved,
}: {
  camera?: CameraDetail;
  onClose: () => void;
  onSaved: (c: CameraDetail) => void;
}) {
  const [form, setForm] = useState(() => initial(camera));
  const create = useCreateCamera();
  const update = useUpdateCamera(camera?.id ?? "");
  const mutation = camera ? update : create;
  const sets = useCredentialSets();
  const test = useTestConnection();
  const [testId, setTestId] = useState<string | null>(null);
  const testTask = useTask(testId);
  const cancelTest = useCancelTask();
  const task = testTask.data;
  const testRunning = Boolean(testId) && !isTerminal(task?.status);
  const testResult =
    task?.status === "succeeded" ? (task.result as unknown as TestConnectionResult | null) : null;
  const set = <K extends keyof FormState>(key: K, value: FormState[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const body = toBody(form);
    if (camera) {
      const { credentials, ...rest } = body;
      update.mutate(form.credMode === "keep" ? rest : { ...rest, credentials }, { onSuccess: onSaved });
    } else {
      create.mutate(body, { onSuccess: onSaved });
    }
  };

  const runTest = () => {
    const credentials = credentialInput(form);
    test.mutate(
      {
        ...connectionFields(form),
        credentials: credentials ?? null,
        camera_id: form.credMode === "keep" ? camera?.id : null,
      },
      { onSuccess: (t) => setTestId(t.id) },
    );
  };

  const select = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100";
  const https = form.scheme === "https";

  return (
    <Dialog title={camera ? `Edit ${camera.name}` : "Add camera"} onClose={onClose}>
      <form onSubmit={onSubmit} className="flex max-h-[75vh] flex-col gap-4 overflow-y-auto pr-1">
        {mutation.isError && <Alert>{errorMessage(mutation.error)}</Alert>}
        <Field label="Name" required autoFocus value={form.name} onChange={(e) => set("name", e.target.value)} />
        <div className="grid grid-cols-[1fr_7rem] gap-3">
          <Field
            label="Host"
            required
            placeholder="192.0.2.10 or camera.local"
            value={form.host}
            onChange={(e) => set("host", e.target.value)}
          />
          <Field
            label="Port"
            inputMode="numeric"
            placeholder={https ? "443" : "80"}
            value={form.port}
            onChange={(e) => set("port", e.target.value.replace(/\D/g, ""))}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <label className="flex flex-col gap-1 text-sm text-slate-300">
            Protocol
            <select className={select} value={form.scheme} onChange={(e) => set("scheme", e.target.value as Scheme)}>
              <option value="https">HTTPS</option>
              <option value="http">HTTP</option>
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm text-slate-300">
            Certificate check
            <select
              className={select}
              value={form.tls_mode}
              disabled={!https}
              onChange={(e) => set("tls_mode", e.target.value as TlsMode)}
            >
              <option value="strict">Strict (default)</option>
              <option value="pinned">Pinned fingerprint</option>
              <option value="insecure_skip_verify">Don't verify (insecure)</option>
            </select>
          </label>
        </div>
        {form.tls_mode === "pinned" && https && (
          <Field
            label="SHA-256 fingerprint"
            required
            placeholder="AB:CD:…"
            value={form.tls_pinned_sha256}
            onChange={(e) => set("tls_pinned_sha256", e.target.value)}
          />
        )}
        {form.tls_mode === "insecure_skip_verify" && https && (
          <Alert>
            The camera's certificate won't be checked, so the connection could be intercepted. This
            choice is logged.
          </Alert>
        )}

        <fieldset className="flex flex-col gap-3 rounded border border-slate-800 p-3">
          <legend className="px-1 text-sm text-slate-300">Credentials</legend>
          <label className="flex flex-col gap-1 text-sm text-slate-300">
            Authentication
            <select
              aria-label="Authentication"
              className={select}
              value={form.credMode}
              onChange={(e) => set("credMode", e.target.value as CredMode)}
            >
              {camera && (
                <option value="keep">
                  Keep current ({camera.credential_set?.name ?? (camera.has_credentials ? "camera-specific" : "none")})
                </option>
              )}
              <option value="none">None</option>
              <option value="set">Saved credential set</option>
              <option value="inline">Enter for this camera</option>
            </select>
          </label>
          {form.credMode === "set" && (
            <label className="flex flex-col gap-1 text-sm text-slate-300">
              Credential set
              <select
                aria-label="Credential set"
                className={select}
                required
                value={form.setId}
                onChange={(e) => set("setId", e.target.value)}
              >
                <option value="" disabled>
                  {sets.data?.length ? "Choose a set" : "No sets yet — create one under Camera credentials"}
                </option>
                {sets.data?.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {form.credMode === "inline" && (
            <div className="grid grid-cols-2 gap-3">
              <Field
                label="Camera username"
                autoComplete="off"
                required
                value={form.username}
                onChange={(e) => set("username", e.target.value)}
              />
              <Field
                label="Camera password"
                type="password"
                autoComplete="new-password"
                required
                value={form.password}
                onChange={(e) => set("password", e.target.value)}
              />
            </div>
          )}
          {!https && (
            <label className="flex items-start gap-2 text-sm text-slate-300">
              <input
                type="checkbox"
                className="mt-1"
                checked={form.allow_basic_over_http}
                onChange={(e) => set("allow_basic_over_http", e.target.checked)}
              />
              <span>
                Allow Basic authentication over HTTP
                <span className="block text-xs text-slate-500">
                  Sends the password unencrypted. Only needed for cameras without Digest support.
                </span>
              </span>
            </label>
          )}
        </fieldset>

        <label className="flex items-center gap-2 text-sm text-slate-300">
          <input
            type="checkbox"
            checked={form.monitoring_enabled}
            onChange={(e) => set("monitoring_enabled", e.target.checked)}
          />
          Monitor this camera's status
        </label>

        {test.isError && <Alert>{errorMessage(test.error)}</Alert>}
        {testRunning && (
          <div role="status" aria-label="Connection test progress" className="flex items-center justify-between gap-3 rounded border border-slate-700 p-3 text-sm">
            <span>
              {task?.message ?? "Starting test…"}
              {task?.progress_total ? (
                <span className="text-slate-500">
                  {" "}
                  ({task.progress_current}/{task.progress_total})
                </span>
              ) : null}
            </span>
            <Button variant="secondary" disabled={cancelTest.isPending} onClick={() => testId && cancelTest.mutate(testId)}>
              Cancel test
            </Button>
          </div>
        )}
        {task && task.status !== "succeeded" && isTerminal(task.status) && (
          <Alert>
            {task.status === "cancelled" ? "The connection test was cancelled." : (task.error_message ?? "The connection test failed.")}
          </Alert>
        )}
        {testResult && (
          <TestResult
            result={testResult}
            pinned={form.tls_mode === "pinned" && form.tls_pinned_sha256 === testResult.tls_fingerprint_sha256}
            onPin={(fp) => setForm((f) => ({ ...f, tls_mode: "pinned", tls_pinned_sha256: fp }))}
          />
        )}

        <div className="flex justify-between gap-2">
          <Button variant="secondary" disabled={!form.host.trim() || test.isPending || testRunning} onClick={runTest}>
            {test.isPending || testRunning ? "Testing…" : "Test connection"}
          </Button>
          <div className="flex gap-2">
            <Button variant="secondary" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {camera ? "Save" : "Add camera"}
            </Button>
          </div>
        </div>
      </form>
    </Dialog>
  );
}
