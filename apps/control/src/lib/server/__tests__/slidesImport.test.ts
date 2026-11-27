import { afterEach, describe, expect, it, vi } from "vitest";
import type { PptxImportJob, PptxImportReport, PptxSlideReport } from "$protocol";
import { ApiRequestError, RestClient } from "../rest";
import { fileProblem, finished, importJob, nameFromFile, progressFraction, progressText, slideNotes, slideStatus, startImport, summary } from "../slidesImport";

const job = (over: Partial<PptxImportJob> = {}): PptxImportJob => ({
  id: "j1",
  collection: "c1",
  fileName: "Spring talk.pptx",
  state: "reading",
  done: 2,
  total: 10,
  createdAt: "2026-11-26T08:00:00Z",
  ...over,
});

const slide = (over: Partial<PptxSlideReport> = {}): PptxSlideReport => ({ number: 1, editable: 3, ...over });

afterEach(() => vi.unstubAllGlobals());

describe("choosing a file", () => {
  it("takes .pptx and its kin, and says what to do with the others", () => {
    expect(fileProblem("Talk.pptx", 100)).toBeNull();
    expect(fileProblem("Loop.PPSX", 100)).toBeNull();
    expect(fileProblem("Talk.key", 100)).toMatch(/Export To ▸ PowerPoint/);
    expect(fileProblem("Old.ppt", 100)).toMatch(/save it as \.pptx/);
    expect(fileProblem("Doc.odp", 100)).toMatch(/OpenDocument/);
    expect(fileProblem("notes.txt", 100)).toMatch(/Google Slides/);
    expect(fileProblem("Empty.pptx", 0)).toMatch(/empty/);
    expect(fileProblem("Huge.pptx", 2 * 1024 ** 3)).toMatch(/1 GB/);
  });

  it("names the presentation after the file", () => {
    expect(nameFromFile("Easter Sunday.pptx")).toBe("Easter Sunday");
    expect(nameFromFile("C:\\decks\\Youth night.v2.pptx")).toBe("Youth night.v2");
    expect(nameFromFile(".pptx")).toBe(".pptx");
  });
});

describe("the requests", () => {
  it("sends the file as the body with its options and the site key", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify(job({ state: "queued" })), { status: 202 });
    });
    const rest = new RestClient({ baseUrl: "http://codex:5555", token: () => null });
    const file = new Blob(["PK..."]);
    const j = await startImport(rest, { token: () => "site-key", clientId: "oriel-1" }, file, "Spring talk.pptx", {
      collection: "c1",
      library: null,
      name: "  Spring  ",
    });
    expect(j.state).toBe("queued");
    const u = new URL(calls[0]!.url);
    expect(u.pathname).toBe("/api/v1/codex/imports/pptx");
    expect(Object.fromEntries(u.searchParams)).toEqual({ fileName: "Spring talk.pptx", collection: "c1", name: "Spring" });
    expect(calls[0]!.init.method).toBe("POST");
    expect(calls[0]!.init.body).toBe(file);
    const h = calls[0]!.init.headers as Record<string, string>;
    expect(h.Authorization).toBe("Bearer site-key");
    expect(h["X-Client-Id"]).toBe("oriel-1");
    expect(h["Content-Type"]).toBe("application/octet-stream");
  });

  it("turns refusals into ApiRequestErrors, codexElsewhere with its box", async () => {
    vi.stubGlobal(
      "fetch",
      async () => new Response(JSON.stringify({ error: { code: "codexElsewhere", message: "on the Codex box" }, codex: { serverId: "s9", name: "Office", fingerprint: "f", addresses: [] } }), { status: 409 }),
    );
    const rest = new RestClient({ baseUrl: "http://show:5555", token: () => null });
    const e = await startImport(rest, { token: () => null }, new Blob(["x"]), "a.pptx").catch((x) => x);
    expect(e).toBeInstanceOf(ApiRequestError);
    expect(e.code).toBe("codexElsewhere");
    expect(e.codex.serverId).toBe("s9");
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ error: { code: "badRequest", message: "a Keynote file", field: "fileName" } }), { status: 400 }));
    const e2 = await startImport(rest, { token: () => null }, new Blob(["x"]), "a.key").catch((x) => x);
    expect(e2.field).toBe("fileName");
    vi.stubGlobal("fetch", async () => {
      throw new TypeError("down");
    });
    const e3 = await startImport(rest, { token: () => null }, new Blob(["x"]), "a.pptx").catch((x) => x);
    expect(e3.code).toBe("network");
  });

  it("follows a job", async () => {
    const seen: string[] = [];
    const fetch = (async (url: string) => {
      seen.push(url);
      return new Response(JSON.stringify(job({ state: "done" })), { status: 200 });
    }) as unknown as typeof globalThis.fetch;
    const rest = new RestClient({ baseUrl: "http://codex:5555", token: () => null, fetch });
    const j = await importJob(rest, "j 1");
    expect(seen[0]).toBe("http://codex:5555/api/v1/codex/imports/pptx/j%201");
    expect(finished(j)).toBe(true);
    expect(finished(job())).toBe(false);
    expect(finished(null)).toBe(false);
  });
});

