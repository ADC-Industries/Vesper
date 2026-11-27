import { describe, expect, it } from "vitest";
import type { MessageToken } from "$protocol";
import { preview, syncTokens, tokenNames, tokenValues } from "./messages";

describe("message tokens", () => {
  it("finds ${name} and {name} tokens in order, once", () => {
    expect(tokenNames("Parent of ${child}, room {room}. ${child}!")).toEqual(["child", "room"]);
    expect(tokenNames("no tokens")).toEqual([]);
  });
  it("keeps known kinds and drops unused tokens", () => {
    const old: MessageToken[] = [
      { name: "t", kind: { type: "timer" } },
      { name: "gone", kind: { type: "text", defaultValue: "x" } },
    ];
    expect(syncTokens("${t} and ${new}", old)).toEqual([
      { name: "t", kind: { type: "timer" } },
      { name: "new", kind: { type: "text" } },
    ]);
  });
  it("sends only changed text values and previews", () => {
    const toks: MessageToken[] = [
      { name: "child", kind: { type: "text", defaultValue: "000" } },
      { name: "t", kind: { type: "timer" } },
    ];
    expect(tokenValues(toks, { child: "123", t: "zz" })).toEqual([{ name: "child", value: "123" }]);
    expect(tokenValues(toks, { child: "000" })).toEqual([]);
    expect(preview("Parent of ${child} in ${t}", toks, { child: "42" })).toBe("Parent of 42 in [t]");
    expect(preview("Parent of ${child}", toks, {})).toBe("Parent of 000");
  });

  it("braces with padding spaces aren't tokens: the server shows them as written (X3-9)", () => {
    expect(tokenNames("Parent of { child } please come")).toEqual([]);
    expect(tokenNames("${ child } and {child }")).toEqual([]);
    expect(syncTokens("Parent of { child }", [])).toEqual([]);
    const toks: MessageToken[] = [{ name: "child", kind: { type: "text", defaultValue: "" } }];
    expect(preview("{ child } / {child}", toks, { child: "Sam" })).toBe("{ child } / Sam");
  });

  it("keeps declared tokens whose names have spaces or other characters (D08-1)", () => {
    const toks: MessageToken[] = [
      { name: "Child Number", kind: { type: "text", defaultValue: "0" } },
      { name: "Room #", kind: { type: "timer" } },
    ];
    // Editing the message (here: nothing in the text changes) keeps both declared tokens.
    expect(syncTokens("Room ${Room #}: parent of ${Child Number}", toks)).toEqual([toks[1], toks[0]]);
    // A name with inner spaces typed in new braces is a token too, as the server would fill it in.
    expect(tokenNames("Parent of {child name}")).toEqual(["child name"]);
    expect(preview("Parent of ${Child Number} in {Room #}", toks, { "Child Number": "42" })).toBe(
      "Parent of 42 in [Room #]",
    );
    // A declared name no longer in the text is dropped.
    expect(syncTokens("Parent of ${Child}", toks)).toEqual([{ name: "Child", kind: { type: "text" } }]);
  });

  it("declares only token names the server accepts (X2-1)", () => {
    // The server's rule: not empty, no leading or trailing whitespace, no `{`, `}` or `$`
    // (inner spaces are fine, as imported names have them).
    const serverAccepts = (n: string) => n !== "" && n === n.trim() && !/[{}$]/.test(n);
    const text = "Parent of {Child Number}, ${room 2}, {a.b-c}, { padded }, {x }, {$y}, {}, {at the back}";
    const names = tokenNames(text);
    expect(names).toEqual(["Child Number", "room 2", "a.b-c", "at the back"]);
    for (const n of names) expect(serverAccepts(n)).toBe(true);
  });
});
