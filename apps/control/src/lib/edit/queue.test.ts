import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PresentationEditResult, PresentationOp } from "$protocol";
import { DEBOUNCE_MS, EditQueue, rebaseText, type QueueTransport } from "./queue";

class Offline extends Error {}
class Locked423 extends Error {}
class Conflict409 extends Error {
  constructor(public text: string) {
    super("conflict");
  }
}

function transport() {
  const sent: PresentationOp[][] = [];
  const applied: string[] = [];
  const failed: string[] = [];
  let next: (() => Promise<PresentationEditResult>) | null = null;
  const t: QueueTransport = {
    send: (ops) => {
      sent.push(ops);
      const f = next;
      next = null;
      return f ? f() : Promise.resolve({ revision: `r${sent.length}`, applied: ops, inverse: [] });
    },
    applied: (label) => applied.push(label),
    isOffline: (e) => e instanceof Offline,
    conflictText: (e) => (e instanceof Conflict409 ? e.text : null),
    failed: (label, e) => failed.push(e instanceof Error && e.message.includes("took over") ? `${label}: ${e.message}` : label),
    isLocked: (e) => e instanceof Locked423,
  };
  return { t, sent, applied, failed, fail: (f: () => Promise<PresentationEditResult>) => (next = f) };
}

const flush = () => vi.advanceTimersByTimeAsync(0);

