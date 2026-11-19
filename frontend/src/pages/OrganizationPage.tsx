import { useState, type FormEvent } from "react";
import { errorMessage } from "../api/client";
import {
  groupTree,
  subtreeIds,
  useCreateGroup,
  useCreateTag,
  useDeleteGroup,
  useDeleteTag,
  useGroups,
  useRenameTag,
  useTags,
  useUpdateGroup,
  type DeleteMode,
  type Group,
  type GroupNode,
  type Tag,
} from "../api/organization";
import { GroupSelect } from "../components/organization";
import { showToast } from "../components/toasts";
import { Alert, Button, Dialog, Field, PageHeader } from "../components/ui";
import { BulkSettingsDialog } from "../components/BulkSettingsDialog";
import { usePageTitle } from "../components/pageTitle";

/** Admin page: the group tree and the tag list (SPEC §9). */
export function OrganizationPage() {
  usePageTitle("Organization");
  return (
    <div>
      <PageHeader title="Groups & tags" />
      <p className="mb-6 max-w-3xl text-sm text-slate-400">
        Groups form a tree (a camera can be in several). Filtering by a group includes its subgroups. Tags are flat
        labels. Assign both on a camera's Edit dialog or in bulk from the camera list.
      </p>
      <div className="grid gap-8 lg:grid-cols-2">
        <GroupsSection />
        <TagsSection />
      </div>
    </div>
  );
}

function GroupsSection() {
  const groups = useGroups();
  const create = useCreateGroup();
  const [name, setName] = useState("");
  const [parent, setParent] = useState("");
  const [editing, setEditing] = useState<GroupNode | null>(null);
  const [deleting, setDeleting] = useState<GroupNode | null>(null);
  const [configuring, setConfiguring] = useState<GroupNode | null>(null);
  const list = groups.data ?? [];
  const tree = groupTree(list);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate(
      { name: name.trim(), parent_id: parent || null },
      {
        onSuccess: (g) => {
          showToast("success", `Group "${g.name}" created.`);
          setName("");
        },
      },
    );
  };

  return (
    <section aria-label="Groups" className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold">Groups</h2>
      <form onSubmit={onSubmit} aria-label="New group" className="flex flex-wrap items-end gap-2">
        <div className="min-w-48 flex-1">
          <Field label="New group" placeholder="Name" maxLength={64} value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <GroupSelect label="Parent group" groups={list} value={parent} onChange={setParent} emptyLabel="Top level" />
        <Button type="submit" disabled={!name.trim() || create.isPending}>
          Add group
        </Button>
      </form>
      {create.isError && <Alert>{errorMessage(create.error)}</Alert>}
      {groups.isError && <Alert>{errorMessage(groups.error)}</Alert>}
      <ul className="divide-y divide-slate-800 rounded border border-slate-800">
        {tree.map((g) => (
          <li key={g.id} className="flex items-center justify-between gap-2 px-3 py-2 text-sm">
            <span style={{ paddingLeft: `${g.depth * 1.25}rem` }}>
              <span className="font-medium">{g.name}</span>
              <span className="ml-2 text-xs text-slate-500">
                {g.camera_count} camera{g.camera_count === 1 ? "" : "s"}
              </span>
            </span>
            <span className="flex gap-2">
              <Button
                variant="secondary"
                aria-label={`Camera settings for ${g.path}`}
                onClick={() => setConfiguring(g)}
              >
                Camera settings
              </Button>
              <Button variant="secondary" aria-label={`Edit ${g.path}`} onClick={() => setEditing(g)}>
                Edit
              </Button>
              <Button variant="secondary" aria-label={`Delete ${g.path}`} onClick={() => setDeleting(g)}>
                Delete
              </Button>
            </span>
          </li>
        ))}
        {tree.length === 0 && <li className="px-3 py-6 text-center text-sm text-slate-500">No groups yet.</li>}
      </ul>
      {editing && <EditGroupDialog group={editing} groups={list} onClose={() => setEditing(null)} />}
      {deleting && <DeleteGroupDialog group={deleting} groups={list} onClose={() => setDeleting(null)} />}
      {configuring && (
        <BulkSettingsDialog
          selection={{ group_id: configuring.id, include_subgroups: true }}
          description={`cameras in ${configuring.path} and its subgroups`}
          onClose={() => setConfiguring(null)}
        />
      )}
    </section>
  );
}

