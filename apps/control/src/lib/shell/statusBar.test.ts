import { describe, expect, it } from "vitest";
import type { AudioState, ScreenState } from "$protocol";
import { audioProblem, screenStatuses } from "./statusBar";

const out = (status: "ok" | "unavailable" | "error", detail: string | null = null) =>
  ({ name: "IDN: Out", kind: "idn", status, detail }) as unknown as ScreenState["outputs"][number];
const screen = (name: string, role: string, enabled = true, outputs = [out("ok")]) =>
  ({ id: name, name, role, enabled, outputs }) as unknown as ScreenState;

describe("status bar", () => {
  it("shows each screen with its role, off, or its output problem", () => {
    const st = screenStatuses(
      [
        screen("Front", "audience"),
        screen("Monitor", "stage", false),
        screen("Lobby", "audience", true, [out("unavailable", "no receiver")]),
        screen("Side", "audience", true, [out("ok"), out("error", "device lost")]),
      ],
      { audience: true, stage: true },
    );
    expect(st.map((s) => [s.text, s.level])).toEqual([
      ["Front · Audience", "ok"],
      ["Monitor · Stage · off", "off"],
      ["Lobby · Audience · no receiver", "warn"],
      ["Side · Audience · device lost", "error"],
    ]);
    expect(st[3]!.title).toContain("device lost");
  });

  it("counts a screen off while its role is switched off (Ctrl+1 / Ctrl+2)", () => {
    const st = screenStatuses([screen("Front", "audience"), screen("Monitor", "stage")], { audience: false, stage: true });
    expect(st.map((s) => s.level)).toEqual(["off", "ok"]);
  });
});

describe("sound card problem", () => {
  const audio = (outputState: "ok" | "null" | "error", detail: string | null = null) =>
    ({ device: "hw:CARD=USB,DEV=0", outputState, detail }) as unknown as AudioState;

  it("is quiet while the card plays or none is set up", () => {
    expect(audioProblem(audio("ok"))).toBeNull();
    expect(audioProblem(audio("null"))).toBeNull();
    expect(audioProblem(null)).toBeNull();
  });

  it("says a lost card is being reconnected, and a card that never opened is retried", () => {
    const lost = audioProblem(audio("error", "hw:CARD=USB,DEV=0: the device was unplugged"))!;
    expect(lost.text).toBe("Sound card lost, reconnecting…");
    expect(lost.title).toContain("unplugged");
    const never = audioProblem(audio("error", "cannot open hw:CARD=USB,DEV=0: (no such device)"))!;
    expect(never.text).toBe("Sound card unavailable, retrying…");
  });
});
