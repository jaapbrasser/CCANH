import { useState, type FormEvent } from "react";
import { errorMessage } from "../api/client";
import {
  useCreateCredentialSet,
  useCredentialSets,
  useDeleteCredentialSet,
  useUpdateCredentialSet,
  type CredentialSet,
} from "../api/credentials";
import { Alert, Button, Dialog, Field, PageHeader } from "../components/ui";

export function CredentialSetsPage() {
  const sets = useCredentialSets();
  const remove = useDeleteCredentialSet();
  const [editing, setEditing] = useState<CredentialSet | "new" | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  return (
    <div>
      <PageHeader
        title="Camera credentials"
        actions={<Button onClick={() => setEditing("new")}>New credential set</Button>}
      />
      <p className="mb-4 max-w-2xl text-sm text-slate-400">
        Shared username/password pairs that many cameras can use. They are encrypted at rest and
        can't be viewed again after saving, only replaced.
      </p>
      <div className="mb-4 flex flex-col gap-2">
        {notice && <Alert kind="success">{notice}</Alert>}
        {remove.isError && <Alert>{errorMessage(remove.error)}</Alert>}
        {sets.isError && <Alert>{errorMessage(sets.error)}</Alert>}
      </div>
      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="px-4 py-2 font-medium">Name</th>
              <th className="px-4 py-2 font-medium">Cameras</th>
              <th className="px-4 py-2 font-medium">Updated</th>
              <th className="px-4 py-2 font-medium">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {sets.data?.map((set) => (
              <tr key={set.id} className="border-t border-slate-800">
                <td className="px-4 py-2">{set.name}</td>
                <td className="px-4 py-2 text-slate-300">{set.camera_count}</td>
                <td className="px-4 py-2 text-slate-400">{new Date(set.updated_at).toLocaleString()}</td>
                <td className="px-4 py-2">
                  <div className="flex justify-end gap-2">
                    <Button variant="secondary" onClick={() => setEditing(set)}>
                      Edit
                    </Button>
                    <Button
                      variant="danger"
                      disabled={remove.isPending}
                      onClick={() =>
                        remove.mutate(set.id, { onSuccess: () => setNotice(`Deleted ${set.name}.`) })
                      }
                    >
                      Delete
                    </Button>
                  </div>
                </td>
              </tr>
            ))}
            {sets.data?.length === 0 && (
              <tr>
                <td colSpan={4} className="px-4 py-8 text-center text-slate-500">
                  No credential sets yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {editing && (
        <CredentialSetDialog
          set={editing === "new" ? undefined : editing}
          onClose={() => setEditing(null)}
          onSaved={(name) => {
            setEditing(null);
            setNotice(`Saved ${name}.`);
          }}
        />
      )}
    </div>
  );
}

function CredentialSetDialog({
  set,
  onClose,
  onSaved,
}: {
  set?: CredentialSet;
  onClose: () => void;
  onSaved: (name: string) => void;
}) {
  const create = useCreateCredentialSet();
  const update = useUpdateCredentialSet();
  const mutation = set ? update : create;
  const [name, setName] = useState(set?.name ?? "");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const done = (s: CredentialSet) => onSaved(s.name);
    if (set) {
      const replace = username !== "" || password !== "";
      update.mutate(
        { id: set.id, name, ...(replace ? { username, password } : {}) },
        { onSuccess: done },
      );
    } else {
      create.mutate({ name, username, password }, { onSuccess: done });
    }
  };

  return (
    <Dialog title={set ? `Edit ${set.name}` : "New credential set"} onClose={onClose}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {mutation.isError && <Alert>{errorMessage(mutation.error)}</Alert>}
        <Field label="Name" required autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        <Field
          label="Camera username"
          autoComplete="off"
          required={!set}
          hint={set ? "Leave both fields empty to keep the current credentials." : undefined}
          value={username}
          onChange={(e) => setUsername(e.target.value)}
        />
        <Field
          label="Camera password"
          type="password"
          autoComplete="new-password"
          required={!set || username !== ""}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={mutation.isPending}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
