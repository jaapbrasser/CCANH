import type { ReactNode } from "react";
import { Navigate, useLocation } from "react-router";
import { useMe } from "../api/auth";

function Loading() {
  return <div className="p-6 text-slate-400">Loading…</div>;
}

/** Only render children for signed-in users; otherwise go to the login page. */
export function RequireAuth({ children }: { children: ReactNode }) {
  const { data: me, isPending } = useMe();
  const location = useLocation();
  if (isPending) return <Loading />;
  if (!me) {
    const next = encodeURIComponent(location.pathname + location.search);
    return <Navigate to={`/login?next=${next}`} replace />;
  }
  return children;
}

/** Only render children for admins. Viewers see a short explanation. */
export function RequireAdmin({ children }: { children: ReactNode }) {
  const { data: me } = useMe();
  if (me?.role !== "admin") {
    return (
      <div className="p-6">
        <h1 className="text-xl font-semibold">Admins only</h1>
        <p className="mt-2 text-slate-400">You need the admin role to view this page.</p>
      </div>
    );
  }
  return children;
}