function EditGroupDialog({ group, groups, onClose }: { group: GroupNode; groups: Group[]; onClose: () => void }) {
  const update = useUpdateGroup();
  const [name, setName] = useState(group.name);
  const [parent, setParent] = useState(group.parent_id ?? "");
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    update.mutate(
      { id: group.id, name: name.trim(), parent_id: parent || null },
      {
        onSuccess: () => {
          showToast("success", "Group saved.");
          onClose();
        },
      },
    );
  };
  return (
    <Dialog title={`Edit ${group.name}`} onClose={onClose}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        {update.isError && <Alert>{errorMessage(update.error)}</Alert>}
        <Field label="Name" required maxLength={64} value={name} onChange={(e) => setName(e.target.value)} />
        <label className="flex flex-col gap-1 text-sm text-slate-300">
          Parent group
          <GroupSelect
            label="Move under"
            groups={groups}
            value={parent}
            onChange={setParent}
            emptyLabel="Top level"
            // A group can't move into itself or its own subgroups.
            exclude={subtreeIds(groups, group.id)}
          />
        </label>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!name.trim() || update.isPending}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DeleteGroupDialog({ group, groups, onClose }: { group: GroupNode; groups: Group[]; onClose: () => void }) {
  const remove = useDeleteGroup();
  const [mode, setMode] = useState<DeleteMode>("move_up");
  const parent = groups.find((g) => g.id === group.parent_id);
  const subgroups = groups.filter((g) => g.parent_id === group.id).length;
  const where = parent ? `"${parent.name}"` : "the top level";
  return (
    <Dialog title={`Delete ${group.name}?`} onClose={onClose}>
      <div className="flex flex-col gap-4 text-sm">
        <p>
          It has {subgroups} subgroup{subgroups === 1 ? "" : "s"} and {group.camera_count} camera
          {group.camera_count === 1 ? "" : "s"} directly in it. Cameras are never deleted. What happens to them?
        </p>
        <fieldset className="flex flex-col gap-2">
          <legend className="sr-only">Subgroups and cameras</legend>
          <label className="flex items-start gap-2">
            <input type="radio" name="mode" checked={mode === "move_up"} onChange={() => setMode("move_up")} />
            <span>
              <span className="font-medium">Move up</span>: subgroups move to {where}
              {parent ? ` and cameras join "${parent.name}"` : " and cameras leave the group"}.
            </span>
          </label>
          <label className="flex items-start gap-2">
            <input type="radio" name="mode" checked={mode === "detach"} onChange={() => setMode("detach")} />
            <span>
              <span className="font-medium">Detach</span>: subgroups become top-level groups and cameras leave the
              group.
            </span>
          </label>
        </fieldset>
        {remove.isError && <Alert>{errorMessage(remove.error)}</Alert>}
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={remove.isPending}
            onClick={() =>
              remove.mutate(
                { id: group.id, mode },
                {
                  onSuccess: () => {
                    showToast("success", `Group "${group.name}" deleted.`);
                    onClose();
                  },
                },
              )
            }
          >
            Delete group
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function TagsSection() {
  const tags = useTags();
  const create = useCreateTag();
  const [name, setName] = useState("");
  const [editing, setEditing] = useState<Tag | null>(null);
  const [deleting, setDeleting] = useState<Tag | null>(null);
  const [configuring, setConfiguring] = useState<Tag | null>(null);
  return (
    <section aria-label="Tags" className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold">Tags</h2>
      <form
        aria-label="New tag"
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate(name.trim(), {
            onSuccess: (t) => {
              showToast("success", `Tag "${t.name}" created.`);
              setName("");
            },
          });
        }}
      >
        <div className="flex-1">
          <Field label="New tag" placeholder="Name" maxLength={64} value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <Button type="submit" disabled={!name.trim() || create.isPending}>
          Add tag
        </Button>
      </form>
      {create.isError && <Alert>{errorMessage(create.error)}</Alert>}
      <ul className="divide-y divide-slate-800 rounded border border-slate-800">
        {tags.data?.map((t) => (
          <li key={t.id} className="flex items-center justify-between gap-2 px-3 py-2 text-sm">
            <span>
              <span className="font-medium">{t.name}</span>
              <span className="ml-2 text-xs text-slate-500">
                {t.camera_count} camera{t.camera_count === 1 ? "" : "s"}
              </span>
            </span>
            <span className="flex gap-2">
              <Button variant="secondary" aria-label={`Camera settings for ${t.name}`} onClick={() => setConfiguring(t)}>
                Camera settings
              </Button>
              <Button variant="secondary" aria-label={`Rename ${t.name}`} onClick={() => setEditing(t)}>
                Rename
              </Button>
              <Button variant="secondary" aria-label={`Delete ${t.name}`} onClick={() => setDeleting(t)}>
                Delete
              </Button>
            </span>
          </li>
        ))}
        {tags.data?.length === 0 && <li className="px-3 py-6 text-center text-sm text-slate-500">No tags yet.</li>}
      </ul>
      {editing && <RenameTagDialog tag={editing} onClose={() => setEditing(null)} />}
      {deleting && <DeleteTagDialog tag={deleting} onClose={() => setDeleting(null)} />}
      {configuring && (
        <BulkSettingsDialog
          selection={{ tag_id: configuring.id }}
          description={`cameras tagged ${configuring.name}`}
          onClose={() => setConfiguring(null)}
        />
      )}
    </section>
  );
}

function RenameTagDialog({ tag, onClose }: { tag: Tag; onClose: () => void }) {
  const rename = useRenameTag();
  const [name, setName] = useState(tag.name);
  return (
    <Dialog title={`Rename ${tag.name}`} onClose={onClose}>
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          rename.mutate({ id: tag.id, name: name.trim() }, { onSuccess: onClose });
        }}
      >
        {rename.isError && <Alert>{errorMessage(rename.error)}</Alert>}
        <Field label="Name" required maxLength={64} value={name} onChange={(e) => setName(e.target.value)} />
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!name.trim() || rename.isPending}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DeleteTagDialog({ tag, onClose }: { tag: Tag; onClose: () => void }) {
  const remove = useDeleteTag();
  return (
    <Dialog title={`Delete ${tag.name}?`} onClose={onClose}>
      <div className="flex flex-col gap-4 text-sm">
        <p>
          The tag is removed from {tag.camera_count} camera{tag.camera_count === 1 ? "" : "s"}. The cameras are kept.
        </p>
        {remove.isError && <Alert>{errorMessage(remove.error)}</Alert>}
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={remove.isPending} onClick={() => remove.mutate(tag.id, { onSuccess: onClose })}>
            Delete tag
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
