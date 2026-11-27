import { describe, expect, it } from "vitest";
import { RestClient } from "../rest";
import { ScriptureClient } from "../scripture";

describe("ScriptureClient", () => {
  it("asks the scripture routes and builds the preview picture's URL", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchFake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response("{}", { status: 200 });
    }) as unknown as typeof fetch;
    const c = new ScriptureClient(new RestClient({ baseUrl: "http://booth:5555", token: () => "tok", fetch: fetchFake }));
    await c.books("TST");
    await c.parse("TST", "Ps 23:1-4, 6");
    await c.search("TST", "quiet road", 10);
    await c.create({ translation: "TST", reference: "John 3:16", verseNumbers: false, referenceOn: "each", then: "show" });
    await c.resplit("p1", true);
    expect(calls.map((x) => x.url)).toEqual([
      "http://booth:5555/api/v1/scripture/translations/TST/books",
      "http://booth:5555/api/v1/scripture/parse?translation=TST&q=Ps+23%3A1-4%2C+6",
      "http://booth:5555/api/v1/scripture/search?translation=TST&q=quiet+road&limit=10",
      "http://booth:5555/api/v1/scripture/presentations",
      "http://booth:5555/api/v1/scripture/presentations/p1/resplit",
    ]);
    expect(JSON.parse(calls[4]!.init.body as string)).toEqual({ force: true });
    expect(calls[3]!.init.method).toBe("POST");
    expect(JSON.parse(calls[3]!.init.body as string).then).toBe("show");
    expect(c.previewUrl({ translation: "TST", reference: "John 3:16", slide: 0, w: 320 })).toBe(
      "http://booth:5555/api/v1/scripture/preview?translation=TST&reference=John+3%3A16&slide=0&w=320&token=tok",
    );
  });
});
