import { useEffect, useSyncExternalStore } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { meKey } from "./auth";
import { cameraKeys, type CameraDetail, type CameraList } from "./cameras";
import { isTerminal, taskKeys, type Task } from "./tasks";
import { applyFound, applyProgress, discoveryKeys } from "./discovery";
import { orgKeys } from "./organization";
import { streamsKey } from "./streams";
import { showToast } from "../components/toasts";
import type { components } from "./schema";

export type ServerMessage = components["schemas"]["WsServerMessage"];
type ClientMessage = components["schemas"]["WsClientMessage"];

/** Close codes from the server (ADR 0007). */
export const CLOSE_SESSION_ENDED = 4401;

const TASK_LABELS: Record<string, string> = {
  test_connection: "Connection test",
  discovery: "Scan",
  bulk_cameras: "Bulk update",
  test_credentials: "Credential test",
};

/** Notify the user when one of their tasks finishes. */
function notifyTaskFinished(task: Task, myId: string | undefined) {
  if (!isTerminal(task.status) || task.created_by !== myId) return;
  const label = TASK_LABELS[task.kind] ?? "Task";
  if (task.status === "succeeded") {
    const ok = (task.result as { ok?: boolean } | null)?.ok;
    const found = (task.result as { found?: number } | null)?.found;
    const skipped = (task.result as { skipped?: { reason: string }[] } | null)?.skipped;
    if (ok === false) showToast("error", `${label} finished with problems.`);
    else if (task.kind === "bulk_cameras" && skipped?.length)
      showToast(
        "error",
        `${label} finished; ${skipped.length} camera${skipped.length === 1 ? "" : "s"} skipped: ${skipped[0]!.reason}`,
      );
    else if (task.kind === "discovery" && found !== undefined)
      showToast("success", `${label} finished: ${found} AXIS device${found === 1 ? "" : "s"} found.`);
    else showToast("success", `${label} finished.`);
  } else if (task.status === "failed") {
    showToast("error", `${label} failed: ${task.error_message ?? "unknown error"}`);
  } else {
    showToast("info", `${label} was cancelled.`);
  }
}

/** Per query client: last state seen per camera, so the top-bar counts refetch only on changes. */
const seen = new WeakMap<QueryClient, { states: Map<string, string>; timer?: ReturnType<typeof setTimeout> }>();
function tracker(qc: QueryClient) {
  let t = seen.get(qc);
  if (!t) seen.set(qc, (t = { states: new Map() }));
  return t;
}
/** Refetch the status counts once things settle (status events come in bursts). */
function refreshSummary(qc: QueryClient) {
  const t = tracker(qc);
  clearTimeout(t.timer);
  t.timer = setTimeout(() => qc.invalidateQueries({ queryKey: cameraKeys.summary() }), 500);
}

