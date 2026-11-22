import { useHealth } from "../api/health";

export function HealthBadge() {
  const { data, isPending, isError } = useHealth();

  if (isPending) {
    return <span className="text-sm text-slate-400">Checking server…</span>;
  }
  if (isError) {
    return (
      <span role="status" className="rounded bg-red-900/60 px-2 py-1 text-sm text-red-200">
        Server unreachable
      </span>
    );
  }

  const ok = data.status === "ok";
  return (
    <span
      role="status"
      className={
        ok
          ? "rounded bg-emerald-900/60 px-2 py-1 text-sm text-emerald-200"
          : "rounded bg-yellow-900/60 px-2 py-1 text-sm text-yellow-200"
      }
    >
      {ok ? "Server healthy" : "Server degraded"} · v{data.version}
    </span>
  );
}
