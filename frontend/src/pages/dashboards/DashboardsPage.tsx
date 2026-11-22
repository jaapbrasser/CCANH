import { useEffect, useRef, useState, type FormEvent } from "react";
import { useNavigate, useParams } from "react-router";
import { useCameras } from "../../api/cameras";
import { errorMessage } from "../../api/client";
import {
  fitTiles,
  PRESETS,
  useCreateDashboard,
  useDashboards,
  useDeleteDashboard,
  useLayoutAutosave,
  useUpdateDashboard,
  type Dashboard,
} from "../../api/dashboards";
import { showToast } from "../../components/toasts";
import { Alert, Button, Dialog, Field } from "../../components/ui";
import { CameraPicker } from "./CameraPicker";
import { DashboardGrid } from "./DashboardGrid";
import { ExpandedCamera } from "./ExpandedCamera";
import { usePageTitle } from "../../components/pageTitle";

const selectClass = "rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm";

/** The signed-in user's dashboards (SPEC §8). `/dashboards/:id` picks one. */
export function DashboardsPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const dashboards = useDashboards();
  const [creating, setCreating] = useState(false);
  const create = useCreateDashboard();
  const list = dashboards.data ?? [];
  const current = list.find((d) => d.id === id) ?? list.find((d) => d.is_default) ?? list[0];
  usePageTitle(current ? "Dashboard" : "Dashboards", current?.name);

  if (dashboards.isPending) return <p className="text-sm text-slate-400">Loading dashboards…</p>;
  if (dashboards.isError) return <Alert>{errorMessage(dashboards.error)}</Alert>;

  return (
    <div className="flex h-[calc(100vh-8rem)] min-h-96 flex-col gap-3">
      {current ? (
        // Re-key per dashboard so its layout state starts fresh.
        <DashboardView
          key={current.id}
          dashboard={current}
          all={list}
          onSwitch={(d) => navigate(`/dashboards/${d}`)}
          onNew={() => setCreating(true)}
        />
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-slate-400">
          <h1 className="text-xl font-semibold text-slate-100">Dashboards</h1>
          <p>Put several cameras on one screen. Your dashboards are private to you.</p>
          <Button onClick={() => setCreating(true)}>Create a dashboard</Button>
        </div>
      )}
      {creating && (
        <NameDialog
          title="New dashboard"
          initial=""
          submitLabel="Create"
          onClose={() => setCreating(false)}
          error={create.error}
          pending={create.isPending}
          onSubmit={(name) =>
            create.mutate(
              { name, rows: 2, cols: 2 },
              {
                onSuccess: (d) => {
                  setCreating(false);
                  navigate(`/dashboards/${d.id}`);
                },
              },
            )
          }
        />
      )}
    </div>
  );
}

