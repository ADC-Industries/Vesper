import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientMessage, ServerMessage } from "$protocol";
import { CommandError, ShowSocket, backoffDelay, type SocketEvents, type SocketLike, type SocketStatus } from "../socket";
import { createShowStore } from "../state";
import { showState } from "./fixtures";

class FakeSocket implements SocketLike {
  binaryType = "blob";
  readyState = 0;
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: { code: number; reason: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  sent: ClientMessage[] = [];
  closed = false;
  constructor(public url: string) {}
  send(data: string) {
    this.sent.push(JSON.parse(data) as ClientMessage);
  }
  close() {
    this.closed = true;
    this.readyState = 3;
  }
  // Test helpers: the server side.
  open() {
    this.readyState = 1;
    this.onopen?.({});
  }
  receive(msg: ServerMessage) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  receiveBinary(buf: ArrayBuffer) {
    this.onmessage?.({ data: buf });
  }
  drop(code = 1006, reason = "") {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
  commands() {
    return this.sent.filter((m): m is Extract<ClientMessage, { type: "command" }> => m.type === "command");
  }
}

const HELLO: ServerMessage = { type: "hello", server: { id: "srv", name: "Booth", version: "0.1.0", protocolVersion: 1 } };

function setup(extra: Partial<SocketEvents> = {}) {
  const sockets: FakeSocket[] = [];
  const store = createShowStore();
  const statuses: SocketStatus[] = [];
  const synced: boolean[] = [];
  const binary: ArrayBuffer[] = [];
  const socket = new ShowSocket({
    url: () => "ws://test/api/v1/ws?protocolVersion=1",
    store,
    clientName: "test",
    backoff: { initialMs: 100, maxMs: 1000, factor: 2, jitter: 0 },
    create: (url) => {
      const s = new FakeSocket(url);
      sockets.push(s);
      return s;
    },
    events: {
      onStatus: (s) => statuses.push(s),
      onSynced: (r) => synced.push(r),
      onBinary: (b) => binary.push(b),
      ...extra,
    },
  });
  const last = () => sockets[sockets.length - 1]!;
  /** Server accepts the connection and sends hello + state. */
  const connect = (revision: number) => {
    last().open();
    last().receive(HELLO);
    last().receive({ type: "state", revision, state: showState() });
  };
  return { socket, store, sockets, statuses, synced, binary, last, connect };
}

describe("ShowSocket", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("goes online after hello + first state, and sends a hello of its own", () => {
    const t = setup();
    t.socket.start();
    expect(t.socket.current.phase).toBe("connecting");
    t.connect(7);
    expect(t.socket.current.phase).toBe("online");
    expect(t.socket.current.server?.name).toBe("Booth");
    expect(t.store.revision).toBe(7);
    expect(t.last().sent[0]).toMatchObject({ type: "hello", protocolVersion: 1 });
    expect(t.last().binaryType).toBe("arraybuffer");
    expect(t.synced).toEqual([false]);
  });

  it("resolves commands with their result and rejects refused ones", async () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    const ok = t.socket.command({ type: "next" });
    const bad = t.socket.command({ type: "prev" });
    const [c1, c2] = t.last().commands();
    expect(c1!.command).toEqual({ type: "next" });
    t.last().receive({ type: "state", revision: 2, state: showState() });
    t.last().receive({ type: "result", id: c1!.id, ok: true, revision: 2 });
    t.last().receive({ type: "result", id: c2!.id, ok: false, error: { code: "conflict", message: "nothing live" } });
    await expect(ok).resolves.toMatchObject({ ok: true, revision: 2 });
    await expect(bad).rejects.toMatchObject({ code: "conflict" });
    expect(t.store.revision).toBe(2);
  });

  it("never queues commands while offline", async () => {
    const t = setup();
    await expect(t.socket.command({ type: "next" })).rejects.toBeInstanceOf(CommandError);
    t.socket.start();
    await expect(t.socket.command({ type: "next" })).rejects.toMatchObject({ code: "disconnected" });
  });

  it("times out commands the server never answers", async () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    const p = t.socket.command({ type: "next" });
    vi.advanceTimersByTime(5001);
    await expect(p).rejects.toMatchObject({ code: "timeout" });
  });

