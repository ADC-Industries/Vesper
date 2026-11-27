import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PresentationOp, TranslateJob } from "$protocol";
import type { Connection } from "../server";
import { TranslatePreview } from "./translatePreview.svelte";

const job = (over: Partial<TranslateJob> = {}): TranslateJob => ({
  id: "j1",
  presentation: "p",
  from: "en",
  to: "es",
  model: "opus-mt-en-es",
  status: "queued",
  done: 0,
  total: 1,
  slides: [],
  notice: "",
  ...over,
});

const done = job({
  status: "done",
  done: 1,
  slides: [{ slide: "s1", translation: { language: "es", texts: [{ element: "e1", paragraphs: [{ runs: [{ text: "Luz de la mañana" }] }] }], machine: { engine: "opus-mt (candle)", model: "opus-mt-en-es" } } }],
});

function fakeConn(answers: TranslateJob[]) {
  const rest = {
    translatePreview: vi.fn(async () => job()),
    translateJob: vi.fn(async () => answers.shift() ?? done),
    cancelTranslateJob: vi.fn(async () => {}),
  };
  const conn = { rest, pushNotice: vi.fn() } as unknown as Connection;
  return { conn, rest };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("a machine-translation preview (shared by Reflow and the slide editor)", () => {
  it("polls the job until done, then offers its proposals to keep", async () => {
    const { conn, rest } = fakeConn([job({ status: "translating" }), done]);
    const onDone = vi.fn();
    const mt = new TranslatePreview(conn, onDone);
    await mt.start({ presentation: "p", slides: ["s1"], from: "en", to: "es" });
    expect(rest.translatePreview).toHaveBeenCalledWith({ presentation: "p", slides: ["s1"], from: "en", to: "es" });
    expect(mt.running).toBe(true);
    await vi.advanceTimersByTimeAsync(300);
    expect(mt.job?.status).toBe("translating");
    expect(mt.proposing).toBe(false);
    await vi.advanceTimersByTimeAsync(400);
    expect(mt.proposing).toBe(true);
    expect(mt.count).toBe(1);
    expect(onDone).toHaveBeenCalledOnce();
    const run = vi.fn(async () => true);
    expect(await mt.keep(run)).toBe(true);
    expect(run).toHaveBeenCalledWith([{ type: "setSlideTranslation", slide: "s1", translation: done.slides[0]!.translation }], "es");
    expect(mt.job).toBeNull();
    expect(mt.proposing).toBe(false);
  });

  it("keeps the proposals when the edit is refused, and skips slides that are gone", async () => {
    const two = job({ status: "done", done: 2, total: 2, slides: [...done.slides, { ...done.slides[0]!, slide: "s2" }] });
    const { conn } = fakeConn([two]);
    const mt = new TranslatePreview(conn);
    await mt.start({ presentation: "p", from: "en", to: "es" });
    await vi.advanceTimersByTimeAsync(300);
    expect(mt.count).toBe(2);
    // Refused (or failed): nothing is lost, Keep can be pressed again.
    expect(await mt.keep(async () => false)).toBe(false);
    expect(mt.count).toBe(2);
    expect(await mt.keep(async () => Promise.reject(new Error("offline")))).toBe(false);
    expect(mt.count).toBe(2);
    expect(mt.keeping).toBe(false);
    // s2 was deleted meanwhile: only s1 is sent.
    const run = vi.fn(async (_ops: PresentationOp[], _language: string) => true);
    expect(await mt.keep(run, (id) => id === "s1")).toBe(true);
    expect(run.mock.calls[0]![0]).toEqual([{ type: "setSlideTranslation", slide: "s1", translation: done.slides[0]!.translation }]);
    expect(mt.proposing).toBe(false);
  });

  it("cancels a running job on the server when its owner goes away", async () => {
    const { conn, rest } = fakeConn([job({ status: "translating" })]);
    const mt = new TranslatePreview(conn);
    await mt.start({ presentation: "p", from: "en", to: "es" });
    await vi.advanceTimersByTimeAsync(300);
    mt.dispose();
    expect(rest.cancelTranslateJob).toHaveBeenCalledWith("j1");
    await vi.advanceTimersByTimeAsync(2000);
    expect(rest.translateJob).toHaveBeenCalledTimes(1);
    // A start after that does nothing.
    await mt.start({ presentation: "p", from: "en", to: "es" });
    expect(rest.translatePreview).toHaveBeenCalledTimes(1);
  });

  it("cancels a job whose start answered after it was replaced or disposed", async () => {
    const { conn, rest } = fakeConn([]);
    let answer: (j: TranslateJob) => void = () => {};
    rest.translatePreview.mockImplementationOnce(() => new Promise<TranslateJob>((r) => (answer = r)));
    const mt = new TranslatePreview(conn);
    const first = mt.start({ presentation: "p", from: "en", to: "es" });
    // A second start overlaps the first one's POST.
    rest.translatePreview.mockImplementationOnce(async () => job({ id: "j2" }));
    await mt.start({ presentation: "p", from: "en", to: "fr" });
    answer(job({ id: "j1" }));
    await first;
    expect(rest.cancelTranslateJob).toHaveBeenCalledWith("j1");
    expect(rest.cancelTranslateJob).not.toHaveBeenCalledWith("j2");
    expect(mt.job?.id).toBe("j2");
    // Disposed while starting: the job it gets is cancelled.
    rest.translatePreview.mockImplementationOnce(() => new Promise<TranslateJob>((r) => (answer = r)));
    const third = mt.start({ presentation: "p", from: "en", to: "es" });
    mt.dispose();
    answer(job({ id: "j3" }));
    await third;
    expect(rest.cancelTranslateJob).toHaveBeenCalledWith("j3");
  });

  it("cancels a running job on the server; discarding writes nothing", async () => {
    const { conn, rest } = fakeConn([job({ status: "downloading" })]);
    const mt = new TranslatePreview(conn);
    await mt.start({ presentation: "p", from: "en", to: "es" });
    mt.cancel();
    expect(rest.cancelTranslateJob).toHaveBeenCalledWith("j1");
    expect(mt.job).toBeNull();
    // Nothing polls after the cancel.
    await vi.advanceTimersByTimeAsync(2000);
    expect(rest.translateJob).not.toHaveBeenCalled();

    await mt.start({ presentation: "p", from: "en", to: "es" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(mt.proposing).toBe(true);
    rest.cancelTranslateJob.mockClear();
    mt.discard();
    expect(mt.proposing).toBe(false);
    expect(rest.cancelTranslateJob).not.toHaveBeenCalled();
  });

  it("says so when the preview could not start", async () => {
    const { conn, rest } = fakeConn([]);
    rest.translatePreview.mockRejectedValueOnce(new Error("no model"));
    const mt = new TranslatePreview(conn);
    await mt.start({ presentation: "p", from: "en", to: "es" });
    expect(mt.job).toBeNull();
    expect(conn.pushNotice).toHaveBeenCalledWith("error", "Could not translate: no model");
  });
});
