import { Navigate, NavLink, Outlet } from "react-router";
import { useMe } from "../api/auth";

/** Settings pages, in sidebar order. `admin` pages are hidden from viewers. */
const SETTINGS_PAGES = [
  { path: "general", label: "General", admin: true },
  { path: "activity", label: "Activity", admin: false },
  { path: "credentials", label: "Credentials", admin: true },
  { path: "users", label: "Users", admin: true },
  { path: "groups-tags", label: "Groups & tags", admin: true },
  { path: "streams", label: "Streams", admin: true },
  { path: "audit", label: "Audit log", admin: true },
] as const;

const linkClass = ({ isActive }: { isActive: boolean }) =>
  `block rounded px-3 py-2 text-sm ${isActive ? "bg-slate-800 text-white" : "text-slate-400 hover:bg-slate-900 hover:text-white"}`;

/** The settings area: a sidebar of the pages this role may open, and the page. */
export function SettingsLayout() {
  const { data: me } = useMe();
  const isAdmin = me?.role === "admin";
  const pages = SETTINGS_PAGES.filter((p) => isAdmin || !p.admin);
  return (
    <div className="flex flex-col gap-6 md:flex-row">
      <aside className="md:w-52 md:shrink-0">
        <p className="mb-2 px-3 text-xs font-semibold uppercase tracking-wide text-slate-500">Settings</p>
        <nav aria-label="Settings" className="flex flex-wrap gap-1 md:flex-col">
          {pages.map((p) => (
            <NavLink key={p.path} to={`/settings/${p.path}`} className={linkClass}>
              {p.label}
            </NavLink>
          ))}
          {isAdmin && (
            <a
              href="/api/docs/"
              className="block rounded px-3 py-2 text-sm text-slate-400 hover:bg-slate-900 hover:text-white"
            >
              API docs ↗
            </a>
          )}
        </nav>
      </aside>
      <div className="min-w-0 flex-1">
        <Outlet />
      </div>
    </div>
  );
}

/** `/settings`: the first page this role may open. */
export function SettingsIndex() {
  const { data: me } = useMe();
  return <Navigate to={me?.role === "admin" ? "/settings/general" : "/settings/activity"} replace />;
}