describe("progress and the report", () => {
  it("says where the job stands", () => {
    expect(progressText(job())).toBe("Reading the slides (3 of 10)");
    expect(progressText(job({ state: "pictures", done: 0, total: 2 }))).toBe("Drawing slide pictures for 2 slides…");
    expect(progressText(job({ state: "media", done: 4, total: 4 }))).toBe("Taking pictures into the Codex (4 of 4)");
    expect(progressText(job({ state: "queued", total: 0 }))).toMatch(/Waiting/);
    const order = ["queued", "reading", "pictures", "media", "saving", "done"] as const;
    const f = order.map((state) => progressFraction(job({ state, done: 1, total: 2 })));
    expect([...f].sort((a, b) => a - b)).toEqual(f);
    expect(f[0]).toBe(0);
    expect(f[5]).toBe(1);
  });

  it("sums up a report and each slide", () => {
    const report: PptxImportReport = {
      presentation: "p",
      name: "Spring talk",
      library: "l",
      width: 1920,
      height: 1080,
      editableSlides: 1,
      picturedSlides: 1,
      media: 3,
      slides: [
        slide({ number: 1, title: "Welcome" }),
        slide({ number: 2, pictured: [{ kind: "chart", name: "Giving", detail: "drawn from the slide's picture" }], picture: "libreOffice" }),
        slide({ number: 3, approximated: [{ kind: "table", detail: "left out (no slide picture)" }] }),
        slide({ number: 4, approximated: [{ kind: "a “cloud” shape", name: "Idea", detail: "imported as an editable rectangle with its text" }] }),
        slide({ number: 5, pictured: [{ kind: "chart" }, { kind: "table" }, { kind: "smartArt" }] }),
      ],
    };
    expect(summary(report)).toBe("5 slides: 1 fully editable, 1 with parts drawn from a picture of the slide, 2 with parts approximated or left out.");
    expect(slideStatus(report.slides[0]!)).toEqual({ tone: "ok", text: "Editable" });
    expect(slideStatus(report.slides[1]!)).toEqual({ tone: "picture", text: "Chart as picture" });
    expect(slideStatus(report.slides[2]!)).toEqual({ tone: "warn", text: "Table left out" });
    expect(slideStatus(report.slides[3]!)).toEqual({ tone: "warn", text: "“cloud” shape approximated" });
    expect(slideStatus(report.slides[4]!).text).toBe("3 objects as picture");
    expect(slideNotes(report.slides[1]!)).toEqual(["Chart “Giving”: drawn from the slide's picture"]);
    expect(slideNotes(report.slides[4]!)).toContain("SmartArt");
  });
});

describe("following a job", () => {
  it("gives up on a job the server no longer knows, and backs off on a dropped connection", async () => {
    const { pollFailure, MAX_POLL_FAILURES } = await import("../slidesImport");
    const gone = pollFailure(new ApiRequestError("notFound", "no import j1", 404), 1);
    expect("lost" in gone && gone.lost).toMatch(/no longer knows this import/);
    const net = new ApiRequestError("network", "Could not reach the server", 0);
    expect(pollFailure(net, 1)).toEqual({ delay: 400 });
    expect(pollFailure(net, 2)).toEqual({ delay: 800 });
    expect(pollFailure(net, 10)).toEqual({ delay: 5000 });
    const lost = pollFailure(net, MAX_POLL_FAILURES);
    expect("lost" in lost && lost.lost).toMatch(/Lost contact/);
    expect(pollFailure(new Error("boom"), 1)).toEqual({ delay: 400 });
  });
});
