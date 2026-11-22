import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import type { CameraSummary } from "../../api/cameras";
import { moveTile, type Layout } from "../../api/dashboards";
import { LivePlayer } from "../../components/LivePlayer";
import { RestartCameraButton } from "../../components/RestartCamera";

const cellId = (position: number) => `cell-${position}`;

function place(position: number, cols: number) {
  return { gridRow: Math.floor(position / cols) + 1, gridColumn: (position % cols) + 1 };
}

/**
 * The dashboard grid. Camera tiles are direct children of the grid, keyed by
 * camera and placed with CSS grid coordinates, so moving or swapping a tile
 * never remounts its player: the video keeps playing and the shared stream
 * (keyed by camera and profile in the stream registry) is never restarted.
 */
export function DashboardGrid({
  layout,
  cameras,
  onChange,
  onExpand,
  onPick,
}: {
  layout: Layout;
  cameras: Map<string, CameraSummary>;
  onChange: (layout: Layout) => void;
  onExpand: (cameraId: string) => void;
  /** Open the camera picker for an empty cell. */
  onPick: (position: number) => void;
}) {
  const sensors = useSensors(
    // A small distance so clicks on a tile still work.
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor),
  );
  const cells = layout.rows * layout.cols;
  const occupied = new Set(layout.tiles.map((t) => t.position));

  const onDragEnd = (e: DragEndEvent) => {
    const from = layout.tiles.find((t) => t.camera_id === e.active.id)?.position;
    const over = e.over?.id;
    if (from === undefined || typeof over !== "string") return;
    const to = Number(over.slice("cell-".length));
    if (Number.isNaN(to) || to === from) return;
    onChange({ ...layout, tiles: moveTile(layout.tiles, from, to) });
  };

  // Stable DOM order (by camera id) so React never reorders the players.
  const tiles = [...layout.tiles].sort((a, b) => a.camera_id.localeCompare(b.camera_id));

  return (
    <DndContext sensors={sensors} onDragEnd={onDragEnd}>
      <div
        role="grid"
        aria-label={`Dashboard grid, ${layout.rows} by ${layout.cols}`}
        className="grid h-full min-h-0 gap-1"
        style={{
          gridTemplateColumns: `repeat(${layout.cols}, minmax(0, 1fr))`,
          gridTemplateRows: `repeat(${layout.rows}, minmax(0, 1fr))`,
        }}
      >
        {Array.from({ length: cells }, (_, p) => p)
          .filter((p) => !occupied.has(p))
          .map((p) => (
            <EmptyCell key={`empty-${p}`} position={p} cols={layout.cols} onPick={() => onPick(p)} />
          ))}
        {tiles.map((t) => (
          <CameraTile
            key={t.camera_id}
            position={t.position}
            cols={layout.cols}
            cameraId={t.camera_id}
            camera={cameras.get(t.camera_id)}
            onExpand={() => onExpand(t.camera_id)}
            onRemove={() => onChange({ ...layout, tiles: layout.tiles.filter((x) => x.camera_id !== t.camera_id) })}
          />
        ))}
      </div>
    </DndContext>
  );
}

function EmptyCell({ position, cols, onPick }: { position: number; cols: number; onPick: () => void }) {
  const { setNodeRef, isOver } = useDroppable({ id: cellId(position) });
  return (
    <div
      ref={setNodeRef}
      role="gridcell"
      style={place(position, cols)}
      className={`flex min-h-24 items-center justify-center rounded border border-dashed ${
        isOver ? "border-sky-400 bg-sky-400/10" : "border-slate-800"
      }`}
    >
      <button
        type="button"
        className="rounded px-3 py-1.5 text-sm text-slate-500 hover:bg-slate-800 hover:text-slate-200"
        aria-label={`Add camera to tile ${position + 1}`}
        onClick={onPick}
      >
        + Add camera
      </button>
    </div>
  );
}

function CameraTile({
  position,
  cols,
  cameraId,
  camera,
  onExpand,
  onRemove,
}: {
  position: number;
  cols: number;
  cameraId: string;
  camera: CameraSummary | undefined;
  onExpand: () => void;
  onRemove: () => void;
}) {
  const { setNodeRef: dragRef, listeners, attributes, transform, isDragging } = useDraggable({ id: cameraId });
  const { setNodeRef: dropRef, isOver } = useDroppable({ id: cellId(position) });
  const name = camera?.name ?? "Camera";
  const state = camera?.status.state;
  const style = {
    ...place(position, cols),
    transform: transform ? `translate(${transform.x}px, ${transform.y}px)` : undefined,
    zIndex: isDragging ? 10 : undefined,
  };
  return (
    <div
      ref={dropRef}
      role="gridcell"
      aria-label={`Tile ${position + 1}: ${name}`}
      style={style}
      className={`relative flex min-h-24 flex-col overflow-hidden rounded border bg-black ${
        isOver && !isDragging ? "border-sky-400" : "border-slate-800"
      } ${isDragging ? "opacity-80 shadow-2xl" : ""}`}
    >
      <div
        ref={dragRef}
        {...listeners}
        {...attributes}
        aria-label={`Move ${name}`}
        className="flex cursor-grab items-center gap-2 bg-slate-900/90 px-2 py-1 text-xs active:cursor-grabbing"
      >
        <span
          aria-hidden
          className={`h-2 w-2 shrink-0 rounded-full ${
            state === "online" ? "bg-emerald-400" : state === "offline" ? "bg-red-400" : "bg-slate-500"
          }`}
        />
        <span className="min-w-0 flex-1 truncate font-medium text-slate-200">{name}</span>
        {state === "offline" && <RestartCameraButton cameraId={cameraId} name={name} compact />}
        <button
          type="button"
          className="rounded px-1.5 text-slate-400 hover:bg-slate-800 hover:text-white"
          aria-label={`Expand ${name}`}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={onExpand}
        >
          ⤢
        </button>
        <button
          type="button"
          className="rounded px-1.5 text-slate-400 hover:bg-slate-800 hover:text-white"
          aria-label={`Remove ${name} from dashboard`}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={onRemove}
        >
          ×
        </button>
      </div>
      {/* Clicking the video expands it; the ⤢ button is the keyboard route. */}
      <div className="min-h-0 flex-1 cursor-zoom-in" onClick={onExpand}>
        <LivePlayer cameraId={cameraId} purpose="grid" label={name} pauseWhenHidden fill />
      </div>
    </div>
  );
}
