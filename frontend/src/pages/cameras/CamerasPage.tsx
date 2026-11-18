import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { useMe } from "../../api/auth";
import { useCameras, type CameraListQuery, type CameraState } from "../../api/cameras";
import { errorMessage } from "../../api/client";
import { useGroups, useTags } from "../../api/organization";
import { Chips, FavoriteButton, GroupSelect } from "../../components/organization";
import { StatusBadge } from "../../components/StatusBadge";
import { Alert, Button, PageHeader, Pagination } from "../../components/ui";
import { relativeTime } from "../../utils/format";
import { BulkBar } from "./BulkBar";
import { CameraFormDialog } from "./CameraFormDialog";

const PAGE_SIZE = 50;
type Sort = NonNullable<CameraListQuery["sort"]>;

const COLUMNS: { key: Sort | null; label: string }[] = [
  { key: "name", label: "Name" },
  { key: "host", label: "Address" },
  { key: "model", label: "Model" },
  { key: "state", label: "Status" },
  { key: "last_success_at", label: "Last contact" },
  { key: "last_change_at", label: "Last change" },
  { key: null, label: "Groups" },
  { key: null, label: "Tags" },
];

const filterClass = "rounded border border-slate-700 bg-slate-900 px-3 py-2";

/** The camera list; `favoritesOnly` makes it the Favorites view. */
export function CamerasPage({ favoritesOnly = false }: { favoritesOnly?: boolean }) {
  const { data: me } = useMe();
  const isAdmin = me?.role === "admin";
  const navigate = useNavigate();
  const groups = useGroups();
  const tags = useTags();
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<"" | CameraState>("");
  const [group, setGroup] = useState("");
  const [tag, setTag] = useState("");
  const [onlyFavorites, setOnlyFavorites] = useState(false);
  const [sort, setSort] = useState<Sort>("name");
  const [order, setOrder] = useState<"asc" | "desc">("asc");
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);

  const cameras = useCameras({
    page,
    page_size: PAGE_SIZE,
    q: search.trim() || undefined,
    status: status || undefined,
    group_id: group || undefined,
    tag_id: tag || undefined,
    favorites: favoritesOnly || onlyFavorites || undefined,
    sort,
    order,
  });
  const items = cameras.data?.items ?? [];
  const visible = new Set(items.map((c) => c.id));
  // Only act on selected cameras that are still listed.
  const selection = selected.filter((id) => visible.has(id));
  const allSelected = items.length > 0 && selection.length === items.length;

  const filtered = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    setPage(1);
  };
  const toggleSort = (key: Sort) => {
    if (key === sort) setOrder(order === "asc" ? "desc" : "asc");
    else {
      setSort(key);
      setOrder("asc");
    }
    setPage(1);
  };

  return (
    <div>
      <PageHeader
        title={favoritesOnly ? "Favorites" : "Cameras"}
        actions={isAdmin && !favoritesOnly && <Button onClick={() => setAdding(true)}>Add camera</Button>}
      />
      {favoritesOnly && (
        <p className="mb-4 text-sm text-slate-400">
          Your favorite cameras. Mark more with the ☆ in the camera list. Only you see your favorites.
        </p>
      )}

      <div className="mb-4 flex flex-wrap gap-3 text-sm">
        <input
          type="search"
          aria-label="Search cameras"
          placeholder="Search name, IP or model"
          className={`w-full max-w-xs ${filterClass}`}
          value={search}
          onChange={(e) => filtered(setSearch)(e.target.value)}
        />
        <select
          aria-label="Filter by status"
          className={filterClass}
          value={status}
          onChange={(e) => filtered(setStatus)(e.target.value as typeof status)}
        >
          <option value="">Any status</option>
          <option value="online">Online</option>
          <option value="offline">Offline</option>
          <option value="unknown">Unknown</option>
        </select>
        <GroupSelect
          label="Filter by group"
          groups={groups.data ?? []}
          value={group}
          onChange={filtered(setGroup)}
          emptyLabel="Any group"
        />
        <select aria-label="Filter by tag" className={filterClass} value={tag} onChange={(e) => filtered(setTag)(e.target.value)}>
          <option value="">Any tag</option>
          {tags.data?.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
        {!favoritesOnly && (
          <label className="flex items-center gap-2 text-slate-300">
            <input type="checkbox" checked={onlyFavorites} onChange={(e) => filtered(setOnlyFavorites)(e.target.checked)} />
            Favorites only
          </label>
        )}
      </div>

      {isAdmin && selection.length > 0 && (
        <BulkBar cameraIds={selection} groups={groups.data ?? []} tags={tags.data ?? []} onDone={() => setSelected([])} />
      )}

      {cameras.isError && <Alert>{errorMessage(cameras.error)}</Alert>}

      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              {isAdmin && (
                <th className="w-8 px-3 py-2">
                  <input
                    type="checkbox"
                    aria-label="Select all cameras on this page"
                    checked={allSelected}
                    onChange={(e) => setSelected(e.target.checked ? items.map((c) => c.id) : [])}
                  />
                </th>
              )}
              <th className="w-8 px-2 py-2">
                <span className="sr-only">Favorite</span>
              </th>
              {COLUMNS.map((col) => (
                <th
                  key={col.label}
                  className="px-4 py-2 font-medium"
                  aria-sort={col.key === sort ? (order === "asc" ? "ascending" : "descending") : undefined}
                >
                  {col.key ? (
                    <button type="button" className="hover:text-white" onClick={() => toggleSort(col.key!)}>
                      {col.label}
                      {col.key === sort && <span aria-hidden> {order === "asc" ? "▲" : "▼"}</span>}
                    </button>
                  ) : (
                    col.label
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {items.map((cam) => (
              <tr
                key={cam.id}
                className="cursor-pointer border-t border-slate-800 hover:bg-slate-900/60"
                onClick={() => navigate(`/cameras/${cam.id}`)}
              >
                {isAdmin && (
                  <td className="px-3 py-2" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      aria-label={`Select ${cam.name}`}
                      checked={selection.includes(cam.id)}
                      onChange={(e) =>
                        setSelected(e.target.checked ? [...selection, cam.id] : selection.filter((s) => s !== cam.id))
                      }
                    />
                  </td>
                )}
                <td className="px-2 py-2">
                  <FavoriteButton cameraId={cam.id} name={cam.name} />
                </td>
                <td className="px-4 py-2">
                  <Link to={`/cameras/${cam.id}`} className="font-medium hover:underline" onClick={(e) => e.stopPropagation()}>
                    {cam.name}
                  </Link>
                  {!cam.monitoring_enabled && <span className="ml-2 text-xs text-slate-500">not monitored</span>}
                </td>
                <td className="px-4 py-2 font-mono text-xs text-slate-300">
                  {cam.host}:{cam.port}
                </td>
                <td className="px-4 py-2 text-slate-300">{cam.model ?? "—"}</td>
                <td className="px-4 py-2">
                  <StatusBadge status={cam.status} />
                </td>
                <td className="px-4 py-2 text-slate-400">{relativeTime(cam.status.last_success_at)}</td>
                <td className="px-4 py-2 text-slate-400">{relativeTime(cam.status.last_change_at)}</td>
                <td className="px-4 py-2">
                  <Chips items={cam.groups} label={`Groups of ${cam.name}`} />
                </td>
                <td className="px-4 py-2">
                  <Chips items={cam.tags} label={`Tags of ${cam.name}`} />
                </td>
              </tr>
            ))}
            {cameras.data?.items.length === 0 && (
              <tr>
                <td colSpan={COLUMNS.length + (isAdmin ? 2 : 1)} className="px-4 py-8 text-center text-slate-500">
                  {favoritesOnly ? "No favorites yet." : "No cameras match."}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {cameras.data && (
        <div className="mt-4">
          <Pagination page={cameras.data.page} pageSize={cameras.data.page_size} total={cameras.data.total} onPage={setPage} />
        </div>
      )}

      {adding && <CameraFormDialog onClose={() => setAdding(false)} onSaved={(cam) => navigate(`/cameras/${cam.id}`)} />}
    </div>
  );
}
