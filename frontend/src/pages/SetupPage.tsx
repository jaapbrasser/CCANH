import { useState, type FormEvent } from "react";
import { Navigate, useNavigate } from "react-router";
import { MIN_PASSWORD_LENGTH, useCompleteSetup, useMe, useSetupStatus } from "../api/auth";
import { errorMessage } from "../api/client";
import { Alert, Button, Field } from "../components/ui";
import { AuthCard } from "./LoginPage";

export function SetupPage() {
  const status = useSetupStatus();
  const { data: me } = useMe();
  const setup = useCompleteSetup();
  const navigate = useNavigate();
  const [token, setToken] = useState("");
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  if (status.isPending) return null;
  if (status.data && !status.data.setup_required) {
    return <Navigate to={me ? "/" : "/login"} replace />;
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setLocalError(null);
    if (password.length < MIN_PASSWORD_LENGTH) {
      setLocalError(`Password must be at least ${MIN_PASSWORD_LENGTH} characters.`);
      return;
    }
    if (password !== confirm) {
      setLocalError("Passwords don't match.");
      return;
    }
    setup.mutate(
      { token: token.trim(), username: username.trim(), password },
      { onSuccess: () => navigate("/", { replace: true }) },
    );
  };

  return (
    <AuthCard title="First-run setup">
      <p className="mb-4 text-sm text-slate-400">
        Create the first admin account. The one-time setup token is printed in the server
        console and log when the server starts with no users.
      </p>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {(localError || setup.isError) && <Alert>{localError ?? errorMessage(setup.error)}</Alert>}
        <Field
          label="Setup token"
          name="token"
          autoComplete="off"
          required
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
        <Field
          label="Admin username"
          name="username"
          autoComplete="username"
          required
          value={username}
          onChange={(e) => setUsername(e.target.value)}
        />
        <Field
          label="Password"
          name="password"
          type="password"
          autoComplete="new-password"
          hint={`At least ${MIN_PASSWORD_LENGTH} characters.`}
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <Field
          label="Confirm password"
          name="confirm"
          type="password"
          autoComplete="new-password"
          required
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
        />
        <Button type="submit" disabled={setup.isPending}>
          {setup.isPending ? "Creating…" : "Create admin"}
        </Button>
      </form>
    </AuthCard>
  );
}
