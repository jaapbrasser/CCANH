import { NavLink, Outlet, useNavigate } from "react-router";
import { useLogout, useMe } from "../api/auth";
import { useHealth } from "../api/health";
import { useLiveState, useLiveUpdates } from "../api/live";
import { HealthBadge } from "./HealthBadge";
import { Toaster } from "./Toaster";

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
      <span className={`h-2 w-2 rounded-full ${live ? "bg-emerald-400" : "bg-amber-400"}`} />
      {live ? "Live" : state === "reconnecting" ? "Reconnecting…" : "Connecting…"}
    </span>
  );
}

export function DemoBanner() {
  const { data } = useHealth();
  if (data?.mode !== "demo") return null;
  return (
    <div role="note" className="bg-amber-500 px-4 py-1 text-center text-sm font-semibold text-slate-950">
      DEMO MODE · simulated cameras · data resets when the server restarts
    </div>
  );
}

export function Layout() {
  const { data: me } = useMe();
  const logout = useLogout();
  const navigate = useNavigate();
  const isAdmin = me?.role === "admin";
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
            <NavLink to="/cameras" className={linkClass}>
              Cameras
            </NavLink>
            <NavLink to="/activity" className={linkClass}>
              Activity
            </NavLink>
            {isAdmin && (
              <>
                <NavLink to="/credentials" className={linkClass}>
                  Credentials
                </NavLink>
                <NavLink to="/users" className={linkClass}>
                  Users
                </NavLink>
                <NavLink to="/streams" className={linkClass}>
                  Streams
                </NavLink>
                <NavLink to="/audit" className={linkClass}>
                  Audit log
                </NavLink>
                <NavLink to="/settings" className={linkClass}>
                  Settings
                </NavLink>
                <a href="/api/docs/" className="rounded px-3 py-1.5 text-sm text-slate-400 hover:text-white">
                  API docs
                </a>
              </>
            )}
          </nav>
        </div>
        <div className="flex items-center gap-4">
          <LiveIndicator />
          <HealthBadge />
          <NavLink to="/account" className={linkClass}>
            {me?.username}
            <span className="ml-2 text-xs uppercase text-slate-500">{me?.role}</span>
          </NavLink>
          <button
            type="button"
            className="text-sm text-slate-400 hover:text-white"
            onClick={() => logout.mutate(undefined, { onSettled: () => navigate("/login") })}
          >
            Sign out
          </button>
        </div>
      </header>
      <main className="mx-auto w-full max-w-7xl flex-1 p-6">
        <Outlet />
      </main>
      <Toaster />
    </div>
  );
}
