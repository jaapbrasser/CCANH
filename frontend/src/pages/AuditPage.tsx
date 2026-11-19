import { useState } from "react";
import { useAudit, type AuditQuery } from "../api/audit";
import { errorMessage } from "../api/client";
import { Alert, PageHeader, Pagination } from "../components/ui";
import { usePageTitle } from "../components/pageTitle";

const PAGE_SIZE = 50;

const ACTIONS = [
  { value: "", label: "All actions" },
  { value: "auth.", label: "Authentication" },
  { value: "auth.login", label: "Logins" },
  { value: "user.", label: "User changes" },
  { value: "setup.", label: "Setup" },
];

export function AuditPage() {
  usePageTitle("Audit log");
  const [page, setPage] = useState(1);
  const [action, setAction] = useState("");
  const [outcome, setOutcome] = useState<"" | "success" | "failure">("");
  const [actor, setActor] = useState("");

  const query: AuditQuery = {
    page,
    page_size: PAGE_SIZE,
    action: action || undefined,
    outcome: outcome || undefined,
    actor: actor.trim() || undefined,
  };
  const audit = useAudit(query);

  const reset = <T,>(setter: (v: T) => void) => (v: T) => {
    setter(v);
    setPage(1);
  };

  return (
    <div>
      <PageHeader title="Audit log" />
      <div className="mb-4 flex flex-wrap gap-3 text-sm">
        <select
          aria-label="Filter by action"
          className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
          value={action}
          onChange={(e) => reset(setAction)(e.target.value)}
        >
          {ACTIONS.map((a) => (
            <option key={a.value} value={a.value}>
              {a.label}
            </option>
          ))}
        </select>
        <select
          aria-label="Filter by outcome"
          className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
          value={outcome}
          onChange={(e) => reset(setOutcome)(e.target.value as typeof outcome)}
        >
          <option value="">Any outcome</option>
          <option value="success">Success</option>
          <option value="failure">Failure</option>
        </select>
        <input
          type="search"
          aria-label="Filter by user"
          placeholder="User"
          className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
          value={actor}
          onChange={(e) => reset(setActor)(e.target.value)}
        />
      </div>

      {audit.isError && <Alert>{errorMessage(audit.error)}</Alert>}

      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="px-4 py-2 font-medium">Time</th>
              <th className="px-4 py-2 font-medium">User</th>
              <th className="px-4 py-2 font-medium">Action</th>
              <th className="px-4 py-2 font-medium">Outcome</th>
              <th className="px-4 py-2 font-medium">Source IP</th>
              <th className="px-4 py-2 font-medium">Details</th>
            </tr>
          </thead>
          <tbody>
            {audit.data?.items.map((entry) => (
              <tr key={entry.id} className="border-t border-slate-800 align-top">
                <td className="whitespace-nowrap px-4 py-2 text-slate-400">
                  {new Date(entry.occurred_at).toLocaleString()}
                </td>
                <td className="px-4 py-2">{entry.actor_username ?? "—"}</td>
                <td className="px-4 py-2 font-mono text-xs">{entry.action}</td>
                <td className="px-4 py-2">
                  <span className={entry.outcome === "success" ? "text-emerald-300" : "text-red-300"}>
                    {entry.outcome}
                  </span>
                </td>
                <td className="px-4 py-2 text-slate-400">{entry.source_ip ?? "—"}</td>
                <td className="px-4 py-2 font-mono text-xs text-slate-400">
                  {entry.details ? JSON.stringify(entry.details) : ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {audit.data && (
        <div className="mt-4">
          <Pagination
            page={audit.data.page}
            pageSize={audit.data.page_size}
            total={audit.data.total}
            onPage={setPage}
          />
        </div>
      )}
    </div>
  );
}
