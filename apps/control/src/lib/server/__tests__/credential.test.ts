import { describe, expect, it } from "vitest";
import type { ServerInfo } from "$protocol";
import { settleCredential } from "../credential";

const BASE = "http://192.0.2.5:5555";
const KEY = "idn1.claims.signature";

function info(extra: Partial<ServerInfo> = {}): ServerInfo {
  return {
    id: "srv",
    name: "Booth",
    version: "0.1.0",
    protocolVersion: 1,
    capabilities: [],
    auth: "paired",
    ...extra,
  };
}

/** A fake server: who sent what, and its answers. */
function server(answer: (path: string, auth: string | null) => Response) {
  const calls: { path: string; method: string; auth: string | null; body?: unknown }[] = [];
  const fetchFake = (async (url: string, init: RequestInit) => {
    const path = new URL(url).pathname;
    const auth = (init.headers as Record<string, string>).Authorization ?? null;
    const body = init.body ? JSON.parse(init.body as string) : undefined;
    calls.push({ path, method: init.method ?? "GET", auth, ...(body !== undefined ? { body } : {}) });
    return answer(path, auth);
  }) as unknown as typeof fetch;
  return { calls, fetchFake };
}

function store() {
  const saved = new Map<string, string | null>();
  return { saved, setToken: (id: string, t: string | null) => void saved.set(id, t) };
}

describe("settleCredential", () => {
  it("swaps an old credential for the site key once, stores it and uses it", async () => {
    const { calls, fetchFake } = server((path, auth) =>
      path === "/api/v1/pair/upgrade"
        ? new Response(JSON.stringify({ token: KEY, clientId: "ab12" }), { status: 200 })
        : new Response(JSON.stringify(info({ upgradeCredential: auth === "Bearer old" ? true : undefined })), {
            status: 200,
          }),
    );
    const s = store();
    const r = await settleCredential(BASE, info({ upgradeCredential: true }), "old", s, { fetch: fetchFake });
    expect(r.token).toBe(KEY);
    expect(r.info.upgradeCredential).toBeUndefined();
    expect(s.saved.get("srv")).toBe(KEY);
    expect(calls.map((c) => [c.method, c.path, c.auth])).toEqual([
      ["POST", "/api/v1/pair/upgrade", "Bearer old"],
      ["GET", "/api/v1/server", `Bearer ${KEY}`],
    ]);
  });

  it("says when this is a web browser, so its key lives a year (not ten)", async () => {
    const { calls, fetchFake } = server((path) =>
      path === "/api/v1/pair/upgrade"
        ? new Response(JSON.stringify({ token: KEY, clientId: "ab12" }), { status: 200 })
        : new Response(JSON.stringify(info()), { status: 200 }),
    );
    await settleCredential(BASE, info({ upgradeCredential: true }), "old", store(), { fetch: fetchFake, browser: true });
    expect(calls[0]).toMatchObject({ path: "/api/v1/pair/upgrade", body: { browser: true } });
    calls.length = 0;
    await settleCredential(BASE, info({ upgradeCredential: true }), "old", store(), { fetch: fetchFake });
    expect(calls[0]!.body).toBeUndefined();
  });

  it("leaves a key, the server machine and an unpaired client alone", async () => {
    const { calls, fetchFake } = server(() => new Response("{}", { status: 500 }));
    const s = store();
    for (const [i, t] of [
      [info(), KEY],
      [info({ auth: "local" }), null],
      [info({ auth: "unpaired", upgradeCredential: true }), "old"],
    ] as const) {
      const r = await settleCredential(BASE, i, t, s, { fetch: fetchFake });
      expect(r).toEqual({ info: i, token: t });
    }
    expect(calls).toEqual([]);
    expect(s.saved.size).toBe(0);
  });

  it("keeps the old credential when the swap fails, to try again next time", async () => {
    const { fetchFake } = server(
      () =>
        new Response(JSON.stringify({ error: { code: "notImplemented", message: "pair at the site's server" } }), {
          status: 501,
        }),
    );
    const s = store();
    const before = info({ upgradeCredential: true });
    const r = await settleCredential(BASE, before, "old", s, { fetch: fetchFake });
    expect(r).toEqual({ info: before, token: "old" });
    expect(s.saved.size).toBe(0);
  });
});
