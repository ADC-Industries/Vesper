import { describe, expect, it } from "vitest";
import { RestClient } from "../rest";
import { SiteClient } from "../site";

type Call = { url: string; method: string; body: unknown; headers: Record<string, string> };

describe("SiteClient.saveSettings", () => {
  it("sends whole settings: the unchanged fields as read, with If-Match", async () => {
    const calls: Call[] = [];
    const current = { enabled: true, joinMode: "code_or_approval", access: "open", pinSet: false, revision: "r7" };
    const fakeFetch = (async (url: string, init: RequestInit = {}) => {
      const method = init.method ?? "GET";
      calls.push({ url, method, body: init.body ? JSON.parse(String(init.body)) : undefined, headers: (init.headers ?? {}) as Record<string, string> });
      return new Response(JSON.stringify(method === "GET" ? current : { ...current, access: "pin", pinSet: true, revision: "r8" }), { status: 200 });
    }) as unknown as typeof fetch;
    const s = new SiteClient(new RestClient({ baseUrl: "http://show:5555", token: () => null, fetch: fakeFetch }));
    await s.saveSettings({ access: "pin", pin: "2468" }, "r7");
    const put = calls.find((c) => c.method === "PUT")!;
    expect(put.url).toBe("http://show:5555/api/v1/site/settings");
    expect(put.body).toEqual({ enabled: true, joinMode: "code_or_approval", access: "pin", pin: "2468" });
    expect(Object.entries(put.headers).find(([k]) => k.toLowerCase() === "if-match")?.[1]).toContain("r7");
    await s.saveSettings({ joinMode: "approval" }, "r8");
    expect(calls.filter((c) => c.method === "PUT")[1]?.body).toEqual({ enabled: true, joinMode: "approval", access: "open" });
  });
});
