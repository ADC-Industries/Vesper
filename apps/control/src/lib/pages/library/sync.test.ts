import { describe, expect, it } from "vitest";
import { boxState, syncedText } from "./sync";

describe("Codex box status in words", () => {
  it("says how many Vespers are synced", () => {
    expect(syncedText({ showing: 3, synced: 3 })).toEqual({ text: "Synced to 3 of 3 Vespers", ok: true });
    expect(syncedText({ showing: 3, synced: 2 })).toEqual({ text: "Synced to 2 of 3 Vespers", ok: false });
    expect(syncedText({ showing: 1, synced: 1 })?.text).toBe("Synced to 1 of 1 Vesper");
    expect(syncedText({ showing: 0, synced: 0 })).toBeNull();
    expect(syncedText(undefined)).toBeNull();
  });

  it("says where each show box is", () => {
    expect(boxState({ online: true, behind: 0 })).toEqual({ status: "ok", text: "Up to date" });
    expect(boxState({ online: true, behind: 1 })).toEqual({ status: "warn", text: "1 change behind" });
    expect(boxState({ online: true, behind: 12 }).text).toBe("12 changes behind");
    expect(boxState({ online: true, behind: null }).text).toBe("Copying");
    expect(boxState({ online: false, behind: 0 })).toEqual({ status: "off", text: "Not heard from" });
  });
});
