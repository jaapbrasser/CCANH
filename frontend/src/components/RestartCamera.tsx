import { useState } from "react";
import { createPortal } from "react-dom";
import { useRestartCamera } from "../api/cameras";
import { errorMessage } from "../api/client";
import { showToast } from "./toasts";
import { Alert, Button, Dialog } from "./ui";

/**
 * Restart (or try to turn on) a camera, after a confirmation (issue #33).
 * Every signed-in user may do this; the server audits it. `compact` is the
 * small variant for dashboard tiles.
 */
export function RestartCameraButton({
  cameraId,
  name,
  compact = false,
}: {
  cameraId: string;
  name: string;
  compact?: boolean;
}) {
  const restart = useRestartCamera();
  const [confirming, setConfirming] = useState(false);
  const open = (e: React.MouseEvent) => {
    e.stopPropagation();
    restart.reset();
    setConfirming(true);
  };
  const run = () =>
    restart.mutate(cameraId, {
      onSuccess: () => {
        setConfirming(false);
        showToast("info", `Restarting ${name}…`);
      },
    });

  const dialog = confirming && (
    <Dialog title={`Restart ${name}?`} onClose={() => setConfirming(false)}>
      <div className="flex flex-col gap-4 text-sm">
        <p>
          The camera reboots through its own interface. It goes offline and its live video stops for everyone until it
          is back, usually within a few minutes. A camera that can't be reached at all can't be restarted this way.
        </p>
        <p className="text-slate-400">This is recorded in the audit log under your name.</p>
        {restart.isError && <Alert>{errorMessage(restart.error)}</Alert>}
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setConfirming(false)}>
            Cancel
          </Button>
          <Button variant="danger" disabled={restart.isPending} onClick={run}>
            Restart camera
          </Button>
        </div>
      </div>
    </Dialog>
  );

  return (
    <>
      {compact ? (
        <button
          type="button"
          className="rounded px-1.5 text-red-300 hover:bg-slate-800 hover:text-white"
          aria-label={`Restart ${name}`}
          title="Restart camera"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={open}
        >
          ⟳
        </button>
      ) : (
        <Button variant="secondary" onClick={open}>
          Restart camera
        </Button>
      )}
      {/* Portal: tiles clip their content, and fullscreen shows only the grid. */}
      {dialog && createPortal(dialog, document.fullscreenElement ?? document.body)}
    </>
  );
}
