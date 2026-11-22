import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../api/client";
import type { PtzMove, PtzOutcome } from "../api/ptz";

const STOP: PtzMove = { pan: 0, tilt: 0, zoom: 0 };
/** The server stops a move that isn't repeated within 1.5 s (ADR 0013). */
const KEEPALIVE_MS = 400;
const SPEEDS = [
  { label: "Slow", value: 20 },
  { label: "Normal", value: 50 },
  { label: "Fast", value: 100 },
] as const;

/**
 * Press-and-hold movement: sends the move on press, repeats it while held and
 * sends a stop on release, when the pointer leaves, or when the window loses
 * focus. Reports cameras that didn't take a command.
 */
function useHold(send: (mv: PtzMove) => Promise<PtzOutcome>) {
  const timer = useRef<ReturnType<typeof setInterval> | undefined>(undefined);
  const held = useRef<PtzMove | null>(null);
  const sendRef = useRef(send);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    sendRef.current = send;
  }, [send]);

  // One command at a time, in order: a quick tap's stop must not overtake its move.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const fire = useCallback((mv: PtzMove) => {
    const run = () => sendRef.current(mv);
    const next = queue.current.then(run, run);
    queue.current = next.catch(() => undefined);
    next.then(
      (out) => {
        const failed = out.results.filter((r) => !r.ok);
        setProblem(
          failed.length === 0
            ? null
            : `${failed.length} of ${out.results.length} camera${out.results.length === 1 ? "" : "s"} didn't move: ${
                failed[0]!.message ?? failed[0]!.error_category
              }`,
        );
      },
      (e: unknown) => setProblem(errorMessage(e)),
    );
  }, []);

  const stop = useCallback(() => {
    clearInterval(timer.current);
    timer.current = undefined;
    if (held.current) {
      held.current = null;
      fire(STOP);
    }
  }, [fire]);

  const start = useCallback(
    (mv: PtzMove) => {
      stop();
      held.current = mv;
      fire(mv);
      timer.current = setInterval(() => held.current && fire(held.current), KEEPALIVE_MS);
    },
    [fire, stop],
  );

  useEffect(() => {
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("blur", stop);
      stop();
    };
  }, [stop]);

  return { start, stop, problem };
}

function HoldButton({
  label,
  symbol,
  mv,
  hold,
  className = "",
}: {
  label: string;
  symbol: string;
  mv: PtzMove;
  hold: ReturnType<typeof useHold>;
  className?: string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={`flex h-10 w-10 select-none items-center justify-center rounded border border-slate-700 bg-slate-900 text-lg text-slate-200 hover:bg-slate-800 active:bg-teal-700 ${className}`}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture?.(e.pointerId);
        hold.start(mv);
      }}
      onPointerUp={hold.stop}
      onPointerCancel={hold.stop}
      onLostPointerCapture={hold.stop}
      onKeyDown={(e) => {
        if ((e.key === " " || e.key === "Enter") && !e.repeat) {
          e.preventDefault();
          hold.start(mv);
        }
      }}
      onKeyUp={(e) => (e.key === " " || e.key === "Enter") && hold.stop()}
      onBlur={hold.stop}
      onContextMenu={(e) => e.preventDefault()}
    >
      {symbol}
    </button>
  );
}

/**
 * Pan/tilt/zoom controls (issue #31). Always placed beside the video, never
 * over it. `title` says what moves, e.g. "Moves 3 PTZ cameras".
 */
export function PtzPad({ title, send }: { title: string; send: (mv: PtzMove) => Promise<PtzOutcome> }) {
  const hold = useHold(send);
  const [speed, setSpeed] = useState<number>(50);
  const s = speed;
  return (
    <section
      aria-label="Camera controls"
      className="flex flex-col items-center gap-3 rounded border border-slate-800 bg-slate-950 p-3 text-sm"
    >
      <p className="text-center text-xs text-slate-400">{title}</p>
      <div className="grid grid-cols-3 gap-1">
        <span />
        <HoldButton label="Tilt up" symbol="▲" mv={{ pan: 0, tilt: s, zoom: 0 }} hold={hold} />
        <span />
        <HoldButton label="Pan left" symbol="◀" mv={{ pan: -s, tilt: 0, zoom: 0 }} hold={hold} />
        <span className="flex h-10 w-10 items-center justify-center text-xs text-slate-600" aria-hidden>
          ●
        </span>
        <HoldButton label="Pan right" symbol="▶" mv={{ pan: s, tilt: 0, zoom: 0 }} hold={hold} />
        <span />
        <HoldButton label="Tilt down" symbol="▼" mv={{ pan: 0, tilt: -s, zoom: 0 }} hold={hold} />
        <span />
      </div>
      <div className="flex gap-1">
        <HoldButton label="Zoom out" symbol="−" mv={{ pan: 0, tilt: 0, zoom: -s }} hold={hold} />
        <HoldButton label="Zoom in" symbol="+" mv={{ pan: 0, tilt: 0, zoom: s }} hold={hold} />
      </div>
      <label className="flex items-center gap-2 text-xs text-slate-400">
        Speed
        <select
          aria-label="Speed"
          className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-slate-100"
          value={speed}
          onChange={(e) => setSpeed(Number(e.target.value))}
        >
          {SPEEDS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      <p className="text-center text-xs text-slate-500">Hold a button to move; release to stop.</p>
      {hold.problem && (
        <p role="alert" className="text-center text-xs text-red-300">
          {hold.problem}
        </p>
      )}
    </section>
  );
}
