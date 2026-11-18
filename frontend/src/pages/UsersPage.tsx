import { useState, type FormEvent } from "react";
import { MIN_PASSWORD_LENGTH, useMe, type Role } from "../api/auth";
import { errorMessage } from "../api/client";
import {
  useCreateUser,
  useDeleteUser,
  useResetPassword,
  useUpdateUser,
  useUsers,
  type User,
} from "../api/users";
import { Alert, Button, Dialog, Field, PageHeader, Pagination } from "../components/ui";

const PAGE_SIZE = 25;

export function UsersPage() {
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [creating, setCreating] = useState(false);
  const [resetting, setResetting] = useState<User | null>(null);
  const [deleting, setDeleting] = useState<User | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const { data: me } = useMe();
  const users = useUsers({ page, page_size: PAGE_SIZE, q: search || undefined, sort: "username" });
  const update = useUpdateUser();

  return (
    <div>
      <PageHeader title="Users" actions={<Button onClick={() => setCreating(true)}>New user</Button>} />

      <div className="mb-4 flex flex-col gap-3">
        <input
          type="search"
          aria-label="Search users"
          placeholder="Search by username"
          className="w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(1);
          }}
        />
        {notice && <Alert kind="success">{notice}</Alert>}
        {update.isError && <Alert>{errorMessage(update.error)}</Alert>}
        {users.isError && <Alert>{errorMessage(users.error)}</Alert>}
      </div>

      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="px-4 py-2 font-medium">Username</th>
              <th className="px-4 py-2 font-medium">Role</th>
              <th className="px-4 py-2 font-medium">Status</th>
              <th className="px-4 py-2 font-medium">Created</th>
              <th className="px-4 py-2 font-medium">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {users.data?.items.map((user) => {
              const self = user.id === me?.id;
              return (
                <tr key={user.id} className="border-t border-slate-800">
                  <td className="px-4 py-2">
                    {user.username}
                    {self && <span className="ml-2 text-xs text-slate-500">(you)</span>}
                  </td>
                  <td className="px-4 py-2">
                    <select
                      aria-label={`Role for ${user.username}`}
                      className="rounded border border-slate-700 bg-slate-900 px-2 py-1"
                      value={user.role}
                      disabled={update.isPending}
                      onChange={(e) =>
                        update.mutate({ id: user.id, role: e.target.value as Role })
                      }
                    >
                      <option value="viewer">Viewer</option>
                      <option value="admin">Admin</option>
                    </select>
                  </td>
                  <td className="px-4 py-2">
                    {user.disabled ? (
                      <span className="text-red-300">Disabled</span>
                    ) : (
                      <span className="text-emerald-300">Active</span>
                    )}
                  </td>
                  <td className="px-4 py-2 text-slate-400">
                    {new Date(user.created_at).toLocaleDateString()}
                  </td>
                  <td className="px-4 py-2">
                    <div className="flex justify-end gap-2">
                      <Button
                        variant="secondary"
                        disabled={update.isPending}
                        onClick={() => update.mutate({ id: user.id, disabled: !user.disabled })}
                      >
                        {user.disabled ? "Enable" : "Disable"}
                      </Button>
                      <Button variant="secondary" onClick={() => setResetting(user)}>
                        Reset password
                      </Button>
                      <Button variant="danger" disabled={self} onClick={() => setDeleting(user)}>
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {users.data && (
        <div className="mt-4">
          <Pagination
            page={users.data.page}
            pageSize={users.data.page_size}
            total={users.data.total}
            onPage={setPage}
          />
        </div>
      )}

      {creating && (
        <CreateUserDialog
          onClose={() => setCreating(false)}
          onCreated={(u) => {
            setCreating(false);
            setNotice(`Created ${u.username}.`);
          }}
        />
      )}
      {resetting && (
        <ResetPasswordDialog
          user={resetting}
          onClose={() => setResetting(null)}
          onDone={() => {
            setNotice(`Password reset for ${resetting.username}. Their sessions were signed out.`);
            setResetting(null);
          }}
        />
      )}
      {deleting && (
        <DeleteUserDialog
          user={deleting}
          onClose={() => setDeleting(null)}
          onDone={() => {
            setNotice(`Deleted ${deleting.username}.`);
            setDeleting(null);
          }}
        />
      )}
    </div>
  );
}

function CreateUserDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (u: User) => void }) {
  const create = useCreateUser();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("viewer");

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate({ username: username.trim(), password, role }, { onSuccess: onCreated });
  };

  return (
    <Dialog title="New user" onClose={onClose}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {create.isError && <Alert>{errorMessage(create.error)}</Alert>}
        <Field label="Username" required autoFocus value={username} onChange={(e) => setUsername(e.target.value)} />
        <Field
          label="Initial password"
          type="password"
          autoComplete="new-password"
          required
          minLength={MIN_PASSWORD_LENGTH}
          hint={`At least ${MIN_PASSWORD_LENGTH} characters.`}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <label className="flex flex-col gap-1 text-sm text-slate-300">
          Role
          <select
            className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
            value={role}
            onChange={(e) => setRole(e.target.value as Role)}
          >
            <option value="viewer">Viewer</option>
            <option value="admin">Admin</option>
          </select>
        </label>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={create.isPending}>
            Create
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function ResetPasswordDialog({ user, onClose, onDone }: { user: User; onClose: () => void; onDone: () => void }) {
  const reset = useResetPassword();
  const [password, setPassword] = useState("");
  return (
    <Dialog title={`Reset password for ${user.username}`} onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          reset.mutate({ id: user.id, new_password: password }, { onSuccess: onDone });
        }}
        className="flex flex-col gap-4"
      >
        {reset.isError && <Alert>{errorMessage(reset.error)}</Alert>}
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          required
          autoFocus
          minLength={MIN_PASSWORD_LENGTH}
          hint="All of this user's sessions will be signed out."
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={reset.isPending}>
            Reset password
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DeleteUserDialog({ user, onClose, onDone }: { user: User; onClose: () => void; onDone: () => void }) {
  const remove = useDeleteUser();
  return (
    <Dialog title={`Delete ${user.username}?`} onClose={onClose}>
      <p className="mb-4 text-sm text-slate-300">
        This permanently deletes the account and signs it out everywhere. It can't be undone.
      </p>
      {remove.isError && (
        <div className="mb-4">
          <Alert>{errorMessage(remove.error)}</Alert>
        </div>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="danger"
          disabled={remove.isPending}
          onClick={() => remove.mutate(user.id, { onSuccess: onDone })}
        >
          Delete user
        </Button>
      </div>
    </Dialog>
  );
}
