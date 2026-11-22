import { NavLink, Outlet } from "react-router";
import { useMe } from "../api/auth";
import { useCameraSummary } from "../api/cameras";
import { useHealth } from "../api/health";
import { useLiveState, useLiveUpdates } from "../api/live";
import { HealthBadge } from "./HealthBadge";
import { Toaster } from "./Toaster";
import { UserMenu } from "./UserMenu";

const linkClass = ({ isActive }: { isActive: boolean }) =>
  `rounded px-3 py-1.5 text-sm ${isActive ? "bg-slate-800 text-white" : "text-slate-400 hover:text-white"}`;

function LiveIndicator() {
  const state = useLiveState();
  const live = state === "live";
  return (
    <span
      role="status"
      aria-label={live ? "Live updates connected" : "Live updates reconnecting"}
      className="flex items-center gap-1.5 text-xs text-slate-400"
    >
      <span className={`h-2 w-2 rounded-full ${live ? "bg-emerald-400" : "bg-yellow-400"}`} />
      {live ? "Live" : state === "reconnecting" ? "Reconnecting…" : "Connecting…"}
    </span>
  );
}

/**
 * `56 🟢 / 44 🔴 (56% online)` for the monitored cameras (issue #32). Unknown
 * counts as offline; cameras with monitoring off aren't counted.
 */
export function CameraCount({ enabled }: { enabled: boolean }) {
  const { data } = useCameraSummary(enabled);
  if (!data) return null;
  const down = data.offline + data.unknown;
  const total = data.online + down;
  if (total === 0) return null;
  const pct = Math.round((data.online / total) * 100);
  return (
    <span
      role="status"
      aria-label={`${data.online} cameras online, ${down} offline or unknown, ${pct}% online`}
      title="Monitored cameras; unknown counts as offline"
      className="whitespace-nowrap text-xs tabular-nums text-slate-400"
    >
      {data.online} 🟢 / {down} 🔴 ({pct}% online)
    </span>
  );
}

export function DemoBanner() {
  const { data } = useHealth();
  if (data?.mode !== "demo") return null;
  return (
    <div role="note" className="bg-sky-500 px-4 py-1 text-center text-sm font-semibold text-slate-950">
      DEMO MODE · simulated cameras · data resets when the server restarts
    </div>
  );
}

export function Layout() {
  const { data: me } = useMe();
  useLiveUpdates(Boolean(me), me?.id, me?.role === "admin");

  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100">
      <DemoBanner />
      <header className="flex items-center justify-between gap-4 border-b border-slate-800 px-6 py-3">
        <div className="flex items-center gap-6">
          <NavLink to="/cameras" className="flex items-center gap-3">
            <img src="/favicon.svg" alt="" className="h-7 w-7" />
            <span className="text-lg font-semibold tracking-tight">Crane Camera Hub</span>
          </NavLink>
          <nav aria-label="Main" className="flex gap-1">
            <NavLink to="/dashboards" className={linkClass}>
              Dashboards
            </NavLink>
            <NavLink to="/cameras" className={linkClass}>
              Cameras
            </NavLink>
          </nav>
        </div>
        <div className="flex items-center gap-4">
          <CameraCount enabled={Boolean(me)} />
          <LiveIndicator />
          <HealthBadge />
          <UserMenu />
          <NavLink
            to="/settings"
            aria-label="Settings"
            title="Settings"
            className={({ isActive }) =>
              `rounded p-1.5 ${isActive ? "bg-slate-800 text-white" : "text-slate-400 hover:text-white"}`
            }
          >
            <svg
              aria-hidden
              viewBox="0 0 24 24"
              className="h-5 w-5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
            </svg>
          </NavLink>
        </div>
      </header>
      <main className="mx-auto w-full max-w-7xl flex-1 p-6">
        <Outlet />
      </main>
      <Toaster />
    </div>
  );
}
