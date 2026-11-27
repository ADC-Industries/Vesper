import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ServerInfo } from "$protocol";
import { session, type OpenPresentation } from "./session.svelte";
import { libraryPage, libraryState } from "./library.svelte";
import { screensState } from "./screens.svelte";
import { shell } from "./shell.svelte";
import { changes } from "./events";
import { creds } from "./prefs";
import { mediaState } from "./media.svelte";
import { showDocsState } from "./showDocs.svelte";
import { stageState } from "./stage.svelte";
import { timersState } from "./timers.svelte";
import type { Connection } from "../server";
import { ApiRequestError } from "../server";

const BASE = "http://192.0.2.5:5555";
const SERVER: ServerInfo = { id: "srv", name: "Booth", version: "0.1.0", protocolVersion: 1, capabilities: [], auth: "unpaired" };

const json = (status: number, v: unknown) => new Response(JSON.stringify(v), { status });

afterEach(() => {
  session.cancelPairing();
  session.disconnect();
  vi.unstubAllGlobals();
});

describe("session pairing", () => {
  it("does not go on to the server once the operator went back", async () => {
    let answer!: (r: Response) => void;
    const calls: string[] = [];
    vi.stubGlobal("fetch", (url: string) => {
      const path = new URL(url).pathname;
      calls.push(path);
      if (path === "/api/v1/pair") return new Promise<Response>((r) => (answer = r));
      return Promise.resolve(json(200, { ...SERVER, auth: "paired" }));
    });
    session.pairing = { baseUrl: BASE, server: { ...SERVER } };
    const p = session.pair("123456");
    await vi.waitFor(() => expect(answer).toBeDefined());
    session.cancelPairing();
    answer(json(200, { token: "idn1.k", clientId: "ab" }));
    await p;
    expect(session.view).toBe("connect");
    expect(session.conn).toBeNull();
  });

  it("a wrong code at a relay waits for approval, says so, and can be typed again", async () => {
    const asked: string[] = [];
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
      const path = new URL(url).pathname;
      asked.push(`${init?.method ?? "GET"} ${path}`);
      if (init?.method === "DELETE") return Promise.resolve(new Response(null, { status: 204 }));
      if (path === "/api/v1/pair")
        return Promise.resolve(json(202, { pairingId: "s3cr3t", expiresInSeconds: 600, codeWrong: true }));
      return Promise.resolve(json(202, { pairingId: "s3cr3t", expiresInSeconds: 599, codeWrong: true }));
    });
    session.pairing = { baseUrl: BASE, server: { ...SERVER } };
    const p = session.pair("000000");
    await vi.waitFor(() => expect(session.pairing?.waiting).toBe(true));
    expect(session.pairing?.codeWrong).toBe(true);
    session.retryCode();
    await p;
    expect(session.pairing).toMatchObject({ waiting: false, codeWrong: false });
    expect(session.connectError).toBeNull();
    expect(asked[0]).toBe("POST /api/v1/pair");
    // The waiting pairing was taken back at the server.
    await vi.waitFor(() => expect(asked).toContain("DELETE /api/v1/pair/s3cr3t"));
  });

  it("going back takes a waiting pairing back; a failure to do so is ignored", async () => {
    const asked: string[] = [];
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
      const path = new URL(url).pathname;
      asked.push(`${init?.method ?? "GET"} ${path}`);
      if (init?.method === "DELETE") return Promise.reject(new TypeError("network down"));
      return Promise.resolve(json(202, { pairingId: "w4it", expiresInSeconds: 600 }));
    });
    session.pairing = { baseUrl: BASE, server: { ...SERVER } };
    const p = session.pair(null);
    await vi.waitFor(() => expect(session.pairing?.waiting).toBe(true));
    session.cancelPairing();
    await p;
    await vi.waitFor(() => expect(asked).toContain("DELETE /api/v1/pair/w4it"));
    expect(session.pairing).toBeNull();
    expect(session.connectError).toBeNull();
    // Only once, and not again on the next cancel.
    session.cancelPairing();
    expect(asked.filter((a) => a.startsWith("DELETE"))).toHaveLength(1);
  });
});

