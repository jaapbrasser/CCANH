import { useEffect, useRef, useState } from "react";
import { attachPlayer, mseSupported } from "../streaming/player";
import { streams, type StreamState } from "../streaming/registry";

const OVERLAY: Partial<Record<StreamState | "unsupported", string>> = {
  connecting: "Connecting…",
  starting: "Starting stream…",
  reconnecting: "Reconnecting to camera…",
  failed: "Live video unavailable",
  stopped: "Stream stopped",
  unsupported: "Live video not supported",
};

/** Live video for a camera. Players of the same camera and profile share one stream. */
export function LivePlayer({
  cameraId,
  profile,
  purpose = "detail",
  label,
}: {
  cameraId: string;
  /** Stream profile; omit to let the server choose for `purpose`. */
  profile?: string;
  purpose?: "detail" | "grid";
  label: string;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const supported = mseSupported();
  const [state, setState] = useState<StreamState | "unsupported">(supported ? "connecting" : "unsupported");
  const [message, setMessage] = useState<string | null>(
    supported ? null : "This browser can't play live video (Media Source Extensions missing).",
  );
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    // Don't open a connection this browser can't play.
    if (!video.current || !supported) return;
    const conn = streams.connection(cameraId, profile, purpose);
    return attachPlayer(video.current, conn, {
      onState: (s, m) => {
        setState(s);
        setMessage(m);
      },
    });
  }, [cameraId, profile, purpose, attempt, supported]);

  const overlay = state === "running" || state === "draining" ? null : OVERLAY[state];
  const canRetry = state === "failed" || state === "stopped";

  return (
    <div className="relative aspect-video w-full overflow-hidden rounded bg-black">
      <video ref={video} aria-label={`Live video: ${label}`} className="h-full w-full object-contain" muted playsInline autoPlay />
      {overlay && (
        <div role="status" className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/60 p-4 text-center text-sm text-slate-200">
          <span className="font-medium">{overlay}</span>
          {message && <span className="max-w-md text-xs text-slate-400">{message}</span>}
          {canRetry && (
            <button
              type="button"
              className="rounded border border-slate-500 px-3 py-1 text-xs hover:bg-slate-800"
              onClick={() => setAttempt((n) => n + 1)}
            >
              Try again
            </button>
          )}
        </div>
      )}
    </div>
  );
}