/** Apply one server message to the query cache. Exported for tests. */
export function applyLiveMessage(qc: QueryClient, msg: ServerMessage, myId?: string): void {
  switch (msg.type) {
    case "camera_status": {
      // Every check sends one; only a changed (or first seen) state matters.
      const states = tracker(qc).states;
      if (states.get(msg.camera_id) !== msg.status.state) {
        states.set(msg.camera_id, msg.status.state);
        refreshSummary(qc);
      }
      qc.setQueriesData<CameraList>(
        { queryKey: cameraKeys.lists() },
        (list) =>
          list && {
            ...list,
            items: list.items.map((c) => (c.id === msg.camera_id ? { ...c, status: msg.status } : c)),
          },
      );
      const prev = qc.getQueryData<CameraDetail>(cameraKeys.detail(msg.camera_id));
      if (prev) {
        qc.setQueryData<CameraDetail>(cameraKeys.detail(msg.camera_id), { ...prev, status: msg.status });
        if (prev.status.state !== msg.status.state) {
          qc.invalidateQueries({ queryKey: cameraKeys.history(msg.camera_id) });
        }
      }
      break;
    }
    case "camera_changed":
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
      qc.invalidateQueries({ queryKey: cameraKeys.detail(msg.camera.id) });
      // Monitoring may have been turned on or off.
      refreshSummary(qc);
      break;
    case "camera_deleted":
      tracker(qc).states.delete(msg.camera_id);
      refreshSummary(qc);
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
      qc.removeQueries({ queryKey: cameraKeys.detail(msg.camera_id) });
      break;
    case "task_updated": {
      const prev = qc.getQueryData<Task>(taskKeys.detail(msg.task.id));
      qc.setQueryData(taskKeys.detail(msg.task.id), msg.task);
      if (prev?.status !== msg.task.status) qc.invalidateQueries({ queryKey: taskKeys.lists() });
      if (!isTerminal(prev?.status)) notifyTaskFinished(msg.task, myId);
      if (msg.task.kind === "discovery" && prev?.status !== msg.task.status) {
        qc.invalidateQueries({ queryKey: discoveryKeys.all });
      }
      break;
    }
    case "discovery_found":
      applyFound(qc, msg.job_id, msg.result);
      break;
    case "discovery_progress":
      applyProgress(qc, msg.job_id, msg);
      break;
    case "organization_changed":
      qc.invalidateQueries({ queryKey: orgKeys.groups });
      break;
    case "streams_updated":
      qc.setQueryData(streamsKey, msg.streams);
      break;
    case "resync":
      qc.invalidateQueries({ queryKey: cameraKeys.all });
      qc.invalidateQueries({ queryKey: taskKeys.all });
      break;
    case "hello":
    case "subscribed":
    case "error":
      break;
  }
}

export type LiveState = "connecting" | "live" | "reconnecting";

let state: LiveState = "connecting";
const listeners = new Set<() => void>();
function setState(next: LiveState) {
  if (state === next) return;
  state = next;
  listeners.forEach((l) => l());
}

/** Connection state for the header indicator. */
export function useLiveState(): LiveState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

const MAX_BACKOFF_MS = 30_000;

/**
 * Keep one WebSocket open while signed in, subscribe to camera topics, patch the
 * query cache from events, and reconnect with exponential backoff. After a
 * reconnect the camera queries are refetched, since events may have been missed.
 */
export function useLiveUpdates(enabled: boolean, myId?: string, admin = false) {
  const qc = useQueryClient();
  useEffect(() => {
    if (!enabled || typeof WebSocket === "undefined") return;
    let socket: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempt = 0;
    let stopped = false;

    const connect = () => {
      const url = new URL("/api/v1/ws", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      socket = new WebSocket(url);
      socket.onopen = () => {
        const topics: ClientMessage["topics"] = ["camera_status", "cameras", "tasks"];
        if (admin) topics.push("streams", "discovery");
        const subscribe: ClientMessage = { type: "subscribe", topics };
        socket?.send(JSON.stringify(subscribe));
        if (attempt > 0) {
          qc.invalidateQueries({ queryKey: cameraKeys.all });
          qc.invalidateQueries({ queryKey: taskKeys.all });
        }
        attempt = 0;
        setState("live");
      };
      socket.onmessage = (event) => {
        try {
          applyLiveMessage(qc, JSON.parse(String(event.data)) as ServerMessage, myId);
        } catch {
          // Ignore malformed frames; the next resync or reconnect repairs state.
        }
      };
      socket.onclose = (event) => {
        if (stopped) return;
        setState("reconnecting");
        if (event.code === CLOSE_SESSION_ENDED) {
          // Re-check the session; the route guard sends the user to sign in.
          qc.invalidateQueries({ queryKey: meKey });
        }
        const delay = Math.min(MAX_BACKOFF_MS, 1000 * 2 ** attempt) * (0.8 + Math.random() * 0.4);
        attempt += 1;
        timer = setTimeout(connect, delay);
      };
    };

    connect();
    return () => {
      stopped = true;
      clearTimeout(timer);
      socket?.close();
      setState("connecting");
    };
  }, [enabled, qc, myId, admin]);
}
