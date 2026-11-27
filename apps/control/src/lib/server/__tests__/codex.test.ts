import { describe, expect, it } from "vitest";
import type { CodexBox } from "$protocol";
import { CodexClient, assetBlob, codexFailure, forbiddenText } from "../codex";
import { CodexBoxError, CodexBoxLink, boxName } from "../codexBox";
import { ApiRequestError, RestClient } from "../rest";

type Call = { url: string; method: string; body: unknown; headers: Record<string, string> };

/** A fake fetch answering by "METHOD URL" (a function or a [status, body] pair). */
function fakeFetch(routes: Record<string, [number, unknown] | (() => [number, unknown]) | "down">, calls: Call[] = []) {
  return (async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? "GET";
    calls.push({ url, method, body: init.body ? JSON.parse(String(init.body)) : undefined, headers: (init.headers ?? {}) as Record<string, string> });
    const key = `${method} ${url}`;
    const r = routes[key];
    if (r === "down" || r === undefined) throw new TypeError("fetch failed");
    const [status, body] = typeof r === "function" ? r() : r;
    return new Response(body === undefined ? "" : JSON.stringify(body), { status });
  }) as unknown as typeof fetch;
}

const API = "http://show:5555/api/v1";

describe("CodexClient requests", () => {
  it("builds the routes, queries and bodies", async () => {
    const calls: Call[] = [];
    const fetch = fakeFetch(
      {
        [`GET ${API}/codex`]: [200, { id: "c" }],
        [`GET ${API}/codex/collections`]: [200, []],
        [`PUT ${API}/codex/assignment`]: [200, { id: "c" }],
        [`GET ${API}/codex/trash`]: [200, []],
        [`POST ${API}/codex/trash/t%201/restore`]: [200, { collection: "a", kind: "presentation", documents: ["p"] }],
        [`DELETE ${API}/codex/trash`]: [200, { purged: 3 }],
        [`GET ${API}/codex/collections/col/media?status=online%2Cmissing`]: [200, []],
        [`GET ${API}/codex/collections/col/media`]: [200, []],
        [`GET ${API}/codex/search?q=open+gate&collection=all&kinds=presentation&limit=40`]: [200, { hits: [], total: 0 }],
        [`GET ${API}/codex/storage`]: [200, {}],
        [`GET ${API}/codex/library?q=loop&kind=video`]: [200, { items: [], total: 0 }],
        [`GET ${API}/codex/library`]: [200, { items: [], total: 0 }],
        [`GET ${API}/media/m%201/used-by`]: [200, [{ kind: "presentation", id: "p", name: "Song" }]],
        [`GET ${API}/props/p1/used-by`]: [200, []],
      },
      calls,
    );
    const c = new CodexClient(new RestClient({ baseUrl: "http://show:5555", token: () => "key", fetch }));
    await c.status();
    await c.collections();
    await c.assign("col");
    await c.assign("col", true);
    await c.trash();
    expect(await c.restoreTrash("t 1")).toMatchObject({ documents: ["p"] });
    expect(await c.emptyTrash()).toEqual({ purged: 3 });
    await c.media("col", ["online", "missing"]);
    await c.media("col");
    await c.search("open gate", { collection: "all", kinds: ["presentation"], limit: 40 });
    await c.storage();
    await c.libraryItems({ q: "loop", kind: "video" });
    await c.libraryItems();
    expect(await c.usedBy("media", "m 1")).toEqual([{ kind: "presentation", id: "p", name: "Song" }]);
    expect(await c.usedBy("prop", "p1")).toEqual([]);
    expect(calls.map((x) => `${x.method} ${x.url.replace(API, "")}`)).toEqual([
      "GET /codex",
      "GET /codex/collections",
      "PUT /codex/assignment",
      "PUT /codex/assignment",
      "GET /codex/trash",
      "POST /codex/trash/t%201/restore",
      "DELETE /codex/trash",
      "GET /codex/collections/col/media?status=online%2Cmissing",
      "GET /codex/collections/col/media",
      "GET /codex/search?q=open+gate&collection=all&kinds=presentation&limit=40",
      "GET /codex/storage",
      "GET /codex/library?q=loop&kind=video",
      "GET /codex/library",
      "GET /media/m%201/used-by",
      "GET /props/p1/used-by",
    ]);
    expect(calls[2]!.body).toEqual({ collection: "col" });
    expect(calls[3]!.body).toEqual({ collection: "col", confirm: true });
    expect(calls[0]!.headers.Authorization).toBe("Bearer key");
    expect(c.blobThumbnailUrl("ab12", "png")).toBe(`${API}/codex/blobs/ab12.png/thumbnail?token=key`);
    expect(c.blobUrl("ab12", "mp4")).toBe(`${API}/codex/blobs/ab12.mp4?token=key`);
  });

  it("finds an asset's file by SHA-256", () => {
    expect(assetBlob({ name: "Dawn.JPG", location: { type: "library", id: "i", sha256: "aa" } })).toEqual({ sha256: "aa", ext: "jpg" });
    expect(assetBlob({ name: "x", location: { type: "managed", sha256: "bb", extension: "mov" } })).toEqual({ sha256: "bb", ext: "mov" });
    expect(assetBlob({ name: "x.png", location: { type: "linked", root: "r", path: "x.png" } })).toBeNull();
    expect(assetBlob({ name: "x.png" })).toBeNull();
  });
});

