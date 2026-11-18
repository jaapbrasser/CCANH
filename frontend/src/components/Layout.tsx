import { NavLink, Outlet, useNavigate } from "react-router";
import { useLogout, useMe } from "../api/auth";
import { HealthBadge } from "./HealthBadge";

const linkClass = ({ isActive }: { isActive: boolean }) =>
  `rounded px-3 py-1.5 text-sm ${isActive ? "bg-slate-800 text-white" : "text-slate-400 hover:text-white"}`;

export function Layout() {
  const { data: me } = useMe();
  const logout = useLogout();
  const navigate = useNavigate();
  const isAdmin = me?.role === "admin";

  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100">
      <header className="flex items-center justify-between gap-4 border-b border-slate-800 px-6 py-3">
        <div className="flex items-center gap-6">
          <NavLink to="/" className="flex items-center gap-3">
            <img src="/favicon.svg" alt="" className="h-7 w-7" />
            <span className="text-lg font-semibold tracking-tight">Crane Camera Hub</span>
          </NavLink>
          <nav aria-label="Main" className="flex gap-1">
            <NavLink to="/" end className={linkClass}>
              Home
            </NavLink>
            {isAdmin && (
              <>
                <NavLink to="/users" className={linkClass}>
                  Users
                </NavLink>
                <NavLink to="/audit" className={linkClass}>
                  Audit log
                </NavLink>
              </>
            )}
          </nav>
        </div>
        <div className="flex items-center gap-4">
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
      <main className="mx-auto w-full max-w-6xl flex-1 p-6">
        <Outlet />
      </main>
    </div>
  );
}
