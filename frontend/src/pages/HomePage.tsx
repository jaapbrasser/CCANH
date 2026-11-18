import { useMe } from "../api/auth";

export function HomePage() {
  const { data: me } = useMe();
  return (
    <div className="max-w-xl">
      <h1 className="text-2xl font-semibold">Welcome, {me?.username}</h1>
      <p className="mt-2 text-slate-400">
        Cameras, live view and dashboards arrive in later phases.
      </p>
      {me?.role === "admin" && (
        <a
          href="/api/docs/"
          className="mt-6 inline-block rounded bg-amber-500 px-4 py-2 font-medium text-slate-950 hover:bg-amber-400"
        >
          Open API docs
        </a>
      )}
    </div>
  );
}
