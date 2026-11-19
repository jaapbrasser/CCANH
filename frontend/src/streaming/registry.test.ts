import { LINGER_MS, RETRY_MS, StreamRegistry, type StreamListener } from "./registry";

class FakeSocket {
  static all: FakeSocket[] = [];
  binaryType = "blob";
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    FakeSocket.all.push(this);
  }
  close() {
    this.closed = true;
  }
  text(msg: object) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  binary(bytes: number[]) {
    this.onmessage?.({ data: new Uint8Array(bytes).buffer });
  }
}

function listener() {
  const log: string[] = [];
  const l: StreamListener = {
    onInit: (i) => log.push(`init ${i.codec} ${i.segment.byteLength}`),
    onMedia: (f, k) => log.push(`media ${new Uint8Array(f).join(",")}${k ? " key" : ""}`),
    onState: (s) => log.push(`state ${s}`),
  };
  return { l, log };
}

function registry() {
  FakeSocket.all = [];
  return new StreamRegistry((url) => new FakeSocket(url) as unknown as WebSocket, () => "https://crane.local/cameras");
}

describe("stream registry", () => {
  afterEach(() => vi.useRealTimers());

  it("shares one socket per camera and profile and builds the URL", () => {
    const r = registry();
    const a = r.connection("cam1", "grid");
    const b = r.connection("cam1", "grid");
    r.connection("cam1", undefined, "detail");
    expect(a).toBe(b);
    expect(FakeSocket.all.map((s) => s.url)).toEqual([
      "wss://crane.local/api/v1/streams/cam1/ws?profile=grid",
      "wss://crane.local/api/v1/streams/cam1/ws?purpose=detail",
    ]);
  });

  it("replays state, init and the current GOP to late players", () => {
    const r = registry();
    const conn = r.connection("cam1", "grid");
    const sock = FakeSocket.all[0]!;
    sock.text({ type: "state", state: "running", message: null });
    sock.text({ type: "init", codec: "avc1.4D401E", width: 640, height: 360 });
    sock.binary([9, 9, 9]);
    sock.binary([0, 1]); // delta before any keyframe in this GOP: dropped on next key
    sock.binary([1, 2]); // keyframe starts a new GOP
    sock.binary([0, 3]);

    const { l, log } = listener();
    conn.attach(l);
    expect(log).toEqual(["state running", "init avc1.4D401E 3", "media 2 key", "media 3"]);
    sock.binary([0, 4]);
    expect(log.at(-1)).toBe("media 4");
  });

  it("lingers after the last player leaves, then closes", () => {
    vi.useFakeTimers();
    const r = registry();
    const conn = r.connection("cam1", "grid");
    const detach = conn.attach(listener().l);
    detach();
    vi.advanceTimersByTime(LINGER_MS - 1);
    // Re-attaching within the linger keeps the same connection.
    expect(r.connection("cam1", "grid")).toBe(conn);
    const again = conn.attach(listener().l);
    vi.advanceTimersByTime(LINGER_MS * 2);
    expect(FakeSocket.all[0]!.closed).toBe(false);
    again();
    vi.advanceTimersByTime(LINGER_MS);
    expect(FakeSocket.all[0]!.closed).toBe(true);
    expect(r.size).toBe(0);
  });

  it("retries with backoff when refused before the stream starts", () => {
    vi.useFakeTimers();
    const r = registry();
    const conn = r.connection("cam1", undefined, "grid");
    const { l, log } = listener();
    conn.attach(l);
    FakeSocket.all[0]!.onclose?.(); // e.g. 409: profiles not known yet
    expect(log.at(-1)).toBe("state connecting");
    expect(r.connection("cam1", undefined, "grid")).toBe(conn);
    vi.advanceTimersByTime(RETRY_MS[0]!);
    expect(FakeSocket.all).toHaveLength(2);
    FakeSocket.all[1]!.text({ type: "state", state: "running", message: null });
    expect(log.at(-1)).toBe("state running");
  });

  it("reports a lost connection as stopped and opens a fresh one next time", () => {
    const r = registry();
    const conn = r.connection("cam1", "grid");
    const { l, log } = listener();
    conn.attach(l);
    FakeSocket.all[0]!.text({ type: "state", state: "running", message: null });
    FakeSocket.all[0]!.onclose?.();
    expect(log.at(-1)).toBe("state stopped");
    expect(r.connection("cam1", "grid")).not.toBe(conn);
    expect(FakeSocket.all).toHaveLength(2);
  });
});
