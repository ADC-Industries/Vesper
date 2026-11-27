import { afterEach, describe, expect, it, vi } from "vitest";
import type { Connection } from "../../server";
import { changes } from "../../state/events";
import { presCache } from "./presCache.svelte";

/** A connection whose presentation requests the test answers by hand, in any order. */
function slowConn() {
  const waiting: { id: string; answer: (name: string) => void; fail: () => void }[] = [];
  const conn = {
    rest: {
      presentation: (id: string) =>
        new Promise((resolve, reject) => waiting.push({ id, answer: (name) => resolve({ id, name, sections: [] }), fail: () => reject(new Error("offline")) })),
    },
  } as unknown as Connection;
  return { conn, waiting };
}

afterEach(() => presCache.use(null));

describe("the Show page's presentation cache", () => {
  it("an older refetch that answers last doesn't overwrite a newer one (A14-5)", async () => {
    const t = slowConn();
    presCache.use(t.conn);
    const first = presCache.load("p");
    t.waiting[0]!.answer("v0");
    await first;
    const r1 = presCache.refresh("p");
    const r2 = presCache.refresh("p");
    t.waiting[2]!.answer("v2");
    await r2;
    t.waiting[1]!.answer("v1");
    await r1;
    expect(presCache.docs.get("p")?.presentation.name).toBe("v2");
  });

  it("follows documentChanged for the whole connection, not only while the Show page is shown (A14-1)", async () => {
    const t = slowConn();
    const off = presCache.attach(t.conn);
    try {
      const l = presCache.load("p");
      t.waiting[0]!.answer("v0");
      await l;
      changes.emit({ type: "documentChanged", kind: "presentation", id: "p", msg: { type: "documentChanged", kind: "presentation", id: "p", change: "changed" } } as never);
      await vi.waitFor(() => expect(t.waiting).toHaveLength(2));
      t.waiting[1]!.answer("v1");
      await vi.waitFor(() => expect(presCache.docs.get("p")?.presentation.name).toBe("v1"));
    } finally {
      off();
    }
  });

  it("after a reconnect, every cached presentation and every failed one is fetched again (A14-1)", async () => {
    const t = slowConn();
    presCache.use(t.conn);
    const a = presCache.load("a");
    const b = presCache.load("b");
    t.waiting[0]!.answer("a0");
    t.waiting[1]!.fail();
    await Promise.all([a, b]);
    expect(presCache.failed.has("b")).toBe(true);
    const all = presCache.refreshAll();
    expect(t.waiting.slice(2).map((w) => w.id).sort()).toEqual(["a", "b"]);
    for (const w of t.waiting.slice(2)) w.answer(`${w.id}1`);
    await all;
    expect(presCache.docs.get("a")?.presentation.name).toBe("a1");
    expect(presCache.docs.get("b")?.presentation.name).toBe("b1");
    expect(presCache.failed.has("b")).toBe(false);
  });
});
