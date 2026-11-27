import { describe, expect, it } from "vitest";
import { ApiRequestError, RestClient, normaliseBaseUrl } from "../rest";

describe("normaliseBaseUrl", () => {
  it("adds the default port to bare hosts only", () => {
    expect(normaliseBaseUrl("booth.local", 5555)).toBe("http://booth.local:5555");
    expect(normaliseBaseUrl(" 10.0.0.5 ", 5555)).toBe("http://10.0.0.5:5555");
    expect(normaliseBaseUrl("10.0.0.5:6000", 5555)).toBe("http://10.0.0.5:6000");
    expect(normaliseBaseUrl("http://localhost:5173/", 5555)).toBe("http://localhost:5173");
    expect(normaliseBaseUrl("https://vesper.example.org", 5555)).toBe("https://vesper.example.org");
    expect(() => normaliseBaseUrl("  ", 5555)).toThrow();
  });
});

describe("RestClient", () => {
  it("sends the bearer token and builds token URLs for images and the WebSocket", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchFake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ revision: 1, state: {} }), { status: 200 });
    }) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555/", token: () => "tok", fetch: fetchFake });
    await c.state();
    expect(calls[0]!.url).toBe("http://booth:5555/api/v1/state");
    expect((calls[0]!.init.headers as Record<string, string>).Authorization).toBe("Bearer tok");
    expect(c.thumbnailUrl("p", "s", 320)).toBe("http://booth:5555/api/v1/presentations/p/slides/s/thumbnail?w=320&token=tok");
    expect(c.wsUrl(1)).toBe("ws://booth:5555/api/v1/ws?protocolVersion=1&token=tok");
  });

  it("builds Codex search queries", async () => {
    const urls: string[] = [];
    const fetchFake = (async (url: string) => {
      urls.push(url);
      return new Response(JSON.stringify({ hits: [], total: 0 }), { status: 200 });
    }) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fetchFake });
    expect(await c.codexSearch("morning star")).toEqual({ hits: [], total: 0 });
    await c.codexSearch('"open fiel', { collection: "all", kinds: ["presentation", "timer"], limit: 20 });
    expect(urls[0]).toBe("http://booth:5555/api/v1/codex/search?q=morning+star");
    expect(urls[1]).toBe(
      "http://booth:5555/api/v1/codex/search?q=%22open+fiel&collection=all&kinds=presentation%2Ctimer&limit=20",
    );
  });

  it("turns error bodies into ApiRequestError", async () => {
    const fetchFake = (async () =>
      new Response(JSON.stringify({ error: { code: "notFound", message: "presentation x not found" } }), {
        status: 404,
      })) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fetchFake });
    const err = await c.presentation("x").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiRequestError);
    expect(err).toMatchObject({ code: "notFound", status: 404 });
  });

  it("reports network failures as code 'network'", async () => {
    const fetchFake = (async () => {
      throw new TypeError("fetch failed");
    }) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fetchFake });
    await expect(c.serverInfo()).rejects.toMatchObject({ code: "network" });
  });

  it("reports a body cut off mid-way as 'network', and a stalled body as 'timeout'", async () => {
    // The headers came; the link then dropped while the body was read.
    const cut = (async () =>
      new Response(
        new ReadableStream({
          start: (c) => c.error(new TypeError("network error")),
        }),
        { status: 200 },
      )) as unknown as typeof fetch;
    const a = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: cut });
    await expect(a.serverInfo()).rejects.toMatchObject({ name: "ApiRequestError", code: "network" });

    // The headers came; the body never ends. As a browser does, aborting errors the body.
    const stall = (async (_url: string, init: RequestInit) =>
      new Response(
        new ReadableStream({
          start: (c) => init.signal?.addEventListener("abort", () => c.error(new DOMException("aborted", "AbortError"))),
        }),
        { status: 200 },
      )) as unknown as typeof fetch;
    const b = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: stall, timeoutMs: 30 });
    await expect(b.serverInfo()).rejects.toMatchObject({ name: "ApiRequestError", code: "timeout" });
  });

  it("sends If-Match on edits and surfaces the field and the current document", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchFake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      if (init.method === "PUT")
        return new Response(
          JSON.stringify({ error: { code: "conflict", message: "changed" }, current: { id: "s", revision: "r2" } }),
          { status: 409 },
        );
      return new Response(JSON.stringify({ error: { code: "badRequest", message: "dup", field: "transport.sourceName" } }), {
        status: 400,
      });
    }) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fetchFake });
    const conflict = await c.updateScreen({ schemaVersion: 1, id: "s" }, "r1").catch((e: unknown) => e);
    expect((calls[0]!.init.headers as Record<string, string>)["If-Match"]).toBe('"r1"');
    expect(calls[0]!.url).toBe("http://booth:5555/api/v1/screens/s");
    expect(conflict).toMatchObject({ status: 409, code: "conflict", current: { revision: "r2" } });
    const bad = await c.createOutput({ screen: "s" }).catch((e: unknown) => e);
    expect((calls[1]!.init.headers as Record<string, string>)["If-Match"]).toBeUndefined();
    expect(bad).toMatchObject({ status: 400, field: "transport.sourceName" });
  });

  it("reads and saves the audio device configuration", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchFake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(init.body ?? JSON.stringify([]), { status: 200 });
    }) as unknown as typeof fetch;
    const c = new RestClient({ baseUrl: "http://booth:5555", token: () => null, fetch: fetchFake });
    await c.audioDevices();
    const saved = await c.saveAudioDevice({ schemaVersion: 1, device: "hw:1,0", masterGainDb: -3 });
    expect(calls[0]!.url).toBe("http://booth:5555/api/v1/audio-devices");
    expect(calls[1]!.url).toBe("http://booth:5555/api/v1/audio-device");
    expect(calls[1]!.init.method).toBe("PUT");
    expect(saved).toEqual({ schemaVersion: 1, device: "hw:1,0", masterGainDb: -3 });
  });
});
