import { useState, type FormEvent } from "react";
import { errorMessage } from "../api/client";
import { useCredentialSets, type CredentialInput } from "../api/credentials";
import { useBulkCameras, type BulkSelection, type BulkSettings } from "../api/organization";
import { showToast } from "./toasts";
import { Alert, Button, Dialog, Field } from "./ui";

type CredMode = "none" | "set" | "inline";
const select = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100 disabled:opacity-50";

/** A setting row: tick "change" to include it; unticked settings are left alone. */
function Row({
  label,
  on,
  onToggle,
  children,
}: {
  label: string;
  on: boolean;
  onToggle: (on: boolean) => void;
  children: React.ReactNode;
}) {
  return (
    <fieldset className="flex flex-col gap-2 rounded border border-slate-800 p-3">
      <legend className="px-1">
        <label className="flex items-center gap-2 text-slate-200">
          <input type="checkbox" checked={on} onChange={(e) => onToggle(e.target.checked)} />
          Change {label.toLowerCase()}
        </label>
      </legend>
      <div className={on ? "" : "pointer-events-none opacity-50"} aria-disabled={!on}>
        {children}
      </div>
    </fieldset>
  );
}

/**
 * Change connection and security settings on many cameras at once (issue
 * #19): a selection, a group (with subgroups) or a tag. Only ticked settings
 * change; cameras that can't take a change are skipped with a reason.
 */
export function BulkSettingsDialog({
  selection,
  description,
  onClose,
  onStarted,
}: {
  selection: BulkSelection;
  /** e.g. "3 selected cameras" or "cameras in Site A". */
  description: string;
  onClose: () => void;
  onStarted?: () => void;
}) {
  const bulk = useBulkCameras();
  const sets = useCredentialSets();
  const [creds, setCreds] = useState(false);
  const [mode, setMode] = useState<CredMode>("set");
  const [setId, setSetId] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [basic, setBasic] = useState(false);
  const [allowBasic, setAllowBasic] = useState(true);
  const [tls, setTls] = useState(false);
  const [tlsMode, setTlsMode] = useState<"strict" | "insecure_skip_verify">("insecure_skip_verify");
  const [conn, setConn] = useState(false);
  const [scheme, setScheme] = useState<"http" | "https">("http");
  const [port, setPort] = useState("");

  const credentials = (): CredentialInput =>
    mode === "set" ? { type: "set", id: setId } : mode === "inline" ? { type: "inline", username, password } : { type: "none" };
  const settings: BulkSettings = {
    ...(creds ? { credentials: credentials() } : {}),
    ...(basic ? { allow_basic_over_http: allowBasic } : {}),
    ...(tls ? { tls_mode: tlsMode } : {}),
    ...(conn ? { scheme, port: port ? Number(port) : scheme === "https" ? 443 : 80 } : {}),
  };
  const nothing = !creds && !basic && !tls && !conn;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    bulk.mutate(
      { ...selection, action: { type: "update_settings", settings } },
      {
        onSuccess: () => {
          showToast("success", `Updating settings for ${description}…`);
          onStarted?.();
          onClose();
        },
      },
    );
  };

  return (
    <Dialog title="Edit camera settings" onClose={onClose}>
      <form onSubmit={onSubmit} className="flex max-h-[75vh] flex-col gap-3 overflow-y-auto pr-1 text-sm">
        <p className="text-slate-400">
          Applies to {description}. Only the settings you tick change; everything else stays as it is.
        </p>
        {bulk.isError && <Alert>{errorMessage(bulk.error)}</Alert>}

        <Row label="Credentials" on={creds} onToggle={setCreds}>
          <div className="flex flex-col gap-2">
            {(
              [
                ["set", "Shared credential set"],
                ["inline", "Username and password (each camera gets its own copy)"],
                ["none", "None (status only)"],
              ] as const
            ).map(([m, label]) => (
              <label key={m} className="flex items-center gap-2">
                <input type="radio" name="bulk-cred" checked={mode === m} onChange={() => setMode(m)} />
                {label}
              </label>
            ))}
            {mode === "set" && (
              <select
                aria-label="Credential set"
                className={select}
                required={creds}
                value={setId}
                onChange={(e) => setSetId(e.target.value)}
              >
                <option value="">Choose a set…</option>
                {sets.data?.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
            )}
            {mode === "inline" && (
              <div className="grid gap-2 sm:grid-cols-2">
                <Field label="Username" required={creds} value={username} onChange={(e) => setUsername(e.target.value)} />
                <Field
                  label="Password"
                  type="password"
                  autoComplete="new-password"
                  required={creds}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </div>
            )}
          </div>
        </Row>

        <Row label="Basic authentication over HTTP" on={basic} onToggle={setBasic}>
          <label className="flex items-start gap-2">
            <input type="checkbox" className="mt-1" checked={allowBasic} onChange={(e) => setAllowBasic(e.target.checked)} />
            <span>
              Allow Basic authentication over HTTP
              <span className="block text-xs text-slate-500">Only affects HTTP cameras. The password then crosses the network readable.</span>
            </span>
          </label>
        </Row>

        <Row label="Certificate check" on={tls} onToggle={setTls}>
          <select
            aria-label="Certificate check"
            className={select}
            value={tlsMode}
            onChange={(e) => setTlsMode(e.target.value as typeof tlsMode)}
          >
            <option value="insecure_skip_verify">Don't verify (insecure)</option>
            <option value="strict">Strict</option>
          </select>
          <p className="mt-1 text-xs text-slate-500">Only HTTPS cameras have a certificate; HTTP cameras are left as they are.</p>
        </Row>

        <Row label="Protocol and port" on={conn} onToggle={setConn}>
          <div className="grid grid-cols-[1fr_7rem] gap-2">
            <select aria-label="Protocol" className={select} value={scheme} onChange={(e) => setScheme(e.target.value as typeof scheme)}>
              <option value="http">HTTP</option>
              <option value="https">HTTPS</option>
            </select>
            <Field
              label="Port"
              inputMode="numeric"
              placeholder={scheme === "https" ? "443" : "80"}
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
            />
          </div>
          <p className="mt-1 text-xs text-slate-500">Changing the address resets the cameras' status and identity.</p>
        </Row>

        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={nothing || bulk.isPending}>
            Apply to {description}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
