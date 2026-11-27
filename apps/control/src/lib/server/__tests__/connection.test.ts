import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientMessage, ServerMessage } from "$protocol";
import { Connection, mergeNotice, type Notice } from "../connection";
import type { SocketLike } from "../socket";
import { showState } from "./fixtures";

class FakeSocket implements SocketLike {
  binaryType = "blob";
  readyState = 0;
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: { code: number; reason: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  sent: ClientMessage[] = [];
  constructor(public url: string) {}
  send(data: string) {
    this.sent.push(JSON.parse(data) as ClientMessage);
  }
  close() {
    this.readyState = 3;
  }
  receive(msg: ServerMessage) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  connect() {
    this.readyState = 1;
    this.onopen?.({});
    this.receive({ type: "hello", server: { id: "s", name: "Booth", version: "0", protocolVersion: 1 } });
    this.receive({ type: "state", revision: 1, state: showState() });
  }
  pings() {
    return this.sent.filter((m): m is Extract<ClientMessage, { type: "ping" }> => m.type === "ping");
  }
}

function setup(heartbeatMs = 1000) {
  const sockets: FakeSocket[] = [];
  const conn = new Connection({
    baseUrl: "http://booth:5555",
    token: null,
    heartbeatMs,
    pingTimeoutMs: 500,
    createSocket: (url) => {
      const s = new FakeSocket(url);
      sockets.push(s);
      return s;
    },
  });
  let notices: Notice[] = [];
  conn.notices.subscribe((n) => (notices = n));
  let phase = "";
  conn.status.subscribe((s) => (phase = s.phase));
  return { conn, sockets, last: () => sockets[sockets.length - 1]!, notices: () => notices, phase: () => phase };
}

const n = (id: number, message: string, key: string | null = null): Notice => ({ id, level: "warning", message, key, count: 1, at: id });

describe("notice merging", () => {
  it("replaces a notice with the same key instead of stacking it", () => {
    let list: Notice[] = [];
    list = mergeNotice(list, n(1, "2 fonts not matched", "fonts"));
    list = mergeNotice(list, n(2, "Output error"));
    list = mergeNotice(list, n(3, "3 fonts not matched", "fonts"));
    expect(list.map((x) => [x.id, x.message])).toEqual([
      [2, "Output error"],
      [3, "3 fonts not matched"],
    ]);
    // Different keys stay separate.
    list = mergeNotice(list, n(4, "1 media file missing in 'A'", "missingMedia:a"));
    list = mergeNotice(list, n(5, "4 media files missing in 'B'", "missingMedia:b"));
    expect(list).toHaveLength(4);
  });

  it("counts repeats of an unkeyed message and keeps the newest six", () => {
    let list: Notice[] = [];
    list = mergeNotice(list, n(1, "same"));
    list = mergeNotice(list, n(2, "same"));
    expect(list).toHaveLength(1);
    expect(list[0]!.count).toBe(2);
    // A keyed notice never folds into an unkeyed one with the same text.
    list = mergeNotice(list, n(3, "same", "k"));
    expect(list).toHaveLength(2);
    for (let i = 10; i < 20; i++) list = mergeNotice(list, n(i, `m${i}`));
    expect(list).toHaveLength(6);
    expect(list[5]!.message).toBe("m19");
  });

  it("applies server notices by key", () => {
    const t = setup(0);
    t.conn.start();
    t.last().connect();
    t.last().receive({ type: "notice", level: "info", message: "1 font not matched", key: "fonts" });
    t.last().receive({ type: "notice", level: "info", message: "2 fonts not matched", key: "fonts" });
    t.last().receive({ type: "notice", level: "error", message: "output error" });
    expect(t.notices().map((x) => x.message)).toEqual(["2 fonts not matched", "output error"]);
    expect(t.notices()[0]!.key).toBe("fonts");
    t.conn.stop();
  });
});

describe("dropped events", () => {
  it("a 'lagged' notice reads everything again, like a reconnect, each time (X3-6)", () => {
    const t = setup(0);
    const synced: boolean[] = [];
    t.conn.onSynced((r) => synced.push(r));
    t.conn.start();
    t.last().connect();
    expect(synced).toEqual([false]);
    const lagged = (n: number) => t.last().receive({ type: "notice", level: "warning", message: `${n} events were dropped because this client is slow; refetch documents`, key: "lagged" });
    lagged(300);
    lagged(300);
    expect(synced).toEqual([false, true, true]);
    expect(t.notices().map((x) => x.message)).toEqual(["Oriel missed some changes from the server and read its lists again."]);
    t.conn.stop();
  });
});

describe("audio levels", () => {
  it("reports audioLevels messages and state levels without touching the show store", () => {
    const t = setup(0);
    const got: number[][] = [];
    t.conn.onAudioLevels((l) => got.push(l.levels));
    t.conn.start();
    t.last().connect(); // fixture state: [-60, -60]
    let states = 0;
    t.conn.show.subscribe(() => states++);
    t.last().receive({ type: "audioLevels", levels: [-12, -14], xruns: 2 });
    expect(t.conn.audioLevels).toEqual({ levels: [-12, -14], xruns: 2 });
    // A state with the same levels does not repeat them; a new one does.
    const base = showState();
    t.last().receive({ type: "state", revision: 2, state: base });
    t.last().receive({ type: "state", revision: 3, state: { ...base, audio: { ...base.audio, levels: [-30, -31] } } });
    expect(got).toEqual([[-60, -60], [-12, -14], [-30, -31]]);
    expect(states).toBe(3); // the initial call plus two states: meters are not states
    t.conn.stop();
  });
});

describe("site events", () => {
  it("hands every siteEvent to its listeners until they unsubscribe", () => {
    const t = setup(0);
    const got: string[] = [];
    const off = t.conn.onSiteEvent((m) => got.push(m.event.text));
    t.conn.start();
    t.last().connect();
    const event = (text: string) => ({ type: "siteEvent" as const, event: { name: "device_online", level: "ok" as const, text, at: "2026-11-23T10:41:00Z", detail: {} } });
    t.last().receive(event("Side Room is online"));
    off();
    t.last().receive(event("Side Room is online again"));
    expect(got).toEqual(["Side Room is online"]);
    t.conn.stop();
  });
});

describe("ping heartbeat", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("pings over the WebSocket when quiet and stays online on pong", async () => {
    const t = setup(1000);
    t.conn.start();
    t.last().connect();
    vi.advanceTimersByTime(999);
    expect(t.last().pings()).toHaveLength(0);
    vi.advanceTimersByTime(1);
    const [ping] = t.last().pings();
    expect(ping).toMatchObject({ type: "ping" });
    t.last().receive({ type: "pong", id: ping!.id });
    await vi.advanceTimersByTimeAsync(600);
    expect(t.phase()).toBe("online");
    // Quiet again: the next beat pings again.
    await vi.advanceTimersByTimeAsync(1000);
    expect(t.last().pings()).toHaveLength(2);
    expect(t.sockets).toHaveLength(1);
    t.conn.stop();
  });

  it("does not ping while messages keep arriving", () => {
    const t = setup(1000);
    t.conn.start();
    t.last().connect();
    for (let i = 0; i < 5; i++) {
      vi.advanceTimersByTime(600);
      t.last().receive({ type: "state", revision: 2 + i, state: showState() });
    }
    expect(t.last().pings()).toHaveLength(0);
    t.conn.stop();
  });

  it("drops the socket when the pong does not come", async () => {
    const t = setup(1000);
    t.conn.start();
    t.last().connect();
    await vi.advanceTimersByTimeAsync(1000);
    expect(t.last().pings()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(499);
    expect(t.phase()).toBe("online");
    await vi.advanceTimersByTimeAsync(1);
    expect(t.phase()).toBe("reconnecting");
    t.conn.stop();
  });
});

describe("timer values", () => {
  it("takes the state's values, then ticks", async () => {
    const { applyTimerTick, timerValuesFromState } = await import("../connection");
    const base = timerValuesFromState([
      { id: "a", name: "A", kind: { type: "countdown", seconds: 60 }, allowOverrun: false, state: "running", currentSeconds: 60, display: "01:00", warningSeconds: null },
      { id: "b", name: "B", kind: { type: "elapsed" }, allowOverrun: false, state: "stopped", currentSeconds: 0, display: "00:00", warningSeconds: null },
    ]);
    const next = applyTimerTick(base, [{ id: "a", state: "running", currentSeconds: 59, display: "00:59" }]);
    expect(next.a!.display).toBe("00:59");
    expect(next.b).toEqual(base.b);
  });
});
