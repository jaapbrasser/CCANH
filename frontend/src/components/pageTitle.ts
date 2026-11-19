import { useEffect } from "react";
import { useHealth } from "../api/health";

export const APP_NAME = "Crane Camera Automated Navigation Hub";

/**
 * `"<section> <name> - Crane Camera Automated Navigation Hub"`, with `[DEMO]`
 * in front in demo mode so a demo tab is never mistaken for production.
 */
export function formatTitle(section: string, name?: string | null, demo = false): string {
  const head = [section, name?.trim()].filter(Boolean).join(" ");
  return `${demo ? "[DEMO] " : ""}${head} - ${APP_NAME}`;
}

/** Sets the browser tab title while the calling page is shown (issue #21). */
export function usePageTitle(section: string, name?: string | null) {
  const { data } = useHealth();
  const demo = data?.mode === "demo";
  useEffect(() => {
    document.title = formatTitle(section, name, demo);
  }, [section, name, demo]);
}