describe("EditQueue", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("debounces typing into one setSlideText with expectText", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("s1", "old");
    q.setText("s1", "o");
    await vi.advanceTimersByTimeAsync(200);
    q.setText("s1", "new text");
    expect(q.status).toBe("saving");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS - 10);
    expect(x.sent).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(20);
    expect(x.sent).toEqual([[{ type: "setSlideText", slide: "s1", text: "new text", expectText: "old" }]]);
    await flush();
    expect(q.status).toBe("saved");
    expect(q.baseOf("s1")).toBe("new text");
    // The next edit expects what was saved.
    q.setText("s1", "newer");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent[1]![0]).toMatchObject({ text: "newer", expectText: "new text" });
    expect(x.applied).toEqual(["Typing", "Typing"]);
  });

  it("sends structural edits in order after earlier typing", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("s1", "a");
    q.setText("s1", "a\nb");
    const done = q.enqueueOps("Insert Slide Break", [
      { type: "setSlideText", slide: "s1", text: "a\nb\n" },
      { type: "splitSlide", slide: "s1", atParagraph: 2 },
    ]);
    await flush();
    await flush();
    await done;
    expect(x.sent.map((o) => o[0]!.type)).toEqual(["setSlideText", "setSlideText"]);
    // The batch expects the text the typing before it saved.
    expect(x.sent[1]![0]).toMatchObject({ expectText: "a\nb" });
    expect(x.sent[1]![1]!.type).toBe("splitSlide");
    // Holds until the caller refetched, then continues.
    q.setText("s2", "x");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 10);
    expect(x.sent).toHaveLength(2);
    q.release();
    await flush();
    expect(x.sent).toHaveLength(3);
  });

  it("keeps edits while offline and retries, never dropping them", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("s1", "a");
    x.fail(() => Promise.reject(new Offline()));
    q.setText("s1", "b");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("offline");
    // More typing while offline coalesces into the queued edit.
    q.setText("s1", "bc");
    expect(q.pendingText("s1")).toBe("bc");
    x.fail(() => Promise.reject(new Offline()));
    await vi.advanceTimersByTimeAsync(1000);
    expect(q.status).toBe("offline");
    q.kick();
    await flush();
    expect(q.status).toBe("saved");
    expect(x.sent.at(-1)![0]).toMatchObject({ text: "bc", expectText: "a" });
  });

  it("pauses a slide on a conflict until the operator chooses", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    const conflicts: unknown[] = [];
    q.onConflict((c) => conflicts.push(c));
    q.setBase("s1", "a");
    x.fail(() => Promise.reject(new Conflict409("theirs")));
    q.setText("s1", "mine");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("conflict");
    expect(q.conflict).toEqual({ key: "s1", mine: "mine", theirs: "theirs" });
    expect(q.hasPending("s1")).toBe(true);
    // Other slides keep saving.
    q.setBase("s2", "x");
    q.setText("s2", "y");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent.at(-1)![0]).toMatchObject({ slide: "s2" });
    // Keep mine: resent against their text.
    q.resolve("s1", "mine");
    await flush();
    expect(x.sent.at(-1)![0]).toMatchObject({ slide: "s1", text: "mine", expectText: "theirs" });
    expect(q.status).toBe("saved");
    expect(conflicts.at(-1)).toBeNull();
  });

  it("use theirs drops the local edit", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.onConflict(() => {});
    q.setBase("s1", "a");
    x.fail(() => Promise.reject(new Conflict409("theirs")));
    q.setText("s1", "mine");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    const n = x.sent.length;
    q.resolve("s1", "theirs");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent).toHaveLength(n);
    expect(q.hasPending("s1")).toBe(false);
    expect(q.baseOf("s1")).toBe("theirs");
  });

  it("flush sends waiting typing at once (before undo)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setText("s1", "abc");
    const f = q.flush();
    await flush();
    await f;
    expect(x.sent).toHaveLength(1);
    expect(q.idle()).toBe(true);
  });

  it("a conflict after the editor left is reported with the typing, not paused silently (A14-6)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.onConflict(() => {});
    q.setText("s1", "my words");
    x.fail(() => Promise.reject(new Conflict409("theirs")));
    q.retire();
    let done = false;
    void q.flush().then(() => (done = true));
    await flush();
    expect(x.failed).toEqual(["Typing"]);
    expect(done).toBe(true);
    expect(q.hasPending("s1")).toBe(false);
    // A conflict shown, then the editor closes before it was answered.
    const y = transport();
    const r = new EditQueue(y.t);
    r.onConflict(() => {});
    r.setText("s2", "unanswered");
    y.fail(() => Promise.reject(new Conflict409("theirs")));
    await r.flush().catch(() => {});
    await flush();
    expect(r.status).toBe("conflict");
    r.retire();
    expect(y.failed).toEqual(["Typing"]);
    expect(r.hasPending("s2")).toBe(false);
  });

  it("a retired queue (another presentation opened) still sends waiting typing", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    const statuses: string[] = [];
    q.subscribe((s) => statuses.push(s));
    q.setText("s1", "typed just before switching");
    q.setText("s2", "and here, while offline");
    x.fail(() => Promise.reject(new Offline()));
    statuses.length = 0;
    q.retire();
    await flush();
    // The first try failed offline: it is retried, not dropped.
    expect(x.sent).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(x.sent.map((ops) => ops[0])).toMatchObject([
      { slide: "s1", text: "typed just before switching" },
      { slide: "s1", text: "typed just before switching" },
      { slide: "s2", text: "and here, while offline" },
    ]);
    expect(q.idle()).toBe(true);
    // The editor that left is no longer told.
    expect(statuses).toEqual([]);
  });

  it("reports other failures and carries on", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    x.fail(() => Promise.reject(new Error("bad request")));
    const p = q.enqueueOps("Merge", [{ type: "mergeSlides", first: "a", second: "b" }]);
    await expect(p).rejects.toThrow("bad request");
    expect(x.failed).toEqual(["Merge"]);
    expect(q.status).toBe("saved");
  });
});

describe("typing during a split or merge (A13-3)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("typing in the old text area right after a split lands on the slide's new text", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("A", "L1\nL2");
    const split = q.enqueueOps("Split", [{ type: "splitSlide", slide: "A", atParagraph: 1 }]);
    await flush();
    await split;
    // The caret is still after L1 in A's text area (the editor hasn't caught up): "x" typed.
    q.setText("A", "L1x\nL2");
    // The refetch: A is "L1" now, the new slide A2 "L2".
    q.setBase("A", "L1");
    q.setBase("A2", "L2");
    expect(q.pendingText("A")).toBe("L1x");
    q.release();
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
    expect(x.sent[1]).toEqual([{ type: "setSlideText", slide: "A", text: "L1x", expectText: "L1" }]);
    expect(x.failed).toEqual([]);
  });

  it("typing that belongs to the part split off can't be placed: reported, not sent as a duplicate", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("A", "L1\nL2");
    const split = q.enqueueOps("Split", [{ type: "splitSlide", slide: "A", atParagraph: 1 }]);
    await flush();
    await split;
    q.setText("A", "L1\nL2x");
    q.setBase("A", "L1");
    q.release();
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
    expect(x.sent).toHaveLength(1);
    expect(x.failed).toEqual(["Typing"]);
  });

  it("typing in a slide merged away is reported, not sent to a slide that's gone", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("A", "L1");
    q.setBase("B", "L2");
    const merge = q.enqueueOps("Merge", [{ type: "mergeSlides", first: "A", second: "B" }]);
    await flush();
    await merge;
    q.setText("B", "L2y");
    q.setText("A", "L1z");
    // The refetch: A holds both, B is gone.
    q.setBase("A", "L1\nL2");
    q.release();
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
    expect(x.sent[1]).toEqual([{ type: "setSlideText", slide: "A", text: "L1z\nL2", expectText: "L1\nL2" }]);
    expect(x.sent).toHaveLength(2);
    expect(x.failed).toEqual(["Typing"]);
  });
});

