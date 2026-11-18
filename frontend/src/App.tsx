import { HealthBadge } from "./components/HealthBadge";

export default function App() {
  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100">
      <header className="flex items-center justify-between border-b border-slate-800 px-6 py-3">
        <div className="flex items-center gap-3">
          <img src="/favicon.svg" alt="" className="h-7 w-7" />
          <h1 className="text-lg font-semibold tracking-tight">Crane Camera Hub</h1>
        </div>
        <HealthBadge />
      </header>
      <main className="flex flex-1 items-center justify-center p-6">
        <div className="max-w-md text-center">
          <h2 className="text-2xl font-semibold">Welcome</h2>
          <p className="mt-2 text-slate-400">
            The platform foundation is running. Cameras, live view and dashboards arrive in
            later phases.
          </p>
          <a
            href="/api/docs/"
            className="mt-6 inline-block rounded bg-amber-500 px-4 py-2 font-medium text-slate-950 hover:bg-amber-400"
          >
            Open API docs
          </a>
        </div>
      </main>
    </div>
  );
}
