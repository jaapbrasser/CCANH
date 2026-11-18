import { useEffect, useSyncExternalStore } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { meKey } from "./auth";
import { cameraKeys, type CameraDetail, type CameraList } from "./cameras";
import type { components } from "./schema";

export type ServerMessage = components["schemas"]["WsServerMessage"];
type ClientMessage = components["schemas"]["WsClientMessage"];

/** Close codes from the server (ADR 0007). */
export const CLOSE_SESSION_ENDED = 4401;

/** Apply one server message to the query cache. Exported for tests. */
export function applyLiveMessage(qc: QueryClient, msg: ServerMessage): void {
  switch (msg.type) {
    case "camera_status": {
      qc.setQueriesData<CameraList>({ queryKey: cameraKeys.lists() }, (list) =>
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
      break;
    case "camera_deleted":
      qc.invalidateQueries({ queryKey: cameraKeys.lists() });
      qc.removeQueries({ queryKey: cameraKeys.detail(msg.camera_id) });
      break;
    case "resync":
      qc.invalidateQueries({ queryKey: cameraKeys.all });
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
export function useLiveUpdates(enabled: boolean) {
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
        const subscribe: ClientMessage = { type: "subscribe", topics: ["camera_status", "cameras"] };
        socket?.send(JSON.stringify(subscribe));
        if (attempt > 0) qc.invalidateQueries({ queryKey: cameraKeys.all });
        attempt = 0;
        setState("live");
      };
      socket.onmessage = (event) => {
        try {
          applyLiveMessage(qc, JSON.parse(String(event.data)) as ServerMessage);
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
  }, [enabled, qc]);
}