describe("queue fixes (batch 10)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("a split pressed while the slide's typing is being saved doesn't duplicate it (D10-3)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("A", "abc\nabd");
    let answer: (r: PresentationEditResult) => void = () => {};
    x.fail(() => new Promise((r) => (answer = r)));
    q.setText("A", "abcX\nabd");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent).toHaveLength(1);
    // While "abcX" is in flight: split after the X, then type Z.
    const split = q.enqueueOps("Split", [{ type: "splitSlide", slide: "A", atParagraph: 1 }]);
    q.setText("A", "abcXZ\nabd");
    answer({ revision: "r1", applied: [], inverse: [] });
    await flush();
    await split;
    q.setBase("A", "abcX");
    q.setBase("A2", "abd");
    q.release();
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent[2]).toEqual([{ type: "setSlideText", slide: "A", text: "abcXZ", expectText: "abcX" }]);
    expect(x.failed).toEqual([]);
  });

  it("copying kept typing away shows the lock holder's current text, not the old one (D10-5)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.subscribe(() => {});
    q.setBase("s1", "old");
    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setText("s1", "mine");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("locked");
    // The lock holder rewrites the slide meanwhile.
    q.setBase("s1", "theirs");
    expect(q.baseOf("s1")).toBe("old");
    q.discardLocked();
    expect(q.baseOf("s1")).toBe("theirs");
    // The next typing expects their text.
    q.setText("s1", "theirs, and more");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent[1]![0]).toMatchObject({ text: "theirs, and more", expectText: "theirs" });
  });

  it("a lock refusal with nobody listening still answers the structural edit behind it (D10-6)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setBase("A", "L1\nL2");
    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setText("A", "L1 typed\nL2");
    const split = q.enqueueOps("Split", [{ type: "splitSlide", slide: "A", atParagraph: 1 }]);
    let settled = false;
    void split.then(
      () => (settled = true),
      () => (settled = true),
    );
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.failed).toEqual(["Typing: someone else took over editing. What you typed: “L1 typed\nL2”"]);
    expect(settled).toBe(true);
    expect(x.sent).toHaveLength(2);
    await q.flush();
  });
});

describe("rebaseText", () => {
  it("moves an edit onto the kept start or end of a text, or says it can't", () => {
    expect(rebaseText("L1\nL2", "L1\nL2", "L1")).toBe("L1");
    expect(rebaseText("L1\nL2", "Lx1\nL2", "L1")).toBe("Lx1");
    expect(rebaseText("L1\nL2", "L\nL2", "L1")).toBe("L");
    expect(rebaseText("L1\nL2", "L1\nxL2", "L1")).toBeNull();
    expect(rebaseText("L1", "L1z", "L1\nL2")).toBe("L1z\nL2");
    expect(rebaseText("zz", "zzq", "aa\nzz")).toBe("aa\nzzq");
  });
});

