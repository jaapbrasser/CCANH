import { useState, type FormEvent } from "react";
import { Link } from "react-router";
import { errorMessage } from "../api/client";
import { useCredentialSets, type CredentialInput } from "../api/credentials";
import {
  rangeEntries,
  useDiscoveryJob,
  useDiscoveryJobs,
  useDiscoveryPreview,
  useRegisterDiscovered,
  useStartDiscovery,
  useTestDiscoveredCredentials,
  type CredentialTestResult,
  type DiscoveredDevice,
  type DiscoveryJobDetail,
} from "../api/discovery";
import { groupTree, useCreateGroup, useGroups } from "../api/organization";
import { SHIPPED_CAMERA_DEFAULTS, useSettings } from "../api/settings";
import { isTerminal, useCancelTask, useTask } from "../api/tasks";
import { errorCategoryLabel } from "../utils/format";
import { MultiPick } from "../components/organization";
import { showToast } from "../components/toasts";
import { Alert, Button, Dialog, Field, PageHeader } from "../components/ui";
import { relativeTime } from "../utils/format";
import { usePageTitle } from "../components/pageTitle";

const REASON: Record<string, string> = {
  serial_number: "serial number",
  mac_address: "MAC address",
  ip: "IP address",
  hostname: "hostname",
};

/** Admin page: scan ranges for AXIS devices and register the ones you approve (SPEC §5). */
export function DiscoveryPage() {
  usePageTitle("Discover cameras");
  const jobs = useDiscoveryJobs();
  const [selected, setSelected] = useState<string | null>(null);
  const jobId = selected ?? jobs.data?.[0]?.id ?? null;
  return (
    <div>
      <Link to="/cameras" className="mb-2 inline-block text-sm text-slate-400 hover:text-white">
        ← Cameras
      </Link>
      <PageHeader title="Discover cameras" />
      <p className="mb-4 max-w-3xl text-sm text-slate-400">
        Scan address ranges for AXIS devices. Each address gets a quick port check and, if it answers, one anonymous
        identification request. Nothing is added until you register it.
      </p>
      <div className="grid gap-6 lg:grid-cols-[22rem_1fr]">
        <div className="flex flex-col gap-6">
          <StartForm onStarted={setSelected} />
          <section aria-label="Recent scans">
            <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-slate-400">Recent scans</h2>
            <ul className="divide-y divide-slate-800 rounded border border-slate-800 text-sm">
              {jobs.data?.map((j) => (
                <li key={j.id}>
                  <button
                    type="button"
                    className={`w-full px-3 py-2 text-left hover:bg-slate-900 ${j.id === jobId ? "bg-slate-900" : ""}`}
                    onClick={() => setSelected(j.id)}
                  >
                    <span className="block truncate font-medium">{j.ranges.join(", ")}</span>
                    <span className="text-xs text-slate-500">
                      {j.task.status} · {j.found} found · {relativeTime(j.created_at)}
                    </span>
                  </button>
                </li>
              ))}
              {jobs.data?.length === 0 && <li className="px-3 py-4 text-slate-500">No scans yet.</li>}
            </ul>
          </section>
        </div>
        {jobId ? (
          <JobPanel id={jobId} />
        ) : (
          <div className="text-sm text-slate-500">Start a scan to see results here.</div>
        )}
      </div>
    </div>
  );
}

function StartForm({ onStarted }: { onStarted: (id: string) => void }) {
  const [text, setText] = useState("");
  const [confirm, setConfirm] = useState(false);
  const preview = useDiscoveryPreview(text);
  const start = useStartDiscovery();
  const p = text.trim() ? preview.data : undefined;
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    start.mutate(
      { ranges: rangeEntries(text), confirm_large: confirm },
      {
        onSuccess: (job) => {
          showToast("success", `Scanning ${job.address_count} addresses.`);
          setConfirm(false);
          onStarted(job.id);
        },
      },
    );
  };
  return (
    <form aria-label="New scan" onSubmit={onSubmit} className="flex flex-col gap-3 rounded border border-slate-800 p-4">
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-slate-300">Address ranges</span>
        <textarea
          aria-label="Address ranges"
          className="min-h-20 rounded border border-slate-700 bg-slate-900 px-3 py-2 font-mono text-sm"
          placeholder={"192.0.2.0/24\n198.51.100.10-50"}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <span className="text-xs text-slate-500">
          CIDR blocks, single addresses or ranges; one per line or comma-separated.
        </span>
      </label>
      <p role="status" className="text-sm text-slate-300">
        {preview.isError && text.trim() ? (
          <span className="text-red-300">{errorMessage(preview.error)}</span>
        ) : p ? (
          `${p.address_count.toLocaleString()} address${p.address_count === 1 ? "" : "es"}`
        ) : (
          " "
        )}
      </p>
      {p?.too_large && <Alert>That's more than one scan can cover. Split it into smaller ranges.</Alert>}
      {p?.needs_confirmation && !p.too_large && (
        <label className="flex items-start gap-2 text-sm text-amber-200">
          <input type="checkbox" checked={confirm} onChange={(e) => setConfirm(e.target.checked)} />
          Scan {p.address_count.toLocaleString()} addresses, more than the usual limit of{" "}
          {p.max_addresses.toLocaleString()}
        </label>
      )}
      {start.isError && <Alert>{errorMessage(start.error)}</Alert>}
      <Button type="submit" disabled={!p || p.too_large || (p.needs_confirmation && !confirm) || start.isPending}>
        Start scan
      </Button>
    </form>
  );
}

