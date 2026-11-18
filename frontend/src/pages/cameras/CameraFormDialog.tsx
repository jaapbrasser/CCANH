import { useState, type FormEvent } from "react";
import { errorMessage } from "../../api/client";
import {
  useCreateCamera,
  useUpdateCamera,
  type CameraDetail,
  type NewCamera,
} from "../../api/cameras";
import type { components } from "../../api/schema";
import { Alert, Button, Dialog, Field } from "../../components/ui";

type Scheme = components["schemas"]["Scheme"];
type TlsMode = components["schemas"]["TlsMode"];

interface FormState {
  name: string;
  host: string;
  port: string;
  scheme: Scheme;
  tls_mode: TlsMode;
  tls_pinned_sha256: string;
  monitoring_enabled: boolean;
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
  };
}

function toBody(f: FormState): NewCamera {
  return {
    name: f.name.trim(),
    host: f.host.trim(),
    port: f.port.trim() ? Number(f.port) : null,
    scheme: f.scheme,
    tls_mode: f.tls_mode,
    tls_pinned_sha256: f.tls_mode === "pinned" ? f.tls_pinned_sha256.trim() : null,
    monitoring_enabled: f.monitoring_enabled,
  };
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
  const set = <K extends keyof FormState>(key: K, value: FormState[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const body = toBody(form);
    if (camera) update.mutate(body, { onSuccess: onSaved });
    else create.mutate(body, { onSuccess: onSaved });
  };

  const select = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100";

  return (
    <Dialog title={camera ? `Edit ${camera.name}` : "Add camera"} onClose={onClose}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
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
            placeholder={form.scheme === "https" ? "443" : "80"}
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
              disabled={form.scheme === "http"}
              onChange={(e) => set("tls_mode", e.target.value as TlsMode)}
            >
              <option value="strict">Strict (default)</option>
              <option value="pinned">Pinned fingerprint</option>
              <option value="insecure_skip_verify">Don't verify (insecure)</option>
            </select>
          </label>
        </div>
        {form.tls_mode === "pinned" && form.scheme === "https" && (
          <Field
            label="SHA-256 fingerprint"
            required
            placeholder="AB:CD:…"
            value={form.tls_pinned_sha256}
            onChange={(e) => set("tls_pinned_sha256", e.target.value)}
          />
        )}
        {form.tls_mode === "insecure_skip_verify" && form.scheme === "https" && (
          <Alert>
            The camera's certificate won't be checked, so the connection could be intercepted. This
            choice is logged.
          </Alert>
        )}
        <label className="flex items-center gap-2 text-sm text-slate-300">
          <input
            type="checkbox"
            checked={form.monitoring_enabled}
            onChange={(e) => set("monitoring_enabled", e.target.checked)}
          />
          Monitor this camera's status
        </label>
        <p className="text-xs text-slate-500">Camera credentials can be added once the AXIS integration ships.</p>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={mutation.isPending}>
            {camera ? "Save" : "Add camera"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