describe("EditQueue with edit locks (G8)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("keeps typing refused by a lock and sends it once the lock is back", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    const seen: string[] = [];
    q.subscribe((s) => seen.push(s));
    q.setBase("s1", "old");
    q.setBase("s2", "two");
    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setText("s1", "mine");
    q.setText("s2", "two more");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("locked");
    expect(seen).toContain("locked");
    expect(x.failed).toEqual([]);
    expect(q.lockedTexts()).toEqual([
      { key: "s1", text: "mine" },
      { key: "s2", text: "two more" },
    ]);
    // Nothing more is sent while locked, even on a reconnect; flush (undo) doesn't wait.
    q.kick();
    await q.flush();
    expect(x.sent).toHaveLength(1);
    q.setText("s1", "mine, still");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent).toHaveLength(1);
    expect(q.pendingText("s1")).toBe("mine, still");
    // Taken back: everything kept is sent, in order.
    q.unlock();
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent.slice(1).map((o) => (o[0] as { text: string }).text)).toEqual(["mine, still", "two more"]);
    expect(q.status).toBe("saved");
  });

  it("lets kept typing go only when asked, and reports it with its text when the editor closes", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.subscribe(() => {});
    q.setBase("s1", "old");
    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setText("s1", "copied away");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    q.discardLocked();
    expect(q.status).toBe("saved");
    expect(q.hasPending("s1")).toBe(false);
    expect(x.failed).toEqual([]);

    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setText("s1", "never dropped");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("locked");
    q.retire();
    expect(x.failed).toEqual(["Typing: someone else took over editing. What you typed: “never dropped”"]);
  });
});

describe("second-language typing in the queue (R3-3, R3-4, R3-5)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const op = (slide: string) => (text: string): PresentationOp => ({
    type: "setSlideTranslation",
    slide,
    translation: { language: "es", texts: [{ element: "e", paragraphs: [{ runs: [{ text }] }] }] },
  });
  const sentText = (ops: PresentationOp[]) => {
    const o = ops[0]!;
    return o.type === "setSlideTranslation" ? o.translation?.texts?.[0]?.paragraphs?.[0]?.runs?.[0]?.text : null;
  };

  it("is debounced, coalesced, and one undo step per slide", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setSecond("s1", "Hol", op("s1"));
    q.setSecond("s1", "Hola", op("s1"));
    expect(q.hasPendingSecond("s1")).toBe(true);
    expect(q.pendingSecond("s1")).toBe("Hola");
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(x.sent.map(sentText)).toEqual(["Hola"]);
    expect(x.applied).toEqual(["Second Language"]);
    expect(q.hasPendingSecond("s1")).toBe(false);
    expect(q.status).toBe("saved");
  });

  it("a failed save is kept and retried while offline, not dropped (R3-3)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    x.fail(() => Promise.reject(new Offline()));
    q.setSecond("s1", "Hola", op("s1"));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("offline");
    expect(q.pendingSecond("s1")).toBe("Hola");
    q.kick();
    await flush();
    expect(x.sent.map(sentText)).toEqual(["Hola", "Hola"]);
    expect(q.status).toBe("saved");
    expect(x.failed).toEqual([]);
  });

  it("is kept while someone else holds the lock, and offered for copying", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.subscribe(() => {});
    x.fail(() => Promise.reject(new Locked423("locked")));
    q.setSecond("s1", "Hola", op("s1"));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    expect(q.status).toBe("locked");
    expect(q.lockedTexts()).toEqual([{ key: "s1", text: "Hola", second: true }]);
    q.unlock();
    await flush();
    expect(x.sent.map(sentText)).toEqual(["Hola", "Hola"]);
    expect(q.status).toBe("saved");
  });

  it("undo waits for it: flush sends a waiting save and resolves after it (R3-4)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setSecond("s1", "Hola", op("s1"));
    let done = false;
    const f = q.flush().then(() => (done = true));
    expect(x.sent).toHaveLength(1);
    expect(done).toBe(false);
    await f;
    expect(x.applied).toEqual(["Second Language"]);
  });

  it("a retired queue (another presentation opened) still sends it (R3-5)", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setSecond("s1", "Hola", op("s1"));
    q.retire();
    await flush();
    expect(x.sent.map(sentText)).toEqual(["Hola"]);
  });

  it("a save that can't be made is reported with the text", async () => {
    const x = transport();
    const q = new EditQueue(x.t);
    q.setSecond("gone", "Hola", () => null);
    x.fail(() => Promise.reject(new Error("bad request")));
    q.setSecond("s2", "Adiós", op("s2"));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 1);
    await flush();
    expect(x.failed).toEqual(["Second-language typing", "Second-language typing"]);
    expect(q.status).toBe("saved");
  });
});