describe("session documentChanged", () => {
  it("a failed refetch is reported, not left as an unhandled rejection", async () => {
    const notices: string[] = [];
    const conn = {
      rest: { libraries: () => Promise.reject(new ApiRequestError("network", "Could not reach the server", 0)) },
      socket: { isOnline: true },
      pushNotice: (_l: string, m: string) => notices.push(m),
    } as unknown as Connection;
    session.conn = conn;
    const changed = (session as unknown as { documentChanged(k: string, id: string): Promise<void> }).documentChanged(
      "library",
      "l1",
    );
    await expect(changed).resolves.toBeUndefined();
    expect(notices).toEqual(["Could not reload a changed library from the server: Could not reach the server"]);
    session.conn = null;
  });

  it("a refetch answered after switching servers does not overwrite the new server's lists", async () => {
    let answer!: (v: unknown) => void;
    const old = {
      rest: { libraries: () => new Promise((r) => (answer = r)) },
      pushNotice: () => {},
    } as unknown as Connection;
    session.conn = old;
    libraryState.libraries = [];
    const changed = (session as unknown as { documentChanged(k: string, id: string): Promise<void> }).documentChanged(
      "library",
      "l1",
    );
    session.conn = null;
    answer([{ id: "old", name: "From the old server" }]);
    await changed;
    expect(libraryState.libraries).toEqual([]);
  });
});

describe("session collection switch", () => {
  it("the old collection's libraries, songs and playlists aren't asked for while this Oriel switches", async () => {
    const asked: string[] = [];
    const conn = {
      rest: {
        libraries: () => (asked.push("libraries"), Promise.resolve([])),
        playlists: () => (asked.push("playlists"), Promise.resolve([])),
        playlist: (id: string) => (asked.push(`playlist ${id}`), Promise.resolve({})),
        library: (id: string) => (asked.push(`library ${id}`), Promise.resolve([])),
      },
      socket: { isOnline: true },
      pushNotice: () => {},
    } as unknown as Connection;
    session.conn = conn;
    session.playlistId = "old-playlist";
    const changed = (kind: string, id: string) =>
      (session as unknown as { documentChanged(k: string, id: string, m: unknown): Promise<void> }).documentChanged(kind, id, {
        type: "documentChanged",
        kind,
        id,
        change: "removed",
      });
    try {
      await session.collectionSwitch(async () => {
        await changed("presentation", "p1");
        await changed("library", "l1");
        await changed("playlistNode", "old-playlist");
      });
      expect(asked).toEqual([]);
      // Afterwards, a change is followed again.
      await changed("playlistNode", "pl2");
      expect(asked).toContain("playlists");
    } finally {
      session.playlistId = null;
      session.conn = null;
    }
  });
});

