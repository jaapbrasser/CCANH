import { useEffect, useState, type RefObject } from "react";

/** How long a player may be out of sight before it pauses (SPEC §8.2). */
export const PAUSE_DELAY_MS = 5000;

/**
 * Whether the element has been out of sight (scrolled off-screen, or the tab
 * hidden) for at least `delayMs`. Becomes false again as soon as it's visible.
 */
export function usePausedWhenHidden(ref: RefObject<Element | null>, enabled: boolean, delayMs = PAUSE_DELAY_MS) {
  const [paused, setPaused] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!enabled || !el) return;
    let onScreen = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      const visible = onScreen && document.visibilityState !== "hidden";
      clearTimeout(timer);
      if (visible) setPaused(false);
      else timer = setTimeout(() => setPaused(true), delayMs);
    };
    const observer =
      typeof IntersectionObserver === "undefined"
        ? null
        : new IntersectionObserver((entries) => {
            onScreen = entries.some((e) => e.isIntersecting);
            update();
          });
    observer?.observe(el);
    document.addEventListener("visibilitychange", update);
    return () => {
      clearTimeout(timer);
      observer?.disconnect();
      document.removeEventListener("visibilitychange", update);
      setPaused(false);
    };
  }, [ref, enabled, delayMs]);
  return paused;
}
