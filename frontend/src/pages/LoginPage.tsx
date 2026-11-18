import { useState, type FormEvent } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router";
import { useLogin, useMe, useSetupStatus } from "../api/auth";
import { useHealth } from "../api/health";
import { errorMessage } from "../api/client";
import { safeNext } from "../auth/session";
import { DemoBanner } from "../components/Layout";
import { Alert, Button, Field } from "../components/ui";

export function LoginPage() {
  const [params] = useSearchParams();
  const next = safeNext(params.get("next"));
  const navigate = useNavigate();
  const { data: me } = useMe();
  const setup = useSetupStatus();
  const login = useLogin();
  const demoAccounts = useHealth().data?.demo_accounts;
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");

  if (setup.data?.setup_required) return <Navigate to="/setup" replace />;
  if (me) return <Navigate to={next} replace />;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    login.mutate(
      { username, password },
      {
        onSuccess: () => {
          // Server-rendered pages (e.g. the API docs) need a full navigation.
          if (next.startsWith("/api/")) window.location.assign(next);
          else navigate(next, { replace: true });
        },
        onError: () => setPassword(""),
      },
    );
  };

  return (
    <AuthCard title="Sign in">
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {login.isError && <Alert>{errorMessage(login.error)}</Alert>}
        <Field
          label="Username"
          name="username"
          autoComplete="username"
          autoFocus
          required
          value={username}
          onChange={(e) => setUsername(e.target.value)}
        />
        <Field
          label="Password"
          name="password"
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <Button type="submit" disabled={login.isPending}>
          {login.isPending ? "Signing in…" : "Sign in"}
        </Button>
      </form>
      {demoAccounts && demoAccounts.length > 0 && (
        <div role="note" className="mt-6 rounded border border-amber-700 bg-amber-950/40 p-3 text-sm text-amber-100">
          <p className="mb-1 font-semibold">Demo mode accounts</p>
          <ul>
            {demoAccounts.map((a) => (
              <li key={a.username}>
                <code>{a.username}</code> / <code>{a.password}</code>{" "}
                <span className="text-amber-300/70">({a.role})</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </AuthCard>
  );
}

export function AuthCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100">
      <DemoBanner />
      <div className="flex flex-1 items-center justify-center p-4">
      <div className="w-full max-w-sm rounded-lg border border-slate-800 bg-slate-900/60 p-6">
        <div className="mb-6 flex items-center gap-3">
          <img src="/favicon.svg" alt="" className="h-8 w-8" />
          <div>
            <p className="text-xs uppercase tracking-wide text-slate-500">Crane Camera Hub</p>
            <h1 className="text-xl font-semibold">{title}</h1>
          </div>
        </div>
        {children}
      </div>
      </div>
    </div>
  );
}