  it("reconnects with exponential backoff and resyncs from the new state", async () => {
    const t = setup();
    t.socket.start();
    t.connect(100);
    const pending = t.socket.command({ type: "next" });
    t.last().drop();
    await expect(pending).rejects.toMatchObject({ code: "disconnected" });
    expect(t.socket.current.phase).toBe("reconnecting");
    expect(t.socket.current.attempt).toBe(1);
    expect(t.sockets).toHaveLength(1);

    vi.advanceTimersByTime(99);
    expect(t.sockets).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(t.sockets).toHaveLength(2); // retry after 100 ms
    t.last().drop(); // still down
    expect(t.socket.current.attempt).toBe(2);
    vi.advanceTimersByTime(199);
    expect(t.sockets).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(t.sockets).toHaveLength(3); // then 200 ms

    // The server restarted: its revisions start low again, and the state must still apply.
    t.connect(2);
    expect(t.socket.current.phase).toBe("online");
    expect(t.socket.current.attempt).toBe(0);
    expect(t.store.revision).toBe(2);
    expect(t.synced).toEqual([false, true]);
  });

  it("re-sends preview subscriptions after a reconnect", () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    t.socket.subscribePreview("scr", 10, 480);
    expect(t.last().commands().map((c) => c.command)).toEqual([{ type: "subscribePreview", screen: "scr", fps: 10, width: 480 }]);
    t.last().drop();
    vi.advanceTimersByTime(100);
    t.connect(1);
    expect(t.last().commands().map((c) => c.command)).toEqual([{ type: "subscribePreview", screen: "scr", fps: 10, width: 480 }]);
    t.socket.unsubscribePreview("scr");
    t.last().drop();
    vi.advanceTimersByTime(100);
    t.connect(1);
    expect(t.last().commands()).toHaveLength(0);
  });

  it("passes binary frames through", () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    t.last().receiveBinary(new ArrayBuffer(40));
    expect(t.binary).toHaveLength(1);
  });

  it("gives up on an incompatible protocol (close 4001)", () => {
    const t = setup();
    t.socket.start();
    t.last().open();
    t.last().receive({ type: "error", error: { code: "incompatibleProtocol", message: "v2 needed" } });
    t.last().drop(4001, "incompatible protocol version");
    expect(t.socket.current.phase).toBe("incompatible");
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength(1);
  });

  it("stops for good when the key is revoked (close 4003)", () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    t.last().drop(4003, "key revoked");
    expect(t.socket.current.phase).toBe("revoked");
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength(1);
  });

  it("passes codexChanged and siteChanged on", () => {
    const codex = vi.fn();
    const site = vi.fn();
    const t = setup({ onCodexChanged: codex, onSiteChanged: site });
    t.socket.start();
    t.connect(1);
    t.last().receive({ type: "codexChanged", parts: ["collections"], collection: "c1" });
    t.last().receive({ type: "siteChanged", parts: ["joinRequests"] });
    expect(codex).toHaveBeenCalledWith({ type: "codexChanged", parts: ["collections"], collection: "c1" });
    expect(site).toHaveBeenCalledWith({ type: "siteChanged", parts: ["joinRequests"] });
  });

  it("passes siteEvent on", () => {
    const onSiteEvent = vi.fn();
    const t = setup({ onSiteEvent });
    t.socket.start();
    t.connect(1);
    const event = { name: "device_joined", level: "ok", text: "Side Room joined the site", at: "2026-11-23T10:41:00Z", detail: {} } as const;
    t.last().receive({ type: "siteEvent", event });
    expect(onSiteEvent).toHaveBeenCalledWith({ type: "siteEvent", event });
  });

  it("fails a connection that opens but never syncs", () => {
    const t = setup();
    t.socket.start();
    t.last().open();
    vi.advanceTimersByTime(6000);
    expect(t.socket.current.phase).toBe("connecting");
    expect(t.socket.current.attempt).toBe(1);
    expect(t.last().closed).toBe(true);
  });

  it("stop() closes and does not reconnect", () => {
    const t = setup();
    t.socket.start();
    t.connect(1);
    t.socket.stop();
    expect(t.socket.current.phase).toBe("idle");
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength(1);
  });
});

describe("backoffDelay", () => {
  it("doubles up to the cap", () => {
    const b = { initialMs: 500, maxMs: 8000, factor: 2, jitter: 0 };
    expect([1, 2, 3, 4, 5, 6, 9].map((n) => backoffDelay(n, b))).toEqual([500, 1000, 2000, 4000, 8000, 8000, 8000]);
  });
  it("stays within the jitter band", () => {
    const b = { initialMs: 1000, maxMs: 8000, factor: 2, jitter: 0.2 };
    expect(backoffDelay(1, b, () => 0)).toBe(800);
    expect(backoffDelay(1, b, () => 1)).toBe(1200);
  });
});