function DashboardView({
  dashboard,
  all,
  onSwitch,
  onNew,
}: {
  dashboard: Dashboard;
  all: Dashboard[];
  onSwitch: (id: string) => void;
  onNew: () => void;
}) {
  const navigate = useNavigate();
  const { layout, change, saving } = useLayoutAutosave(dashboard, (e) =>
    showToast("error", `Layout not saved: ${errorMessage(e)}. Your last change was undone.`),
  );
  // Names and live status for tiles (the list cache gets live updates).
  const cameras = useCameras({ page_size: 200 });
  const byId = new Map(cameras.data?.items.map((c) => [c.id, c]) ?? []);
  const update = useUpdateDashboard();
  const remove = useDeleteDashboard();
  const [picking, setPicking] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const gridBox = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === gridBox.current);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  const preset = PRESETS.find((p) => p.rows === layout.rows && p.cols === layout.cols);
  const resize = (rows: number, cols: number) => {
    const tiles = fitTiles(layout.tiles, layout.cols, rows, cols);
    if (!tiles) {
      showToast("error", `A ${rows}×${cols} grid has room for ${rows * cols} cameras; remove some first.`);
      return;
    }
    change({ rows, cols, tiles });
  };
  const onDashboard = new Set(layout.tiles.map((t) => t.camera_id));

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <h1 className="sr-only">Dashboard: {dashboard.name}</h1>
        <select
          aria-label="Dashboard"
          className={`${selectClass} font-medium`}
          value={dashboard.id}
          onChange={(e) => onSwitch(e.target.value)}
        >
          {all.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
              {d.is_default ? " (default)" : ""}
            </option>
          ))}
        </select>
        <select
          aria-label="Grid size"
          className={selectClass}
          value={preset?.label ?? ""}
          onChange={(e) => {
            const p = PRESETS.find((x) => x.label === e.target.value);
            if (p) resize(p.rows, p.cols);
          }}
        >
          {!preset && (
            <option value="" disabled>
              {layout.rows}×{layout.cols}
            </option>
          )}
          {PRESETS.map((p) => (
            <option key={p.label} value={p.label}>
              {p.label}
            </option>
          ))}
        </select>
        <span role="status" className="text-xs text-slate-500">
          {saving ? "Saving…" : "Saved"}
        </span>
        <span className="ml-auto flex flex-wrap gap-2">
          <Button variant="secondary" onClick={() => void gridBox.current?.requestFullscreen?.()}>
            Fullscreen
          </Button>
          {!dashboard.is_default && (
            <Button
              variant="secondary"
              disabled={update.isPending}
              onClick={() =>
                update.mutate(
                  { id: dashboard.id, make_default: true },
                  { onSuccess: () => showToast("success", `"${dashboard.name}" is now your default dashboard.`) },
                )
              }
            >
              Make default
            </Button>
          )}
          <Button variant="secondary" onClick={() => setRenaming(true)}>
            Rename
          </Button>
          <Button variant="secondary" onClick={() => setConfirmDelete(true)}>
            Delete
          </Button>
          <Button onClick={onNew}>New dashboard</Button>
        </span>
      </div>

      <div ref={gridBox} className={`min-h-0 flex-1 ${fullscreen ? "bg-black p-1" : ""}`}>
        {fullscreen && (
          <button
            type="button"
            className="absolute right-3 top-3 z-10 rounded bg-slate-900/80 px-3 py-1 text-sm"
            onClick={() => void document.exitFullscreen()}
          >
            Exit fullscreen
          </button>
        )}
        <DashboardGrid layout={layout} cameras={byId} onChange={change} onExpand={setExpanded} onPick={setPicking} />
      </div>

      {picking !== null && (
        <CameraPicker
          onDashboard={onDashboard}
          onClose={() => setPicking(null)}
          onPick={(cameraId) => {
            change({
              ...layout,
              tiles: [...layout.tiles, { position: picking, camera_id: cameraId }].sort(
                (a, b) => a.position - b.position,
              ),
            });
            setPicking(null);
          }}
        />
      )}
      {expanded && <ExpandedCamera cameraId={expanded} onClose={() => setExpanded(null)} />}
      {renaming && (
        <NameDialog
          title="Rename dashboard"
          initial={dashboard.name}
          submitLabel="Save"
          onClose={() => setRenaming(false)}
          error={update.error}
          pending={update.isPending}
          onSubmit={(name) => update.mutate({ id: dashboard.id, name }, { onSuccess: () => setRenaming(false) })}
        />
      )}
      {confirmDelete && (
        <Dialog title={`Delete "${dashboard.name}"?`} onClose={() => setConfirmDelete(false)}>
          <div className="flex flex-col gap-4 text-sm">
            <p>The dashboard and its layout are removed. Cameras aren't affected.</p>
            {remove.isError && <Alert>{errorMessage(remove.error)}</Alert>}
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => setConfirmDelete(false)}>
                Cancel
              </Button>
              <Button
                variant="danger"
                disabled={remove.isPending}
                onClick={() => remove.mutate(dashboard.id, { onSuccess: () => navigate("/dashboards") })}
              >
                Delete dashboard
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </>
  );
}

function NameDialog({
  title,
  initial,
  submitLabel,
  onClose,
  onSubmit,
  error,
  pending,
}: {
  title: string;
  initial: string;
  submitLabel: string;
  onClose: () => void;
  onSubmit: (name: string) => void;
  error: unknown;
  pending: boolean;
}) {
  const [name, setName] = useState(initial);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit(name.trim());
  };
  return (
    <Dialog title={title} onClose={onClose}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        {Boolean(error) && <Alert>{errorMessage(error)}</Alert>}
        <Field label="Name" required autoFocus maxLength={64} value={name} onChange={(e) => setName(e.target.value)} />
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!name.trim() || pending}>
            {submitLabel}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
