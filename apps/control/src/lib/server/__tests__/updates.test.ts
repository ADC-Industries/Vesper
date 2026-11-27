import { afterEach, describe, expect, it, vi } from "vitest";
import { RestClient } from "../rest";
import { UpdatesClient, base64Text } from "../updates";

afterEach(() => vi.unstubAllGlobals());

describe("UpdatesClient", () => {
  it("queues an action for a machine", async () => {
    const calls: { url: string; method: string; body: unknown }[] = [];
    const fakeFetch = (async (url: string, init: RequestInit = {}) => {
      calls.push({ url, method: init.method ?? "GET", body: init.body ? JSON.parse(String(init.body)) : undefined });
      return new Response(JSON.stringify({ report: {}, pending: [] }), { status: 202 });
    }) as unknown as typeof fetch;
    const c = new UpdatesClient(new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fakeFetch }));
    await c.queue("m 1", { type: "update", product: "vesper", version: "0.2.0" });
    expect(calls[0]).toEqual({ url: "http://booth:5555/api/v1/updates/launchers/m%201/actions", method: "POST", body: { type: "update", product: "vesper", version: "0.2.0" } });
  });

  it("uploads a package with its signature, base64 in a header", async () => {
    const sig = "untrusted comment: signature\nRUQ=\ntrusted comment: vesper-package product=vesper version=0.2.0 platform=linux-x86_64\nAAAA\n";
    let seen: { url: string; headers: Record<string, string>; body: unknown } | null = null;
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      seen = { url, headers: init.headers as Record<string, string>, body: init.body };
      return new Response(JSON.stringify({ id: "abc", product: "vesper", version: "0.2.0" }), { status: 201 });
    });
    const c = new UpdatesClient(new RestClient({ baseUrl: "http://booth:5555", token: () => "key1" }));
    const file = new Blob(["package bytes"]);
    const p = await c.upload(file, "vesper_0.2.0-1_amd64.deb", sig, { token: () => "key1", clientId: "c1" });
    expect(p.id).toBe("abc");
    expect(seen!.url).toBe("http://booth:5555/api/v1/updates/packages?file=vesper_0.2.0-1_amd64.deb");
    expect(atob(seen!.headers["X-Vesper-Signature"] ?? "")).toBe(sig);
    expect(seen!.headers.Authorization).toBe("Bearer key1");
    expect(seen!.body).toBe(file);
  });

  it("turns a refused upload into an error with the server's words", async () => {
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ error: { code: "badRequest", message: "the package was refused: the signature doesn't match the package" } }), { status: 400 }));
    const c = new UpdatesClient(new RestClient({ baseUrl: "http://booth:5555", token: () => null }));
    await expect(c.upload(new Blob(["x"]), "p", "sig", { token: () => null })).rejects.toThrow(/doesn't match/);
  });

  it("encodes non-ASCII signature comments", () => {
    expect(new TextDecoder().decode(Uint8Array.from(atob(base64Text("é")), (ch) => ch.charCodeAt(0)))).toBe("é");
  });
});
