import { useState } from "react";
import { errorMessage } from "../../api/client";
import { useBulkCameras, type BulkAction, type Group, type Tag } from "../../api/organization";
import { GroupSelect } from "../../components/organization";
import { showToast } from "../../components/toasts";
import { Alert, Button, Dialog } from "../../components/ui";

type Choice = "add_tags" | "remove_tags" | "add_groups" | "remove_groups" | "monitoring_on" | "monitoring_off" | "delete";

const LABELS: Record<Choice, string> = {
  add_tags: "Add tag",
  remove_tags: "Remove tag",
  add_groups: "Add to group",
  remove_groups: "Remove from group",
  monitoring_on: "Turn monitoring on",
  monitoring_off: "Turn monitoring off",
  delete: "Delete cameras",
};

const selectClass = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-100";

/** Actions for the selected cameras. Runs as a background task. */
export function BulkBar({
  cameraIds,
  groups,
  tags,
  onDone,
}: {
  cameraIds: string[];
  groups: Group[];
  tags: Tag[];
  onDone: () => void;
}) {
  const bulk = useBulkCameras();
  const [choice, setChoice] = useState<Choice>("add_tags");
  const [target, setTarget] = useState("");
  const [confirming, setConfirming] = useState(false);
  const needsTag = choice === "add_tags" || choice === "remove_tags";
  const needsGroup = choice === "add_groups" || choice === "remove_groups";
  const n = cameraIds.length;

  const action = (): BulkAction | null => {
    switch (choice) {
      case "add_tags":
      case "remove_tags":
        return target ? { type: choice, tag_ids: [target] } : null;
      case "add_groups":
      case "remove_groups":
        return target ? { type: choice, group_ids: [target] } : null;
      case "monitoring_on":
        return { type: "set_monitoring", enabled: true };
      case "monitoring_off":
        return { type: "set_monitoring", enabled: false };
      case "delete":
        return { type: "delete" };
    }
  };

  const run = () => {
    const a = action();
    if (!a) return;
    bulk.mutate(
      { camera_ids: cameraIds, action: a },
      {
        onSuccess: () => {
          showToast("success", `${LABELS[choice]}: started for ${n} camera${n === 1 ? "" : "s"}.`);
          setConfirming(false);
          onDone();
        },
      },
    );
  };

  return (
    <section
      aria-label="Bulk actions"
      className="mb-4 flex flex-wrap items-center gap-3 rounded border border-sky-900 bg-sky-950/40 p-3 text-sm"
    >
      <span className="font-medium">
        {n} selected
      </span>
      <select
        aria-label="Bulk action"
        className={selectClass}
        value={choice}
        onChange={(e) => {
          setChoice(e.target.value as Choice);
          setTarget("");
        }}
      >
        {(Object.keys(LABELS) as Choice[]).map((c) => (
          <option key={c} value={c}>
            {LABELS[c]}
          </option>
        ))}
      </select>
      {needsTag && (
        <select aria-label="Tag" className={selectClass} value={target} onChange={(e) => setTarget(e.target.value)}>
          <option value="">Choose a tag…</option>
          {tags.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      )}
      {needsGroup && (
        <GroupSelect label="Group" groups={groups} value={target} onChange={setTarget} emptyLabel="Choose a group…" />
      )}
      <Button
        variant={choice === "delete" ? "danger" : "primary"}
        disabled={bulk.isPending || ((needsTag || needsGroup) && !target)}
        onClick={() => (choice === "delete" ? setConfirming(true) : run())}
      >
        Apply
      </Button>
      <Button variant="secondary" onClick={onDone}>
        Clear selection
      </Button>
      {bulk.isError && !confirming && <Alert>{errorMessage(bulk.error)}</Alert>}

      {confirming && (
        <Dialog title={`Delete ${n} camera${n === 1 ? "" : "s"}?`} onClose={() => setConfirming(false)}>
          <div className="flex flex-col gap-4 text-sm">
            <p>
              This removes the selected cameras with their status history, and stops their live streams. It can't be
              undone.
            </p>
            {bulk.isError && <Alert>{errorMessage(bulk.error)}</Alert>}
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button variant="danger" disabled={bulk.isPending} onClick={run}>
                Delete {n} camera{n === 1 ? "" : "s"}
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </section>
  );
}
