import type { StreamConnection, StreamInit, StreamState } from "./registry";

export interface PlayerCallbacks {
  onState(state: StreamState | "unsupported", message: string | null): void;
}

/** Whether this browser can play live video at all. */
export function mseSupported(): boolean {
  return typeof window !== "undefined" && "MediaSource" in window;
}

/**
 * Play a shared stream in a <video> element through Media Source Extensions.
 * Keeps close to the live edge and trims old buffer. Returns a cleanup function.
 */
export function attachPlayer(video: HTMLVideoElement, conn: StreamConnection, cb: PlayerCallbacks): () => void {
  if (!mseSupported()) {
    cb.onState("unsupported", "This browser can't play live video (Media Source Extensions missing).");
    return () => {};
  }
  let mediaSource: MediaSource | null = null;
  let sourceBuffer: SourceBuffer | null = null;
  let objectUrl: string | null = null;
  let queue: ArrayBuffer[] = [];
  let waitingForKeyframe = true;
  let codec: string | null = null;

  const pump = () => {
    if (!sourceBuffer || sourceBuffer.updating || queue.length === 0) return;
    try {
      sourceBuffer.appendBuffer(queue.shift()!);
    } catch {
      // QuotaExceeded or a decode error: drop the queue and resync at a keyframe.
      queue = [];
      waitingForKeyframe = true;
    }
  };

  const reset = (init: StreamInit) => {
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    queue = [init.segment];
    waitingForKeyframe = true;
    sourceBuffer = null;
    codec = init.codec;
    const type = `video/mp4; codecs="${init.codec}"`;
    if (!MediaSource.isTypeSupported(type)) {
      cb.onState("unsupported", `This browser can't decode ${init.codec}.`);
      return;
    }
    mediaSource = new MediaSource();
    objectUrl = URL.createObjectURL(mediaSource);
    video.src = objectUrl;
    mediaSource.addEventListener(
      "sourceopen",
      () => {
        if (!mediaSource) return;
        sourceBuffer = mediaSource.addSourceBuffer(type);
        sourceBuffer.mode = "segments";
        sourceBuffer.addEventListener("updateend", pump);
        pump();
      },
      { once: true },
    );
  };

  const detach = conn.attach({
    onInit: (init) => {
      if (init.codec !== codec || !sourceBuffer) reset(init);
    },
    onMedia: (fragment, keyframe) => {
      if (waitingForKeyframe && !keyframe) return;
      waitingForKeyframe = false;
      queue.push(fragment);
      if (queue.length > 300) {
        // Falling far behind (e.g. tab in background): resync at the next keyframe.
        queue = [];
        waitingForKeyframe = true;
        return;
      }
      pump();
    },
    onState: (state, message) => cb.onState(state, message),
  });

  // Stay near the live edge; trim what's been played.
  const timer = setInterval(() => {
    if (!sourceBuffer || sourceBuffer.buffered.length === 0) return;
    const end = sourceBuffer.buffered.end(sourceBuffer.buffered.length - 1);
    if (end - video.currentTime > 1.5) video.currentTime = Math.max(sourceBuffer.buffered.start(0), end - 0.3);
    if (!sourceBuffer.updating && video.currentTime > 30) {
      try {
        sourceBuffer.remove(0, video.currentTime - 20);
      } catch {
        /* ignore */
      }
    }
    if (video.paused) void video.play().catch(() => {});
  }, 500);

  return () => {
    clearInterval(timer);
    detach();
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    video.removeAttribute("src");
  };
}
