// The Show page's status bar: one entry per screen of the live state. Plain TypeScript, tested.

import type { AudioState, ScreenState } from "$protocol";

export interface ScreenStatus {
  id: string;
  /** "Main Projector · Audience", with " · off" or the output problem. */
  text: string;
  /** ok (on, outputs healthy), off (switched off), warn (an output unavailable), error. */
  level: "ok" | "off" | "warn" | "error";
  /** Tooltip: each output's state. */
  title: string;
}

const ROLE: Record<string, string> = { audience: "Audience", stage: "Stage" };

export function screenStatuses(screens: readonly ScreenState[], rolesEnabled: { audience: boolean; stage: boolean }): ScreenStatus[] {
  return screens.map((s) => {
    const role = ROLE[s.role] ?? s.role;
    const roleOn = s.role === "stage" ? rolesEnabled.stage : s.role === "audience" ? rolesEnabled.audience : true;
    const bad = s.outputs.filter((o) => o.status !== "ok");
    const err = bad.find((o) => o.status === "error");
    const outputs = s.outputs.map((o) => `${o.name}: ${o.status === "ok" ? "ok" : (o.detail ?? o.status)}`);
    let level: ScreenStatus["level"] = "ok";
    let extra = "";
    if (!s.enabled || !roleOn) {
      level = "off";
      extra = " · off";
    } else if (err) {
      level = "error";
      extra = ` · ${err.detail ?? "output error"}`;
    } else if (bad.length) {
      level = "warn";
      extra = ` · ${bad[0]!.detail ?? "output unavailable"}`;
    }
    return {
      id: s.id,
      text: `${s.name} · ${role}${extra}`,
      level,
      title: outputs.length ? outputs.join("\n") : "No outputs",
    };
  });
}

/** The sound card's problem for the status bar, or `null` while it plays (or none is set up). */
export interface AudioProblem {
  /** "Sound card lost, reconnecting…" or "Sound card unavailable, retrying…". */
  text: string;
  /** The server's words: the card and why. */
  title: string;
}

/**
 * A configured sound card that isn't playing (`outputState: "error"`): the server plays on its
 * null output meanwhile (the audio clock, network outputs and recordings keep going) and opens
 * the card again by itself, backing off to once a minute. A card that was playing and went away
 * is "lost"; one that never opened ("cannot open …") is "unavailable".
 */
export function audioProblem(audio: Pick<AudioState, "device" | "outputState" | "detail"> | null | undefined): AudioProblem | null {
  if (!audio || audio.outputState !== "error") return null;
  const detail = audio.detail ?? "";
  const lost = !detail.startsWith("cannot open");
  const device = audio.device || "the sound card";
  return {
    text: lost ? "Sound card lost, reconnecting…" : "Sound card unavailable, retrying…",
    title: `${device}: ${detail || "not playing"}. The show goes on without local sound; Vesper keeps trying to open it (Settings ▸ Audio).`,
  };
}
