import { useState } from "react";
import { errorMessage } from "../api/client";
import { isTerminal, useCancelTask, useTasks, type Task } from "../api/tasks";
import { Alert, Button, PageHeader, Pagination } from "../components/ui";
import { errorCategoryLabel, relativeTime } from "../utils/format";

const KIND_LABELS: Record<string, string> = { test_connection: "Connection test" };

const STATUS_STYLES: Record<Task["status"], string> = {
  queued: "text-slate-300",
  running: "text-sky-300",
  succeeded: "text-emerald-300",
  failed: "text-red-300",
  cancelled: "text-slate-400",
};

function outcome(task: Task): string {
  if (task.status === "failed") {
    return [errorCategoryLabel(task.error_category), task.error_message].filter(Boolean).join(": ");
  }
  if (task.status === "succeeded") {
    const ok = (task.result as { ok?: boolean } | null)?.ok;
    return ok === false ? "Finished with problems" : "Finished";
  }
  if (task.progress_total) return `${task.message ?? ""} (${task.progress_current}/${task.progress_total})`;
  return task.message ?? "";
}

export function ActivityPage() {
  const [page, setPage] = useState(1);
  const tasks = useTasks({ page, page_size: 25 });
  const cancel = useCancelTask();

  return (
    <div>
      <PageHeader title="Activity" />
      <p className="mb-4 text-sm text-slate-400">Background tasks you started (admins see everyone's). Updates live.</p>
      {(tasks.isError || cancel.isError) && <Alert>{errorMessage(tasks.error ?? cancel.error)}</Alert>}
      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="px-4 py-2 font-medium">Task</th>
              <th className="px-4 py-2 font-medium">Started by</th>
              <th className="px-4 py-2 font-medium">Status</th>
              <th className="px-4 py-2 font-medium">Details</th>
              <th className="px-4 py-2 font-medium">Created</th>
              <th className="px-4 py-2 font-medium">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {tasks.data?.items.map((t) => (
              <tr key={t.id} className="border-t border-slate-800">
                <td className="px-4 py-2">{KIND_LABELS[t.kind] ?? t.kind}</td>
                <td className="px-4 py-2 text-slate-300">{t.created_by_username ?? "system"}</td>
                <td className={`px-4 py-2 ${STATUS_STYLES[t.status]}`}>{t.status}</td>
                <td className="px-4 py-2 text-slate-400">{outcome(t)}</td>
                <td className="px-4 py-2 text-slate-400">{relativeTime(t.created_at)}</td>
                <td className="px-4 py-2 text-right">
                  {!isTerminal(t.status) && (
                    <Button variant="secondary" disabled={cancel.isPending} onClick={() => cancel.mutate(t.id)}>
                      Cancel
                    </Button>
                  )}
                </td>
              </tr>
            ))}
            {tasks.data?.items.length === 0 && (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-slate-500">
                  No tasks yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {tasks.data && (
        <div className="mt-4">
          <Pagination page={tasks.data.page} pageSize={tasks.data.page_size} total={tasks.data.total} onPage={setPage} />
        </div>
      )}
    </div>
  );
}