describe("area modules' documentChanged refetches", () => {
  const changed = (kind: string, id: string, change = "changed") =>
    changes.emit({ type: "documentChanged", kind, id, msg: { type: "documentChanged", kind, id, change } } as never);

  it("each kind is refetched once, by its own module; a failure is reported per kind", async () => {
    const asked: string[] = [];
    const notices: string[] = [];
    const ok = (name: string, v: unknown) => () => (asked.push(name), Promise.resolve(v));
    const conn = {
      rest: {
        timers: ok("timers", [{ id: "t1" }]),
        messages: ok("messages", [{ id: "m1" }]),
        props: ok("props", []),
        themes: ok("themes", []),
        stageLayouts: ok("stageLayouts", [{ id: "s1" }]),
        stageLayoutTemplates: ok("stageLayoutTemplates", []),
        mediaBins: () => (asked.push("mediaBins"), Promise.reject(new ApiRequestError("network", "Could not reach the server", 0))),
      },
      socket: { isOnline: true },
      pushNotice: (_l: string, m: string) => notices.push(m),
    } as unknown as Connection;
    session.conn = conn;
    const offs = [mediaState.attach(), showDocsState.attach(), timersState.attach(), stageState.attach()];
    try {
      changed("timer", "t1");
      changed("message", "m1");
      changed("stageLayout", "s1");
      changed("mediaBin", "b1");
      await vi.waitFor(() => expect(notices).toHaveLength(1));
      expect(asked.sort()).toEqual(["mediaBins", "messages", "stageLayouts", "timers"]);
      expect(timersState.timerDocs).toEqual([{ id: "t1" }]);
      expect(showDocsState.selectedMessage).toBe("m1");
      expect(stageState.stageLayoutDocs).toEqual([{ id: "s1" }]);
      expect(notices).toEqual(["Could not reload a changed mediaBin from the server: Could not reach the server"]);
    } finally {
      for (const off of offs) off();
      session.conn = null;
    }
  });

  it("a removed prop that is open in the Edit view closes; the one open before comes back", async () => {
    const conn = {
      rest: { props: () => Promise.resolve([]), presentation: () => new Promise(() => {}) },
      socket: { isOnline: true },
      pushNotice: () => {},
    } as unknown as Connection;
    session.conn = conn;
    const before = { presentation: { id: "p0" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
    session.open = before;
    session.showOneSlide({ id: "prop1", sections: [] } as never);
    expect(session.open?.prop).toBe(true);
    const off = showDocsState.attach();
    try {
      changed("prop", "prop1", "removed");
      await vi.waitFor(() => expect(session.open).toBe(before));
    } finally {
      off();
      session.open = null;
      session.conn = null;
    }
  });
});

describe("batch 7: the open presentation, live following and the preview screen", () => {
  const ref = { playlist: "pl", item: "i1" };
  const fakeConn = (rest: Record<string, unknown> = {}) =>
    ({ rest: { presentation: (id: string) => Promise.resolve({ id, sections: [] }), ...rest }, socket: { isOnline: true }, pushNotice: () => {} }) as unknown as Connection;

  afterEach(() => {
    session.open = null;
    session.conn = null;
    shell.reset();
  });

  it("editing the open song on the Library page gives its playlist item back (A12-1)", async () => {
    session.conn = fakeConn();
    session.open = { presentation: { id: "A" }, slides: [], playlistItem: ref } as unknown as OpenPresentation;
    await libraryPage.startEdit("A");
    expect(session.open?.playlistItem).toBeNull();
    await libraryPage.endEdit();
    expect(session.open?.presentation.id).toBe("A");
    expect(session.open?.playlistItem).toEqual(ref);
  });

  it("live moving on in the playlist doesn't switch the presentation being edited (A12-2)", async () => {
    const opened: string[] = [];
    session.conn = fakeConn({ presentation: (id: string) => (opened.push(id), Promise.resolve({ id, sections: [] })) });
    const follow = (presentation: string) =>
      (session as unknown as { followLive(s: unknown): void }).followLive({ presentation, playlist: { ...ref, item: presentation } });
    session.open = { presentation: { id: "A" }, slides: [], playlistItem: ref } as unknown as OpenPresentation;
    follow("A");
    for (const editing of [() => (shell.mode = "edit"), () => (shell.mode = "reflow"), () => (shell.pageEditing = true)]) {
      shell.reset();
      editing();
      follow("B");
      follow("A");
      expect(opened).toEqual([]);
    }
    shell.reset();
    follow("B");
    await vi.waitFor(() => expect(session.open?.presentation.id).toBe("B"));
  });

  it("overlapping refetches of the open presentation: the last one asked for wins (A14-5)", async () => {
    const answers: ((name: string) => void)[] = [];
    session.conn = fakeConn({ presentation: (id: string) => new Promise((r) => answers.push((name) => r({ id, name, sections: [] }))) });
    session.open = { presentation: { id: "A", name: "v0" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
    const r1 = session.reopen();
    const r2 = session.reopen();
    answers[1]!("v2");
    await r2;
    answers[0]!("v1");
    await r1;
    expect(session.open?.presentation.name).toBe("v2");
  });

  it("a deleted screen that the preview showed is replaced by another (A12-6)", async () => {
    session.conn = fakeConn({ screens: () => Promise.resolve([{ id: "s2", kind: { type: "audience" } }]) });
    screensState.previewScreen = "s1";
    await (session as unknown as { documentChanged(k: string, id: string): Promise<void> }).documentChanged("screen", "s1");
    expect(screensState.previewScreen).toBe("s2");
  });

  it("disconnecting closes popovers, menus, prompts and page edits (A13-5)", async () => {
    shell.popover = "connection";
    shell.menuOpen = true;
    shell.pageEditing = true;
    const asked = shell.confirm("Q", "Sure?");
    session.disconnect();
    expect(shell.dialogOpen).toBe(false);
    expect([shell.popover, shell.menuOpen, shell.pageEditing]).toEqual([null, false, false]);
    await expect(asked).resolves.toBe(false);
  });

  it("the ChordPro dialog counts as an open dialog and Esc closes it (H11-3)", () => {
    shell.chordPro = { mode: "import" };
    expect(shell.dialogOpen).toBe(true);
    shell.closeDialogs();
    expect(shell.chordPro).toBeNull();
    expect(shell.dialogOpen).toBe(false);
  });
});

describe("batch 7: the Library page follows what it missed", () => {
  function libraryConn() {
    const asked: string[] = [];
    let synced: ((r: boolean) => void) | null = null;
    const answer = (what: string) => (asked.push(what), Promise.resolve(what === "/codex" ? { collection: "c1" } : []));
    const rest = new Proxy(
      { get: (path: string) => answer(path) },
      { get: (t, k: string) => (k in t ? t[k as "get"] : k === "then" ? undefined : () => answer(k)) },
    );
    const conn = {
      rest,
      socket: { isOnline: true },
      pushNotice: () => {},
      onSynced: (fn: (r: boolean) => void) => ((synced = fn), () => (synced = null)),
    } as unknown as Connection;
    return { conn, asked, sync: (r: boolean) => synced?.(r) };
  }

  it("reads its lists again after a reconnect or dropped events (A12-5)", async () => {
    const t = libraryConn();
    session.conn = t.conn;
    const off = libraryPage.attach();
    try {
      await vi.waitFor(() => expect(t.asked).toContain("presentations"));
      t.asked.length = 0;
      t.sync(true);
      await vi.waitFor(() => expect(t.asked).toContain("presentations"));
    } finally {
      off();
      session.conn = null;
    }
  });

  it("a collection deleted for good elsewhere leaves the collection trash list (X1-5)", async () => {
    const t = libraryConn();
    session.conn = t.conn;
    const off = libraryPage.attach();
    try {
      await vi.waitFor(() => expect(t.asked).toContain("presentations"));
      t.asked.length = 0;
      changes.emit({ type: "codexChanged", parts: ["status", "collectionTrash"], collection: null } as never);
      await vi.waitFor(() => expect(t.asked).toContain("/codex/collection-trash"));
    } finally {
      off();
      session.conn = null;
    }
  });
});

describe("batch 7: connecting from the main view and the Codex box", () => {
  const CODEX = "http://192.0.2.9:5555";
  const SHOW = "http://192.0.2.5:5555";
  const notices: string[] = [];
  const mainConn = () =>
    ({ rest: {}, socket: { isOnline: true }, stop: () => {}, pushNotice: (_l: string, m: string) => notices.push(m) }) as unknown as Connection;

  // Node has no localStorage: the remembered keys live in a map for these tests.
  const kv = new Map<string, string>();
  const store = { getItem: (k: string) => kv.get(k) ?? null, setItem: (k: string, v: string) => void kv.set(k, v), removeItem: (k: string) => void kv.delete(k) };
  let saved: unknown;
  beforeEach(() => {
    saved = (creds as unknown as { kv: unknown }).kv;
    (creds as unknown as { kv: unknown }).kv = store;
  });
  afterEach(() => {
    notices.length = 0;
    kv.clear();
    (creds as unknown as { kv: unknown }).kv = saved;
  });

  it("an address that doesn't answer is a notice on the main view; nothing is left for the connect screen (A12-3)", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("fetch failed")));
    const conn = mainConn();
    session.conn = conn;
    session.view = "main";
    await session.connectFromMain(CODEX, { codexBox: "hub" });
    expect(session.conn).toBe(conn);
    expect(session.view).toBe("main");
    expect(session.connectError).toBeNull();
    expect(notices).toEqual(["Could not reach a server at 192.0.2.9:5555. Check that the server is running and the address is right."]);
    session.conn = null;
  });

  it("the Codex box gets this client's site key; when it still needs pairing, the connect screen opens for it (A12-3)", async () => {
    const auth: (string | null)[] = [];
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
      auth.push(new Headers(init?.headers).get("Authorization"));
      return Promise.resolve(json(200, { ...SERVER, id: "hub", name: "Hub", auth: "unpaired" }));
    });
    session.conn = mainConn();
    session.server = { ...SERVER, id: "show", auth: "paired" };
    creds.setToken("show", "site-key");
    session.view = "main";
    try {
      await session.connectFromMain(CODEX, { codexBox: "hub" });
      expect(auth).toEqual([null, "Bearer site-key"]);
      expect(session.view).toBe("connect");
      expect(session.conn).toBeNull();
      expect(session.pairing?.server.name).toBe("Hub");
    } finally {
      creds.setToken("show", null);
      session.server = null;
    }
  });

  it("an Oriel on the show box itself pairs with the Codex box for the key, then goes back to the show box (X1-3)", async () => {
    const asked: string[] = [];
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
      const u = new URL(url);
      asked.push(`${init?.method ?? "GET"} ${u.origin}${u.pathname}`);
      if (u.pathname === "/api/v1/pair") return Promise.resolve(json(200, { token: "hub-key", clientId: "ab" }));
      if (u.origin === SHOW) return Promise.reject(new TypeError("fetch failed"));
      return Promise.resolve(json(200, { ...SERVER, id: "hub", name: "Hub", auth: "unpaired" }));
    });
    session.conn = mainConn();
    session.server = { ...SERVER, id: "show", auth: "local" };
    session.baseUrl = SHOW;
    session.view = "main";
    const p = session.pairWithCodexBox({ serverId: "hub", name: "Hub", addresses: [CODEX] });
    await vi.waitFor(() => expect(shell.prompt?.value).toBe(CODEX));
    shell.prompt!.resolve({ value: CODEX });
    await p;
    expect(session.pairing).toMatchObject({ server: { name: "Hub" }, returnTo: SHOW });
    await session.pair("123456");
    expect(creds.token("hub")).toBe("hub-key");
    // Back to the show box (unreachable in this test: the connect screen says so).
    expect(asked[asked.length - 1]).toBe(`GET ${SHOW}/api/v1/server`);
    expect(session.pairing).toBeNull();
    session.server = null;
  });

  it("Back from that pairing goes back to the show box too (X1-3)", async () => {
    const asked: string[] = [];
    vi.stubGlobal("fetch", (url: string) => (asked.push(new URL(url).origin), Promise.reject(new TypeError("fetch failed"))));
    session.pairing = { baseUrl: CODEX, server: { ...SERVER, id: "hub" }, returnTo: SHOW };
    session.cancelPairing();
    await vi.waitFor(() => expect(asked).toEqual([SHOW]));
  });

  it("the site key for the Codex box's side connection: the main one, else the one got there (X1-3)", () => {
    session.server = { ...SERVER, id: "show", auth: "local" };
    expect(session.siteKey("hub")).toBeNull();
    creds.setToken("hub", "hub-key");
    expect(session.siteKey("hub")).toBe("hub-key");
    creds.setToken("show", "site-key");
    expect(session.siteKey("hub")).toBe("site-key");
    creds.setToken("show", null);
    session.server = null;
  });

  it("an open presentation removed without a client: deleted, unless the box shows another collection now (X1-6)", async () => {
    let collection = "c1";
    const conn = {
      rest: { codex: () => Promise.resolve({ collection }), libraries: () => Promise.resolve([]), library: () => Promise.resolve([]) },
      socket: { isOnline: true },
      pushNotice: (_l: string, m: string) => notices.push(m),
    } as unknown as Connection;
    session.conn = conn;
    session.server = { ...SERVER, auth: "paired", capabilities: ["codex"] };
    const removed = (id: string) =>
      (session as unknown as { documentChanged(k: string, id: string, m: unknown): Promise<void> }).documentChanged("presentation", id, {
        type: "documentChanged",
        kind: "presentation",
        id,
        change: "removed",
        collection: "c1",
      });
    try {
      session.open = { presentation: { id: "p1", name: "Song One" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
      await removed("p1");
      await vi.waitFor(() => expect(notices).toEqual(["“Song One” was deleted by another operator."]));
      collection = "c2";
      session.open = { presentation: { id: "p2", name: "Song Two" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
      await removed("p2");
      await vi.waitFor(() => expect(notices[1]).toBe("“Song Two” was closed: it isn't in the collection this box shows now."));
    } finally {
      session.conn = null;
      session.server = null;
    }
  });
});
