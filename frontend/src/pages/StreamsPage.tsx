import { Link } from "react-router";
import { useCameras } from "../api/cameras";
import { errorMessage } from "../api/client";
import { useStopStream, useStreams, type StreamInfo } from "../api/streams";
import { Alert, Button, PageHeader } from "../components/ui";
import { usePageTitle } from "../components/pageTitle";

function rate(bytesPerSec: number): string {
  const bits = bytesPerSec * 8;
  if (bits >= 1e6) return `${(bits / 1e6).toFixed(1)} Mbit/s`;
  if (bits >= 1e3) return `${Math.round(bits / 1e3)} kbit/s`;
  return `${bits} bit/s`;
}

function mb(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

const STATE_STYLE: Record<StreamInfo["state"], string> = {
  starting: "text-slate-300",
  running: "text-emerald-300",
  reconnecting: "text-amber-300",
  draining: "text-slate-400",
  failed: "text-red-300",
  stopped: "text-slate-500",
};

export function StreamsPage() {
  usePageTitle("Streams");
  const streams = useStreams();
  const cameras = useCameras({ page_size: 200 });
  const stop = useStopStream();
  const names = new Map(cameras.data?.items.map((c) => [c.id, c.name]) ?? []);
  const list = streams.data ?? [];
  const viewers = list.reduce((n, s) => n + s.subscribers, 0);
  const upstream = list.reduce((n, s) => n + s.in_bytes_per_sec, 0);
  const downstream = list.reduce((n, s) => n + s.out_bytes_per_sec, 0);

  return (
    <div>
      <PageHeader title="Streams" />
      <p className="mb-4 max-w-3xl text-sm text-slate-400">
        Each row is one connection to a camera, shared by all its viewers. Updates live. Video is buffered in memory
        only; nothing is recorded.
      </p>
      <section aria-label="Totals" className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {[
          ["Camera connections", list.length],
          ["Viewers", viewers],
          ["From cameras", rate(upstream)],
          ["To viewers", rate(downstream)],
        ].map(([label, value]) => (
          <div key={label} className="rounded border border-slate-800 p-3">
            <p className="text-xs text-slate-400">{label}</p>
            <p className="text-xl font-semibold">{value}</p>
          </div>
        ))}
      </section>
      {(streams.isError || stop.isError) && <Alert>{errorMessage(streams.error ?? stop.error)}</Alert>}
      <div className="overflow-x-auto rounded border border-slate-800">
        <table className="w-full text-left text-sm">
          <thead className="bg-slate-900 text-slate-400">
            <tr>
              <th className="px-4 py-2 font-medium">Camera</th>
              <th className="px-4 py-2 font-medium">Profile</th>
              <th className="px-4 py-2 font-medium">State</th>
              <th className="px-4 py-2 font-medium">Viewers</th>
              <th className="px-4 py-2 font-medium">In</th>
              <th className="px-4 py-2 font-medium">Out</th>
              <th className="px-4 py-2 font-medium">Startup</th>
              <th className="px-4 py-2 font-medium">Buffer</th>
              <th className="px-4 py-2 font-medium">Reconnects</th>
              <th className="px-4 py-2 font-medium">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {list.map((s) => (
              <tr key={`${s.camera_id}|${s.profile}`} className="border-t border-slate-800">
                <td className="px-4 py-2">
                  <Link to={`/cameras/${s.camera_id}`} className="hover:underline">
                    {names.get(s.camera_id) ?? s.camera_id}
                  </Link>
                </td>
                <td className="px-4 py-2 text-slate-300">
                  {s.profile || "default"}
                  {s.width && s.height && <span className="text-slate-500"> · {s.width}×{s.height}</span>}
                </td>
                <td className={`px-4 py-2 ${STATE_STYLE[s.state]}`} title={s.last_error ?? undefined}>
                  {s.state}
                </td>
                <td className="px-4 py-2">{s.subscribers}</td>
                <td className="px-4 py-2 text-slate-300">{rate(s.in_bytes_per_sec)}</td>
                <td className="px-4 py-2 text-slate-300">{rate(s.out_bytes_per_sec)}</td>
                <td className="px-4 py-2 text-slate-400">{s.startup_ms != null ? `${s.startup_ms} ms` : "—"}</td>
                <td className="px-4 py-2 text-slate-400">{mb(s.buffer_bytes)}</td>
                <td className="px-4 py-2 text-slate-400">{s.reconnects}</td>
                <td className="px-4 py-2 text-right">
                  <Button
                    variant="danger"
                    disabled={stop.isPending}
                    onClick={() => stop.mutate({ cameraId: s.camera_id, profile: s.profile })}
                  >
                    Stop
                  </Button>
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={10} className="px-4 py-8 text-center text-slate-500">
                  No active streams. Open a camera's live view to start one.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
