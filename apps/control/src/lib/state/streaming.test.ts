import { afterEach, describe, expect, it } from "vitest";
import type { ServerInfo, StreamingSettings, StreamTarget } from "$protocol";
import { RestClient } from "../server/rest";
import type { Connection } from "../server";
import { session } from "./session.svelte";
import { streaming } from "./streaming.svelte";

const SERVER: ServerInfo = { id: "srv", name: "Booth", version: "0.1.0", protocolVersion: 1, capabilities: ["streaming"], auth: "local" };

function target(id: string, name: string): StreamTarget {
  return { id, name, kind: "rtmp", url: "rtmp://192.0.2.5/live", hasKey: true, screen: null, codec: "h264", videoMbps: 6, audioKbps: 160 };
}
function settings(...targets: StreamTarget[]): StreamingSettings {
  return { recording: { screen: null, container: "mp4", codec: "h264", videoMbps: 8, audioKbps: 192, splitMinutes: 0, folder: null, minFreeGb: 5 }, recordingsFolder: "/srv/rec", encoders: [], targets } as unknown as StreamingSettings;
}

/** A connection whose `GET /streaming` answers what `answer()` gives (a promise to hold it back). */
function fakeConn(answer: () => StreamingSettings | Promise<StreamingSettings>): Connection {
  const fetch = (async (url: string) => {
    const path = new URL(url).pathname;
    if (path === "/api/v1/streaming") return new Response(JSON.stringify(await answer()), { status: 200 });
    return new Response(JSON.stringify({ streams: [], recording: null }), { status: 200 });
  }) as unknown as typeof globalThis.fetch;
  const rest = new RestClient({ baseUrl: "http://192.0.2.5:5555", token: () => null, fetch });
  return { rest, pushNotice: () => {} } as unknown as Connection;
}

afterEach(() => {
  session.conn = null;
  session.server = null;
});

describe("streaming state (F10-2)", () => {
  it("loads the targets afresh, and shows none of another server's once Oriel connects elsewhere", async () => {
    let targets = [target("t1", "Platform A")];
    const one = fakeConn(() => settings(...targets));
    session.server = SERVER;
    session.conn = one;
    await streaming.loadSettings();
    expect(streaming.settings?.targets.map((t) => t.name)).toEqual(["Platform A"]);
    // Another Oriel added a target: the next load (the popover opening) shows it.
    targets = [...targets, target("t2", "Platform B")];
    await streaming.loadSettings();
    expect(streaming.settings?.targets.map((t) => t.name)).toEqual(["Platform A", "Platform B"]);

    let release!: (s: StreamingSettings) => void;
    const two = fakeConn(() => new Promise<StreamingSettings>((r) => (release = r)));
    session.conn = two;
    expect(streaming.settings).toBeNull();
    expect(streaming.status).toBeNull();
    const loading = streaming.loadSettings();
    // A late answer from the first server is not taken for the second.
    session.conn = one;
    session.conn = two;
    release(settings(target("t9", "Overflow room")));
    await loading;
    expect(streaming.settings?.targets.map((t) => t.name)).toEqual(["Overflow room"]);
  });

  it("drops an answer that arrives after Oriel connected elsewhere", async () => {
    let release!: (s: StreamingSettings) => void;
    const one = fakeConn(() => new Promise<StreamingSettings>((r) => (release = r)));
    session.server = SERVER;
    session.conn = one;
    const loading = streaming.loadSettings();
    session.conn = fakeConn(() => settings());
    release(settings(target("t1", "Platform A")));
    await loading;
    expect(streaming.settings).toBeNull();
  });
});
