// Visibility conditions and live-data link settings in the slide editor's Shape tab (plan G16,
// docs/TRANSITIONS-PLAN.md section 5): the kinds "Add condition" offers, a new condition of each
// kind, list edits (whole-list `setElement {visibility}` patches) and labels.

import type { DataLink, Element, ElementPatch, Layer, MediaCondition, PlaylistItemWhich, TimerCondition, TimerShow, VisibilityCondition } from "$protocol";

export type ConditionKind = Exclude<VisibilityCondition["type"], "unsupported" | "unknown">;

/** What "Add condition" offers, in menu order. `videoInput` needs a video input to name (the
 *  show's logical inputs, `GET /video-inputs`). */
export const CONDITION_KINDS: readonly { kind: ConditionKind; label: string }[] = [
  { kind: "elementHasText", label: "Object has text" },
  { kind: "timer", label: "Timer" },
  { kind: "message", label: "Message showing" },
  { kind: "stageMessage", label: "Stage message showing" },
  { kind: "layer", label: "Layer live" },
  { kind: "videoInput", label: "Video input live" },
  { kind: "playlistItem", label: "Playlist item" },
  { kind: "videoCountdown", label: "Video state" },
  { kind: "audioCountdown", label: "Audio state" },
];

export const TIMER_STATES: readonly { v: TimerCondition; label: string }[] = [
  { v: "hasTimeRemaining", label: "has time remaining" },
  { v: "hasExpired", label: "has expired" },
  { v: "isRunning", label: "is running" },
  { v: "notRunning", label: "is not running" },
];

export const MEDIA_STATES: readonly { v: MediaCondition; label: string }[] = [
  { v: "hasTimeRemaining", label: "has time remaining" },
  { v: "hasExpired", label: "has ended (or none)" },
  { v: "isRunning", label: "is playing" },
  { v: "notRunning", label: "is not playing" },
  { v: "looping", label: "loops" },
  { v: "notLooping", label: "does not loop" },
];

export const CONDITION_LAYERS: readonly { v: Layer; label: string }[] = [
  { v: "slide", label: "Slide" },
  { v: "media", label: "Media" },
  { v: "videoInput", label: "Video Input" },
  { v: "announcements", label: "Announcements" },
  { v: "props", label: "Props" },
  { v: "messages", label: "Messages" },
  { v: "audio", label: "Audio" },
];

export const PLAYLIST_ITEMS: readonly { v: PlaylistItemWhich; label: string }[] = [
  { v: "current", label: "Current item" },
  { v: "next", label: "Next item" },
  { v: "currentHeader", label: "Current header" },
  { v: "nextHeader", label: "Next header" },
  { v: "playlist", label: "A playlist" },
];

export const TIMER_SHOWS: readonly { v: TimerShow; label: string }[] = [
  { v: "value", label: "Value" },
  { v: "elapsed", label: "Elapsed" },
  { v: "remaining", label: "Remaining" },
];

/** A video input a condition can name (a `VideoInputEntry` or its link). */
export type InputChoice = { id: string; name?: string };

/** The link a `videoInput` condition stores for `input` (its name kept for display). */
export function inputLink(input: InputChoice): { id: string; name?: string } {
  return { id: input.id, ...(input.name ? { name: input.name } : {}) };
}

/** A new condition of `kind`. `others` = the slide's other objects with text (the first one is
 * the default for "Object has text"), `inputs` = the show's video inputs (the first one is the
 * default for "Video input live"); `null` when the kind can't be made (nothing to name). */