describe("codexFailure", () => {
  const box: CodexBox = { serverId: "hub", name: "Hub", addresses: ["http://hub:5555"] };

  it("maps the answers the Library prompts for", async () => {
    const fetch = fakeFetch({
      [`PUT ${API}/codex/assignment`]: [409, { error: { code: "liveShow", message: "a slide is live", live: ["a slide of “Song”"] } }],
      [`DELETE ${API}/codex/trash`]: [409, { error: { code: "codexElsewhere", message: "managed on Hub" }, codex: box }],
      [`POST ${API}/codex/trash/x/restore`]: [403, { error: { code: "forbidden", message: "admins only" } }],
      [`GET ${API}/codex/collections`]: [503, { error: { code: "unavailable", message: "the site's Codex can't be reached" } }],
      [`GET ${API}/codex/trash`]: [409, { error: { code: "conflict", message: "its library is gone" } }],
      [`GET ${API}/codex/storage`]: "down",
    });
    const c = new CodexClient(new RestClient({ baseUrl: "http://show:5555", token: () => null, fetch }));
    const fail = (p: Promise<unknown>) => p.then(() => null, codexFailure);
    expect(await fail(c.assign("col"))).toEqual({ kind: "liveShow", message: "a slide is live", live: ["a slide of “Song”"] });
    expect(await fail(c.emptyTrash())).toEqual({ kind: "elsewhere", box, message: "managed on Hub" });
    expect(await fail(c.restoreTrash("x"))).toEqual({ kind: "forbidden", message: "admins only" });
    expect(await fail(c.collections())).toMatchObject({ kind: "unavailable", box: null });
    expect(await fail(c.trash())).toMatchObject({ kind: "conflict" });
    expect(await fail(c.storage())).toMatchObject({ kind: "offline" });
    expect(codexFailure(new Error("boom"))).toEqual({ kind: "other", message: "boom" });
    expect(forbiddenText("Switching the collection")).toMatch(/^Switching the collection is for the site's admins/);
  });
});

describe("CodexBoxLink", () => {
  const box: CodexBox = { serverId: "hub", name: "Hub", addresses: ["http://gone:5555", "http://other:5555", "http://hub:5555"] };
  const routes = {
    [`DELETE ${API}/codex/trash`]: [409, { error: { code: "codexElsewhere", message: "managed elsewhere" }, codex: box }] as [number, unknown],
    "GET http://gone:5555/api/v1/server": "down" as const,
    "GET http://other:5555/api/v1/server": [200, { id: "someone-else" }] as [number, unknown],
    "GET http://hub:5555/api/v1/server": [200, { id: "hub" }] as [number, unknown],
    "DELETE http://hub:5555/api/v1/codex/trash": [200, { purged: 2 }] as [number, unknown],
  };

  function setup(answer: boolean, r: typeof routes | Record<string, unknown> = routes) {
    const calls: Call[] = [];
    const fetch = fakeFetch(r as Parameters<typeof fakeFetch>[0], calls);
    const asked: string[] = [];
    const unreachable: string[] = [];
    const link = new CodexBoxLink({
      token: () => "site-key",
      clientId: () => "client-1",
      fetch,
      prompts: {
        ask: async (b) => (asked.push(boxName(b)), answer),
        unreachable: async (b) => void unreachable.push(boxName(b)),
      },
    });
    const main = new CodexClient(new RestClient({ baseUrl: "http://show:5555", token: () => "site-key", fetch }));
    return { link, main, calls, asked, unreachable };
  }

  it("an Oriel on the show box itself (no key): says so and offers pairing; nothing is sent there (X1-3)", async () => {
    const calls: Call[] = [];
    const fetch = fakeFetch(routes, calls);
    const unpaired: string[] = [];
    const asked: string[] = [];
    const link = new CodexBoxLink({
      token: () => null,
      fetch,
      prompts: { ask: async (b) => (asked.push(boxName(b)), true), unreachable: async () => {}, unpaired: async (b) => void unpaired.push(boxName(b)) },
    });
    const main = new CodexClient(new RestClient({ baseUrl: "http://show:5555", token: () => null, fetch }));
    await expect(link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash())).rejects.toMatchObject({ reason: "unpaired" });
    expect(unpaired).toEqual(["Hub"]);
    expect(asked).toEqual([]);
    expect(calls.map((c) => c.url)).toEqual([`${API}/codex/trash`]);
  });

  it("the key this client got at the Codex box is sent there when the main connection has none (X1-3)", async () => {
    const calls: Call[] = [];
    const fetch = fakeFetch(routes, calls);
    const link = new CodexBoxLink({
      token: (b) => (b?.serverId === "hub" ? "hub-key" : null),
      fetch,
      prompts: { ask: async () => true, unreachable: async () => {}, unpaired: async () => {} },
    });
    const main = new CodexClient(new RestClient({ baseUrl: "http://show:5555", token: () => null, fetch }));
    expect(await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash())).toEqual({ purged: 2 });
    const sent = calls.find((c) => c.url === "http://hub:5555/api/v1/codex/trash")!;
    expect(sent.headers.Authorization).toBe("Bearer hub-key");
  });

  it("asks once, then sends the request to the Codex box with the site key", async () => {
    const { link, main, calls, asked } = setup(true);
    const seen: (string | null)[] = [];
    link.subscribe((b) => seen.push(b?.name ?? null));
    expect(await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash())).toEqual({ purged: 2 });
    expect(asked).toEqual(["Hub"]);
    const sent = calls.find((c) => c.url === "http://hub:5555/api/v1/codex/trash")!;
    expect(sent.method).toBe("DELETE");
    expect(sent.headers.Authorization).toBe("Bearer site-key");
    expect(sent.headers["X-Client-Id"]).toBe("client-1");
    expect(seen).toEqual([null, "Hub"]);
    // The second time: no question, the same address.
    await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash());
    expect(asked).toEqual(["Hub"]);
    expect(calls.filter((c) => c.url.endsWith("/server")).length).toBe(3);
  });

  it("sends nothing when the operator says no", async () => {
    const { link, main, calls } = setup(false);
    const err = await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash()).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(CodexBoxError);
    expect((err as CodexBoxError).reason).toBe("declined");
    expect(calls.some((c) => c.url.startsWith("http://hub"))).toBe(false);
  });

  it("says so when no address answers", async () => {
    const { link, main, unreachable } = setup(true, { ...routes, "GET http://hub:5555/api/v1/server": "down" });
    const err = await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash()).catch((e: unknown) => e);
    expect((err as CodexBoxError).reason).toBe("unreachable");
    expect(unreachable).toEqual(["Hub"]);
    expect(link.box).toBeNull();
  });

  it("passes other errors through and needs the box's name", async () => {
    const { link, main } = setup(true, { [`DELETE ${API}/codex/trash`]: [403, { error: { code: "forbidden", message: "no" } }] });
    const err = await link.run(main.rest, (rest) => new CodexClient(rest).emptyTrash()).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiRequestError);
    const unknownBox = setup(true, { [`DELETE ${API}/codex/trash`]: [409, { error: { code: "codexElsewhere", message: "?" } }] });
    const e2 = await unknownBox.link.run(unknownBox.main.rest, (rest) => new CodexClient(rest).emptyTrash()).catch((e: unknown) => e);
    expect((e2 as CodexBoxError).reason).toBe("unknown");
    expect(boxName(null)).toBe("the site's Codex box");
  });
});

describe("CodexBackupsClient requests", () => {
  it("asks for a restore preview onto another box with ?target= (R1-2)", async () => {
    const { CodexBackupsClient } = await import("../codexBackups");
    const calls: Call[] = [];
    const preview = { snapshot: {}, changes: [], unchanged: 0, summary: "", restart: [] };
    const fetch = fakeFetch(
      {
        [`GET ${API}/codex/backups/set%201/snapshots/s1/preview`]: [200, preview],
        [`GET ${API}/codex/backups/set%201/snapshots/s1/preview?target=srv-new`]: [200, preview],
      },
      calls,
    );
    const c = new CodexBackupsClient(new RestClient({ baseUrl: "http://show:5555", token: () => "key", fetch }));
    await c.preview("set 1", "s1");
    await c.preview("set 1", "s1", "srv-new");
    expect(calls.map((x) => x.url)).toEqual([`${API}/codex/backups/set%201/snapshots/s1/preview`, `${API}/codex/backups/set%201/snapshots/s1/preview?target=srv-new`]);
  });
});
