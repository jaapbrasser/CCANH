import { useState } from "react";
import { useCameras } from "../../api/cameras";
import { errorMessage } from "../../api/client";
import { useGroups, useTags } from "../../api/organization";
import { GroupSelect } from "../../components/organization";
import { StatusBadge } from "../../components/StatusBadge";
import { Alert, Button, Dialog } from "../../components/ui";

const inputClass = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm";

/** Pick a camera for a tile (SPEC §8.3). Cameras already on the dashboard are marked. */
export function CameraPicker({
  onDashboard,
  onPick,
  onClose,
}: {
  onDashboard: Set<string>;
  onPick: (cameraId: string) => void;
  onClose: () => void;
}) {
  const groups = useGroups();
  const tags = useTags();
  const [q, setQ] = useState("");
  const [group, setGroup] = useState("");
  const [tag, setTag] = useState("");
  const [favorites, setFavorites] = useState(false);
  const cameras = useCameras({
    page_size: 50,
    q: q.trim() || undefined,
    group_id: group || undefined,
    tag_id: tag || undefined,
    favorites: favorites || undefined,
  });

  return (
    <Dialog title="Add a camera" onClose={onClose}>
      <div className="flex flex-col gap-3">
        <input
          type="search"
          autoFocus
          aria-label="Search cameras"
          placeholder="Search name or IP"
          className={inputClass}
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <div className="flex flex-wrap gap-2">
          <GroupSelect label="Filter by group" groups={groups.data ?? []} value={group} onChange={setGroup} emptyLabel="Cranes" />
          <select aria-label="Filter by tag" className={inputClass} value={tag} onChange={(e) => setTag(e.target.value)}>
            <option value="">Any tag</option>
            {tags.data?.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-2 text-sm text-slate-300">
            <input type="checkbox" checked={favorites} onChange={(e) => setFavorites(e.target.checked)} />
            Favorites
          </label>
        </div>
        {cameras.isError && <Alert>{errorMessage(cameras.error)}</Alert>}
        <ul aria-label="Cameras" className="max-h-80 divide-y divide-slate-800 overflow-y-auto rounded border border-slate-800">
          {cameras.data?.items.map((c) => {
            const there = onDashboard.has(c.id);
            return (
              <li key={c.id} className="flex items-center justify-between gap-2 px-3 py-2 text-sm">
                <span className="min-w-0">
                  <span className="block truncate font-medium">{c.name}</span>
                  <span className="flex items-center gap-2 text-xs text-slate-500">
                    {c.host} <StatusBadge status={c.status} />
                  </span>
                </span>
                {there ? (
                  <span className="shrink-0 text-xs text-slate-500">On this dashboard</span>
                ) : (
                  <Button variant="secondary" aria-label={`Add ${c.name}`} onClick={() => onPick(c.id)}>
                    Add
                  </Button>
                )}
              </li>
            );
          })}
          {cameras.data?.items.length === 0 && <li className="px-3 py-6 text-center text-sm text-slate-500">No cameras match.</li>}
        </ul>
      </div>
    </Dialog>
  );
}
