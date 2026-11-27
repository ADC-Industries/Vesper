import { describe, expect, it } from "vitest";
import { Credentials, type KVStore } from "../storage";

function memStore(init: Record<string, string> = {}): KVStore & { data: Map<string, string> } {
  const data = new Map(Object.entries(init));
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

describe("Credentials storage", () => {
  it("stores under the oriel. prefix", () => {
    const kv = memStore();
    new Credentials(kv).setToken("s1", "tok");
    expect([...kv.data.keys()]).toEqual(["oriel.token.s1"]);
  });

  it("moves values saved under the legacy stage. prefix", () => {
    const kv = memStore({ "stage.token.s1": "old", "stage.pref.grid": "4" });
    const c = new Credentials(kv);
    expect(c.token("s1")).toBe("old");
    expect(kv.data.get("oriel.token.s1")).toBe("old");
    expect(kv.data.has("stage.token.s1")).toBe(false);
    c.setPref("grid", "6");
    expect(c.pref("grid")).toBe("6");
    expect(kv.data.has("stage.pref.grid")).toBe(false);
  });

  it("prefers the new key over a legacy one", () => {
    const kv = memStore({ "stage.token.s1": "old", "oriel.token.s1": "new" });
    expect(new Credentials(kv).token("s1")).toBe("new");
  });
});
