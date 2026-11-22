import { api, unwrap } from "./client";
import type { components } from "./schema";

export type PtzMove = components["schemas"]["PtzMove"];
export type PtzOutcome = components["schemas"]["PtzOutcome"];

/** Send a PTZ command to one camera (issue #31). */
export function moveCamera(id: string, mv: PtzMove): Promise<PtzOutcome> {
  return unwrap(api.POST("/api/v1/cameras/{id}/ptz", { params: { path: { id } }, body: mv }));
}

/** Send a PTZ command to every PTZ camera on one of the user's dashboards. */
export function moveDashboard(id: string, mv: PtzMove): Promise<PtzOutcome> {
  return unwrap(api.POST("/api/v1/dashboards/{id}/ptz", { params: { path: { id } }, body: mv }));
}