function statusOf(d: DiscoveredDevice, all: DiscoveredDevice[]) {
  if (d.registered_camera_id) {
    return (
      <Link to={`/cameras/${d.registered_camera_id}`} className="text-emerald-300 hover:underline">
        Registered
      </Link>
    );
  }
  const dup = d.duplicate_of;
  if (dup?.camera_id) {
    return (
      <span className="text-amber-300">
        Already registered as{" "}
        <Link to={`/cameras/${dup.camera_id}`} className="underline">
          {dup.camera_name}
        </Link>{" "}
        <span className="text-xs text-slate-500">(same {REASON[dup.reason]})</span>
      </span>
    );
  }
  if (dup?.result_id) {
    const other = all.find((r) => r.id === dup.result_id);
    return <span className="text-amber-300">Same device as {other?.ip ?? "another result"}</span>;
  }
  return <span className="text-sky-300">New</span>;
}

function JobPanel({ id }: { id: string }) {
  const job = useDiscoveryJob(id);
  const cancel = useCancelTask();
  const [picked, setPicked] = useState<string[]>([]);
  const [registering, setRegistering] = useState(false);
  if (job.isPending) return <p className="text-sm text-slate-400">Loading…</p>;
  if (job.isError) return <Alert>{errorMessage(job.error)}</Alert>;
  const j = job.data;
  const running = !isTerminal(j.task.status);
  const pct = j.address_count ? Math.round((j.scanned / j.address_count) * 100) : 0;
  const results = [...j.results].sort((a, b) => ipKey(a.ip) - ipKey(b.ip));
  const selectable = results.filter((r) => !r.registered_camera_id && !r.duplicate_of).map((r) => r.id);
  const selection = picked.filter((p) => selectable.includes(p));

  return (
    <section aria-label="Scan results" className="flex flex-col gap-4">
      <div className="rounded border border-slate-800 p-4">
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <h2 className="font-semibold">{j.ranges.join(", ")}</h2>
          {running && (
            <Button variant="secondary" disabled={cancel.isPending} onClick={() => cancel.mutate(j.id)}>
              Cancel scan
            </Button>
          )}
        </div>
        <div
          role="progressbar"
          aria-label="Scan progress"
          aria-valuenow={j.scanned}
          aria-valuemin={0}
          aria-valuemax={j.address_count}
          className="h-2 overflow-hidden rounded bg-slate-800"
        >
          <div className="h-full bg-amber-500 transition-all" style={{ width: `${pct}%` }} />
        </div>
        <p className="mt-2 text-sm text-slate-300">
          {j.task.status === "cancelled"
            ? "Cancelled. "
            : j.task.status === "failed"
              ? `Failed: ${j.task.error_message ?? "unknown error"}. `
              : running
                ? "Scanning… "
                : "Finished. "}
          {j.scanned.toLocaleString()} of {j.address_count.toLocaleString()} addresses checked, {j.responsive}{" "}
          responding, {j.found} AXIS device{j.found === 1 ? "" : "s"} found.
        </p>
      </div>

      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-400">Devices found</h3>
        <Button disabled={selection.length === 0} onClick={() => setRegistering(true)}>
          Register {selection.length || ""} selected
        </Button>
      </div>
      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="w-8 px-3 py-2">
                <input
                  type="checkbox"
                  aria-label="Select all new devices"
                  checked={selectable.length > 0 && selection.length === selectable.length}
                  onChange={(e) => setPicked(e.target.checked ? selectable : [])}
                />
              </th>
              <th className="px-3 py-2 font-medium">Address</th>
              <th className="px-3 py-2 font-medium">Model</th>
              <th className="px-3 py-2 font-medium">Serial</th>
              <th className="px-3 py-2 font-medium">Firmware</th>
              <th className="px-3 py-2 font-medium">Status</th>
            </tr>
          </thead>
          <tbody>
            {results.map((r) => {
              const canPick = selectable.includes(r.id);
              return (
                <tr key={r.id} className="border-t border-slate-800">
                  <td className="px-3 py-2">
                    {canPick && (
                      <input
                        type="checkbox"
                        aria-label={`Select ${r.ip}`}
                        checked={selection.includes(r.id)}
                        onChange={(e) =>
                          setPicked(e.target.checked ? [...selection, r.id] : selection.filter((x) => x !== r.id))
                        }
                      />
                    )}
                  </td>
                  <td className="px-3 py-2 font-mono text-xs">
                    {r.scheme}://{r.ip}
                    {r.port !== (r.scheme === "https" ? 443 : 80) ? `:${r.port}` : ""}
                  </td>
                  <td className="px-3 py-2">{r.model ?? "—"}</td>
                  <td className="px-3 py-2 font-mono text-xs">{r.serial_number ?? "—"}</td>
                  <td className="px-3 py-2 text-slate-400">{r.firmware_version ?? "—"}</td>
                  <td className="px-3 py-2">{statusOf(r, results)}</td>
                </tr>
              );
            })}
            {results.length === 0 && (
              <tr>
                <td colSpan={6} className="px-3 py-8 text-center text-slate-500">
                  {running ? "Devices appear here as they're found." : "No AXIS devices found."}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {registering && (
        <RegisterDialog
          job={j}
          resultIds={selection}
          onClose={() => setRegistering(false)}
          onDone={() => {
            setRegistering(false);
            setPicked([]);
          }}
        />
      )}
    </section>
  );
}

function ipKey(ip: string): number {
  return ip.split(".").reduce((n, o) => n * 256 + Number(o), 0);
}

type CredMode = "none" | "set" | "inline";

/** Most devices one credential test checks (server: `MAX_CREDENTIAL_TESTS`). */
const MAX_TESTS = 20;

function RegisterDialog({
  job,
  resultIds,
  onClose,
  onDone,
}: {
  job: DiscoveryJobDetail;
  resultIds: string[];
  onClose: () => void;
  onDone: () => void;
}) {
  const register = useRegisterDiscovered(job.id);
  const sets = useCredentialSets();
  const groups = useGroups();
  const createGroup = useCreateGroup();
  const settings = useSettings();
  const defaults = settings.data?.current.camera_defaults ?? SHIPPED_CAMERA_DEFAULTS;
  const [mode, setMode] = useState<CredMode>("none");
  const [setId, setSetId] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [groupIds, setGroupIds] = useState<string[]>([]);
  const [monitoring, setMonitoring] = useState(true);
  const [basicChoice, setBasicChoice] = useState<boolean | null>(null);
  const allowBasic = basicChoice ?? defaults.allow_basic_over_http;
  const n = resultIds.length;
  const selected = job.results.filter((r) => resultIds.includes(r.id));
  const httpCount = selected.filter((r) => r.scheme === "http").length;

  const test = useTestDiscoveredCredentials(job.id);
  const [testId, setTestId] = useState<string | null>(null);
  const testTask = useTask(testId);
  const testing = Boolean(testId) && !isTerminal(testTask.data?.status);
  const testResult =
    testTask.data?.status === "succeeded" ? (testTask.data.result as unknown as CredentialTestResult | null) : null;

  const credentials = (): CredentialInput =>
    mode === "set"
      ? { type: "set", id: setId }
      : mode === "inline"
        ? { type: "inline", username, password }
        : { type: "none" };
  const credentialsComplete = mode === "none" || (mode === "set" ? Boolean(setId) : Boolean(username && password));

  const runTest = () => {
    setTestId(null);
    test.mutate(
      { result_ids: resultIds.slice(0, MAX_TESTS), credentials: credentials(), allow_basic_over_http: allowBasic },
      { onSuccess: (t) => setTestId(t.id) },
    );
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    register.mutate(
      {
        result_ids: resultIds,
        credentials: credentials(),
        group_ids: groupIds,
        monitoring_enabled: monitoring,
        allow_basic_over_http: allowBasic,
      },
      {
        onSuccess: (out) => {
          const skipped = out.skipped.length ? `, ${out.skipped.length} skipped (${out.skipped[0]!.reason})` : "";
          showToast(
            "success",
            `Registered ${out.registered.length} camera${out.registered.length === 1 ? "" : "s"}${skipped}.`,
          );
          onDone();
        },
      },
    );
  };

  return (
    <Dialog title={`Register ${n} device${n === 1 ? "" : "s"}`} onClose={onClose}>
      <form onSubmit={onSubmit} className="flex max-h-[75vh] flex-col gap-4 overflow-y-auto pr-1 text-sm">
        {register.isError && <Alert>{errorMessage(register.error)}</Alert>}
        <p className="text-slate-400">
          HTTPS devices are pinned to the certificate seen during the scan. Names come from the device; one that reports
          none shows its address until it does.
        </p>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-slate-300">Credentials</legend>
          {(
            [
              ["none", "None (status only)"],
              ["set", "Shared credential set"],
              ["inline", "Username and password for these cameras"],
            ] as const
          ).map(([m, label]) => (
            <label key={m} className="flex items-center gap-2">
              <input
                type="radio"
                name="cred"
                checked={mode === m}
                onChange={() => {
                  setMode(m);
                  setTestId(null);
                }}
              />
              {label}
            </label>
          ))}
          {mode === "set" && (
            <select
              aria-label="Credential set"
              className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
              value={setId}
              onChange={(e) => setSetId(e.target.value)}
              required
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
              <Field label="Username" required value={username} onChange={(e) => setUsername(e.target.value)} />
              <Field
                label="Password"
                type="password"
                required
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
          )}
          {httpCount > 0 && (
            <label className="flex items-start gap-2 text-slate-300">
              <input
                type="checkbox"
                className="mt-1"
                checked={allowBasic}
                onChange={(e) => {
                  setBasicChoice(e.target.checked);
                  setTestId(null);
                }}
              />
              <span>
                Allow Basic authentication over HTTP
                <span className="block text-xs text-slate-500">
                  Applies to {httpCount} of {n} selected device{n === 1 ? "" : "s"} (the ones found on HTTP). The
                  password then crosses the network readable.
                </span>
              </span>
            </label>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="secondary" disabled={!credentialsComplete || testing || test.isPending} onClick={runTest}>
              {testing ? "Testing…" : "Test credentials"}
            </Button>
            <span className="text-xs text-slate-500">
              {n > MAX_TESTS
                ? `Tests the first ${MAX_TESTS} selected devices.`
                : "One sign-in attempt per device; nothing is saved."}
            </span>
          </div>
          {test.isError && <Alert>{errorMessage(test.error)}</Alert>}
          {testTask.data?.status === "failed" && <Alert>{testTask.data.error_message ?? "The test failed."}</Alert>}
          {testResult && <CredentialResults result={testResult} />}
        </fieldset>
        <MultiPick
          legend="Groups"
          options={groupTree(groups.data ?? []).map((g) => ({ id: g.id, label: g.name, depth: g.depth }))}
          selected={groupIds}
          onChange={setGroupIds}
          empty="No groups yet: create one below."
          createLabel="New group name"
          onCreate={async (name) => (await createGroup.mutateAsync({ name })).id}
        />
        <label className="flex items-center gap-2 text-slate-300">
          <input type="checkbox" checked={monitoring} onChange={(e) => setMonitoring(e.target.checked)} />
          Monitor these cameras' status
        </label>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={register.isPending}>
            Register {n}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function CredentialResults({ result }: { result: CredentialTestResult }) {
  const passed = result.results.filter((r) => r.ok).length;
  return (
    <div role="region" aria-label="Credential test result" className="rounded border border-slate-700 p-3">
      <p className={`mb-2 font-medium ${passed === result.results.length ? "text-emerald-300" : "text-amber-300"}`}>
        Credentials accepted by {passed} of {result.results.length} device{result.results.length === 1 ? "" : "s"}
      </p>
      <ul className="flex flex-col gap-1">
        {result.results.map((r) => (
          <li key={r.result_id} className="flex gap-2">
            <span aria-hidden className={r.ok ? "text-emerald-400" : "text-red-400"}>
              {r.ok ? "✓" : "✕"}
            </span>
            <span className="font-mono">{r.ip}</span>
            {!r.ok && (
              <span className="text-slate-400">
                {errorCategoryLabel(r.error_category)}
                {r.message && <span className="block text-xs text-slate-500">{r.message}</span>}
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
