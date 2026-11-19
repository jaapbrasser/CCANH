/**
 * Shared live-stream connections, keyed by (camera, profile).
 *
 * Several players showing the same camera and profile share one media
 * WebSocket, and the server shares one upstream camera connection across all
 * of them. Connections are reference-counted and linger briefly after the last
 * player detaches, so re-rendering or rearranging the UI never restarts a
 * stream (SPEC §8.2).
 *
 * Protocol (ADR 0002): text {"type":"init",codec,width,height} then a binary
 * init segment; binary media = 1 flag byte (bit 0 keyframe) + fMP4 fragment;
 * text {"type":"state",state,message}.
 */

export type StreamState = "connecting" | "starting" | "running" | "reconnecting" | "draining" | "failed" | "stopped";

export interface StreamInit {
  codec: string;
  width: number;
  height: number;
  segment: ArrayBuffer;
}

export interface StreamListener {
  onInit(init: StreamInit): void;
  onMedia(fragment: ArrayBuffer, keyframe: boolean): void;
  onState(state: StreamState, message: string | null): void;
}

type SocketFactory = (url: string) => WebSocket;

/** How long a connection stays open after its last player leaves. */
export const LINGER_MS = 3000;
/** Retry delays when the server refuses the stream before it starts (e.g. 409:
 * the camera's stream profiles aren't known yet right after it was added). */
export const RETRY_MS = [1000, 2000, 5000, 10000, 30000];
/** Cap for the cached current GOP (bytes) used to start late players. */
const MAX_GOP_BYTES = 8 * 1024 * 1024;

export class StreamConnection {
  readonly key: string;
  private socket: WebSocket | null = null;
  private listeners = new Set<StreamListener>();
  private pendingInit: Omit<StreamInit, "segment"> | null = null;
  private init: StreamInit | null = null;
  private gop: ArrayBuffer[] = [];
  private gopBytes = 0;
  private state: StreamState = "connecting";
  private message: string | null = null;
  private lingerTimer: ReturnType<typeof setTimeout> | undefined;
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private retries = 0;
  private closed = false;

  constructor(
    key: string,
    private url: string,
    private makeSocket: SocketFactory,
    private onGone: (key: string) => void,
  ) {
    this.key = key;
    this.open();
  }

  private open() {
    const ws = this.makeSocket(this.url);
    let received = false;
    ws.binaryType = "arraybuffer";
    ws.onmessage = (ev) => {
      received = true;
      this.retries = 0;
      this.handle(ev.data);
    };
    ws.onclose = () => {
      if (this.closed) return;
      // Refused before anything arrived (the browser hides the status): the
      // stream may simply not be available yet, so try again with backoff.
      if (!received && this.listeners.size > 0) {
        const delay = RETRY_MS[Math.min(this.retries, RETRY_MS.length - 1)]!;
        this.retries += 1;
        this.setState("connecting", "Waiting for the camera's stream…");
        this.retryTimer = setTimeout(() => {
          if (!this.closed) this.open();
        }, delay);
        return;
      }
      if (this.state !== "failed" && this.state !== "stopped") this.setState("stopped", "connection lost");
      this.close();
    };
    this.socket = ws;
  }

  private handle(data: unknown) {
    if (typeof data === "string") {
      let msg: { type?: string; codec?: string; width?: number; height?: number; state?: StreamState; message?: string | null };
      try {
        msg = JSON.parse(data);
      } catch {
        return;
      }
      if (msg.type === "init") {
        this.pendingInit = { codec: msg.codec ?? "", width: msg.width ?? 0, height: msg.height ?? 0 };
      } else if (msg.type === "state" && msg.state) {
        this.setState(msg.state, msg.message ?? null);
      }
      return;
    }
    if (!(data instanceof ArrayBuffer)) return;
    if (this.pendingInit) {
      this.init = { ...this.pendingInit, segment: data };
      this.pendingInit = null;
      this.gop = [];
      this.gopBytes = 0;
      this.listeners.forEach((l) => l.onInit(this.init!));
      return;
    }
    const keyframe = ((new Uint8Array(data, 0, 1)[0] ?? 0) & 1) === 1;
    const fragment = data.slice(1);
    if (keyframe) {
      this.gop = [];
      this.gopBytes = 0;
    }
    if (this.gopBytes + fragment.byteLength <= MAX_GOP_BYTES) {
      this.gop.push(fragment);
      this.gopBytes += fragment.byteLength;
    }
    this.listeners.forEach((l) => l.onMedia(fragment, keyframe));
  }

  private setState(state: StreamState, message: string | null) {
    this.state = state;
    this.message = message;
    this.listeners.forEach((l) => l.onState(state, message));
  }

  /** Attach a player. It gets the current state, init and GOP immediately. */
  attach(listener: StreamListener): () => void {
    clearTimeout(this.lingerTimer);
    this.listeners.add(listener);
    listener.onState(this.state, this.message);
    if (this.init) {
      listener.onInit(this.init);
      this.gop.forEach((f, i) => listener.onMedia(f, i === 0));
    }
    return () => {
      this.listeners.delete(listener);
      if (this.listeners.size === 0) {
        this.lingerTimer = setTimeout(() => this.close(), LINGER_MS);
      }
    };
  }

  get viewerCount() {
    return this.listeners.size;
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.lingerTimer);
    clearTimeout(this.retryTimer);
    this.socket?.close();
    this.onGone(this.key);
  }
}

export class StreamRegistry {
  private connections = new Map<string, StreamConnection>();

  constructor(
    private makeSocket: SocketFactory = (url) => new WebSocket(url),
    private base: () => string = () => window.location.href,
  ) {}

  static key(cameraId: string, profile: string | undefined, purpose: string) {
    return `${cameraId}|${profile ?? `@${purpose}`}`;
  }

  /** Get (or open) the shared connection for a camera stream. */
  connection(cameraId: string, profile: string | undefined, purpose: "detail" | "grid" = "detail") {
    const key = StreamRegistry.key(cameraId, profile, purpose);
    let conn = this.connections.get(key);
    if (!conn) {
      const url = new URL(`/api/v1/streams/${cameraId}/ws`, this.base());
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      if (profile !== undefined) url.searchParams.set("profile", profile);
      else url.searchParams.set("purpose", purpose);
      conn = new StreamConnection(key, url.toString(), this.makeSocket, (k) => this.connections.delete(k));
      this.connections.set(key, conn);
    }
    return conn;
  }

  get size() {
    return this.connections.size;
  }
}

/** The app-wide registry. */
export const streams = new StreamRegistry();
