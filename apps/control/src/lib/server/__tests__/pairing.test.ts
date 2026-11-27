import { describe, expect, it } from "vitest";
import { RestClient } from "../rest";
import { awaitPairing, isPending, pairingEndedText } from "../pairing";
import { ApiRequestError } from "../rest";

const BASE = "http://192.0.2.5:5555";

/** A fake server answering each request from `answers`, in turn. */
function server(answers: (() => Response)[]) {
  const calls: { path: string; method: string; body: unknown }[] = [];
  const fetchFake = (async (url: string, init: RequestInit) => {
    calls.push({
      path: new URL(url).pathname,
      method: init.method ?? "GET",
      body: init.body ? JSON.parse(init.body as string) : undefined,
    });
    const a = answers.shift();
    if (!a) throw new TypeError("fetch failed");
    return a();
  }) as unknown as typeof fetch;
  return { calls, rest: new RestClient({ baseUrl: BASE, token: () => null, fetch: fetchFake }) };
}

const json = (status: number, v: unknown) => () => new Response(JSON.stringify(v), { status });
const waiting = json(202, { pairingId: "s3cr3t", expiresInSeconds: 600 });

describe("pairing", () => {
  it("sends the code only when typed, and says when this is a browser", async () => {
    const { calls, rest } = server([waiting, json(200, { token: "idn1.k", clientId: "ab" })]);
    const first = await rest.pair(null, "Vesper Oriel", true);
    expect(isPending(first)).toBe(true);
    const second = await rest.pair("482913", "Vesper Oriel");
    expect(isPending(second)).toBe(false);
    expect(calls.map((c) => c.body)).toEqual([
      { clientName: "Vesper Oriel", browser: true },
      { code: "482913", clientName: "Vesper Oriel" },
    ]);
  });

  it("waits until approved, asking again through a moment offline", async () => {
    const { calls, rest } = server([
      waiting,
      () => {
        throw new TypeError("offline");
      },
      json(200, { token: "idn1.k", clientId: "ab" }),
    ]);
    const got = await awaitPairing(rest, { pairingId: "s3cr3t", expiresInSeconds: 600 }, { cancelled: false }, 1);
    expect(got).toEqual({ token: "idn1.k", clientId: "ab" });
    expect(calls.every((c) => c.method === "GET" && c.path === "/api/v1/pair/s3cr3t")).toBe(true);
  });

  it("stops when cancelled", async () => {
    const { rest } = server([waiting, waiting, waiting]);
    const ctl = { cancelled: false };
    const p = awaitPairing(rest, { pairingId: "s3cr3t", expiresInSeconds: 600 }, ctl, 5);
    ctl.cancelled = true;
    expect(await p).toBeNull();
  });

  it("says clearly when refused or ended unanswered", async () => {
    const refused = server([
      json(403, { error: { code: "forbidden", message: "the site's administrator refused this pairing: not now" } }),
    ]);
    const e = await awaitPairing(refused.rest, { pairingId: "x", expiresInSeconds: 1 }, { cancelled: false }, 1).catch(
      (x: unknown) => x,
    );
    expect(e).toBeInstanceOf(ApiRequestError);
    expect(pairingEndedText(e)).toBe("The site's administrator refused this computer (not now).");
    expect(pairingEndedText(new ApiRequestError("forbidden", "the site's administrator refused this pairing: not approved", 403))).toBe(
      "The site's administrator refused this computer.",
    );
    expect(pairingEndedText(new ApiRequestError("notFound", "no such pairing", 404))).toMatch(/in time/);
    expect(pairingEndedText(new ApiRequestError("unauthorized", "x", 401))).toBeNull();
  });
});
