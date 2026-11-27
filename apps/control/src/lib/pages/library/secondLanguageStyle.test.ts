import { describe, expect, it } from "vitest";
import { SerialSaves } from "./secondLanguageStyle";

describe("the theme's second-language saves (H11-8)", () => {
  it("run one after another, each with the revision the one before got", async () => {
    const sent: [string, string | null][] = [];
    let n = 1;
    const s = new SerialSaves<string>(
      async (v, rev) => {
        sent.push([v, rev]);
        await Promise.resolve();
        return `r${++n}`;
      },
      () => {},
    );
    s.reset("r1");
    void s.save("italic");
    await s.save("colour");
    expect(sent).toEqual([
      ["italic", "r1"],
      ["colour", "r2"],
    ]);
    expect(s.pending).toBe(0);
  });

  it("drop what is queued behind a failure, and take a reload's revision", async () => {
    const sent: string[] = [];
    const failed: unknown[] = [];
    const s = new SerialSaves<string>(
      async (v) => {
        sent.push(v);
        if (v === "bad") throw new Error("409");
        return "r9";
      },
      (e) => failed.push(e),
    );
    s.reset("r1");
    void s.save("bad");
    await s.save("after");
    expect(sent).toEqual(["bad"]);
    expect(failed).toHaveLength(1);
    // A rename reloaded the theme: its revision is used next.
    const seen: (string | null)[] = [];
    const t = new SerialSaves<string>(async (_v, rev) => (seen.push(rev), "r6"), () => {});
    t.reset("r5");
    await t.save("x");
    expect(seen).toEqual(["r5"]);
  });
});