export function newCondition(kind: ConditionKind, others: readonly Pick<Element, "id">[], inputs: readonly InputChoice[] = []): VisibilityCondition | null {
  switch (kind) {
    case "elementHasText":
      return others[0] ? { type: "elementHasText", element: others[0].id } : null;
    case "timer":
      return { type: "timer" };
    case "message":
      return { type: "message" };
    case "stageMessage":
      return { type: "stageMessage" };
    case "layer":
      return { type: "layer", layer: "slide" };
    case "playlistItem":
      return { type: "playlistItem", which: "next" };
    case "videoCountdown":
      return { type: "videoCountdown", state: "isRunning" };
    case "audioCountdown":
      return { type: "audioCountdown", state: "isRunning" };
    case "videoInput":
      return inputs[0] ? { type: "videoInput", input: inputLink(inputs[0]) } : null;
  }
}

/** The element's conditions with `c` added. */
export function withCondition(list: readonly VisibilityCondition[] | undefined, c: VisibilityCondition): VisibilityCondition[] {
  return [...(list ?? []), c];
}

/** The element's conditions with the one at `i` replaced. */
export function replaceCondition(list: readonly VisibilityCondition[] | undefined, i: number, c: VisibilityCondition): VisibilityCondition[] {
  return (list ?? []).map((x, j) => (j === i ? c : x));
}

/** The element's conditions without the one at `i`. */
export function removeCondition(list: readonly VisibilityCondition[] | undefined, i: number): VisibilityCondition[] {
  return (list ?? []).filter((_, j) => j !== i);
}

/** The `setElement` patch for a new list. */
export function visibilityPatch(list: VisibilityCondition[]): ElementPatch {
  return { visibility: list };
}

/** `active` as a select value ("yes" / "no"); the model omits `true`. */
export function activeOf(c: { active?: boolean }): "yes" | "no" {
  return c.active === false ? "no" : "yes";
}

/** `active` from a select value, omitted when true (as the server writes it). */
export function withActive<T extends { active?: boolean }>(c: T, v: string): T {
  const { active: _, ...rest } = c;
  return (v === "no" ? { ...rest, active: false } : rest) as T;
}

/** A short description of a condition, for the Objects list and the row title. */
export function conditionLabel(c: VisibilityCondition, names: { element?: (id: string) => string; message?: (id: string) => string; input?: (id: string) => string | undefined } = {}): string {
  const not = (c as { active?: boolean }).active === false;
  switch (c.type) {
    case "elementHasText": {
      const n = names.element?.(c.element) ?? "Object";
      return c.hasText === false ? `${n} is empty` : `${n} has text`;
    }
    case "timer":
      return `${c.timer?.name ?? "Most recent timer"} ${TIMER_STATES.find((s) => s.v === (c.state ?? "hasTimeRemaining"))?.label ?? ""}`.trim();
    case "videoInput":
      return `${names.input?.(c.input.id) ?? c.input.name ?? "Video input"} ${not ? "not live" : "live"}`;
    case "message": {
      const n = c.message ? (names.message?.(c.message.id) ?? c.message.name ?? "Message") : "Any message";
      return `${n} ${not ? "not showing" : "showing"}`;
    }
    case "stageMessage":
      return not ? "No stage message" : "Stage message showing";
    case "layer":
      return `${CONDITION_LAYERS.find((l) => l.v === c.layer)?.label ?? c.layer} layer ${not ? "clear" : "live"}`;
    case "playlistItem": {
      const w = PLAYLIST_ITEMS.find((p) => p.v === (c.which ?? "current"))?.label ?? "Item";
      return `${w} ${not ? "missing" : "exists"}`;
    }
    case "videoCountdown":
    case "audioCountdown":
      return `${c.type === "videoCountdown" ? "Video" : "Audio"} ${MEDIA_STATES.find((s) => s.v === (c.state ?? "hasTimeRemaining"))?.label ?? ""}`.trim();
    case "unsupported":
      return `Imported condition (${c.sourceType}), not evaluated`;
    default:
      return "Unknown condition";
  }
}

/** A timer link with another `show` (`value` is omitted, as the server writes it). */
export function timerShowLink(link: Extract<DataLink, { type: "timer" }>, show: TimerShow): DataLink {
  const { show: _, ...rest } = link;
  return show === "value" ? rest : { ...rest, show };
}
