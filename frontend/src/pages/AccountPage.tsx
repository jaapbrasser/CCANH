import { useState, type FormEvent } from "react";
import { MIN_PASSWORD_LENGTH, useChangePassword, useMe } from "../api/auth";
import { errorMessage } from "../api/client";
import { Alert, Button, Field, PageHeader } from "../components/ui";

export function AccountPage() {
  const { data: me } = useMe();
  const change = useChangePassword();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setLocalError(null);
    setDone(false);
    if (next.length < MIN_PASSWORD_LENGTH) {
      setLocalError(`New password must be at least ${MIN_PASSWORD_LENGTH} characters.`);
      return;
    }
    if (next !== confirm) {
      setLocalError("New passwords don't match.");
      return;
    }
    change.mutate(
      { current_password: current, new_password: next },
      {
        onSuccess: () => {
          setDone(true);
          setCurrent("");
          setNext("");
          setConfirm("");
        },
      },
    );
  };

  return (
    <div className="max-w-md">
      <PageHeader title="Account" />
      <p className="mb-6 text-sm text-slate-400">
        Signed in as <strong className="text-slate-200">{me?.username}</strong> ({me?.role}).
      </p>
      <h2 className="mb-3 text-lg font-semibold">Change password</h2>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {(localError || change.isError) && <Alert>{localError ?? errorMessage(change.error)}</Alert>}
        {done && <Alert kind="success">Password changed. Your other sessions were signed out.</Alert>}
        <Field
          label="Current password"
          type="password"
          autoComplete="current-password"
          required
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
        />
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          hint={`At least ${MIN_PASSWORD_LENGTH} characters.`}
          required
          value={next}
          onChange={(e) => setNext(e.target.value)}
        />
        <Field
          label="Confirm new password"
          type="password"
          autoComplete="new-password"
          required
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
        />
        <Button type="submit" disabled={change.isPending}>
          Change password
        </Button>
      </form>
    </div>
  );
}
