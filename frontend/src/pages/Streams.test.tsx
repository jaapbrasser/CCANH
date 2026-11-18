import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, live, server, signedInAs } from "../test/server";

const CAMERA_ID = "00000000-0000-7000-8000-000000000001";

function stream(over: Record<string, unknown> = {}) {
  return {
    camera_id: CAMERA_ID,
    profile: "grid",
    state: "running",
    subscribers: 3,
    codec: "avc1.4D401E",
    width: 640,
    height: 360,
    in_bytes_per_sec: 50_000,
    out_bytes_per_sec: 150_000,
    bytes_in_total: 1_000_000,
    bytes_out_total: 3_000_000,
    buffer_bytes: 1_048_576,
    startup_ms: 120,
    reconnects: 0,
    last_error: null,
    started_at: new Date().toISOString(),
    ...over,
  };
}

describe("streams page", () => {
  it("lists shared streams, updates live and force-stops", async () => {
    signedInAs(ADMIN);
    let stopped: string | null = null;
    server.use(
      http.get("/api/v1/streams", () => HttpResponse.json([stream()])),
      http.delete("/api/v1/streams/:id", ({ request }) => {
        stopped = new URL(request.url).searchParams.get("profile");
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/streams");
    const table = await screen.findByRole("table");
    const row = (await within(table).findByText("3")).closest("tr")!;
    expect(within(row).getByText("running")).toBeInTheDocument();
    expect(within(row).getByText("400 kbit/s")).toBeInTheDocument();

    await waitFor(() => expect(screen.getByText("Live")).toBeInTheDocument());
    act(() => live.broadcast(JSON.stringify({ type: "streams_updated", streams: [stream({ subscribers: 5 })] })));
    await within(table).findByText("5");

    await userEvent.click(within(table).getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(stopped).toBe("grid"));
  });

  it("is admin only", async () => {
    signedInAs(VIEWER);
    renderApp("/streams");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Streams" })).not.toBeInTheDocument());
  });
});
