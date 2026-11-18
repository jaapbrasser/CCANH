import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { useMe } from "../../api/auth";
import { useCameras, type CameraListQuery, type CameraState } from "../../api/cameras";
import { errorMessage } from "../../api/client";
import { StatusBadge } from "../../components/StatusBadge";
import { Alert, Button, PageHeader, Pagination } from "../../components/ui";
import { relativeTime } from "../../utils/format";
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
  { key: null, label: "Response" },
];

export function CamerasPage() {
  const { data: me } = useMe();
  const navigate = useNavigate();
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<"" | CameraState>("");
  const [sort, setSort] = useState<Sort>("name");
  const [order, setOrder] = useState<"asc" | "desc">("asc");
  const [adding, setAdding] = useState(false);

  const cameras = useCameras({
    page,
    page_size: PAGE_SIZE,
    q: search.trim() || undefined,
    status: status || undefined,
    sort,
    order,
  });

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
        title="Cameras"
        actions={me?.role === "admin" && <Button onClick={() => setAdding(true)}>Add camera</Button>}
      />

      <div className="mb-4 flex flex-wrap gap-3 text-sm">
        <input
          type="search"
          aria-label="Search cameras"
          placeholder="Search name, IP or model"
          className="w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(1);
          }}
        />
        <select
          aria-label="Filter by status"
          className="rounded border border-slate-700 bg-slate-900 px-3 py-2"
          value={status}
          onChange={(e) => {
            setStatus(e.target.value as typeof status);
            setPage(1);
          }}
        >
          <option value="">Any status</option>
          <option value="online">Online</option>
          <option value="offline">Offline</option>
          <option value="unknown">Unknown</option>
        </select>
      </div>

      {cameras.isError && <Alert>{errorMessage(cameras.error)}</Alert>}

      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
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
            {cameras.data?.items.map((cam) => (
              <tr
                key={cam.id}
                className="cursor-pointer border-t border-slate-800 hover:bg-slate-900/60"
                onClick={() => navigate(`/cameras/${cam.id}`)}
              >
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
                <td className="px-4 py-2 text-slate-400">
                  {cam.status.response_time_ms != null ? `${cam.status.response_time_ms} ms` : "—"}
                </td>
              </tr>
            ))}
            {cameras.data?.items.length === 0 && (
              <tr>
                <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-slate-500">
                  No cameras match.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {cameras.data && (
        <div className="mt-4">
          <Pagination
            page={cameras.data.page}
            pageSize={cameras.data.page_size}
            total={cameras.data.total}
            onPage={setPage}
          />
        </div>
      )}

      {adding && (
        <CameraFormDialog
          onClose={() => setAdding(false)}
          onSaved={(cam) => navigate(`/cameras/${cam.id}`)}
        />
      )}
    </div>
  );
}
