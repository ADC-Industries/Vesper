// Slide actions in the Edit view: one-line summaries, new actions of each kind, and the edit
// ops that add, remove, move and change them (each a single undo step).

import type { ActionKind, ClearLayer, PresentationOp, SlideAction, TimerOperation } from "$protocol";
import { formatDuration } from "../pages/timers/timers";

/** Kinds the Actions menu can add. */
export const ADDABLE = ["timer", "message", "prop", "look", "clearLayer", "clearGroup", "stageLayout", "macro"] as const;
export type AddableKind = (typeof ADDABLE)[number];

export const KIND_LABELS: Record<ActionKind["type"], string> = {
  media: "Media",
  videoInput: "Video Input",
  clearLayer: "Clear Layer",
  clearGroup: "Clear Group",
  timer: "Timer",
  message: "Message",
  prop: "Prop",
  look: "Look",
  stageLayout: "Stage Layout",
  macro: "Macro",
  trigger: "Trigger",
  unsupported: "Unsupported",
  unknown: "Unknown",
};

/** Layers a Clear Layer action can target, in menu order. */
export const CLEAR_TARGETS: { layer: ClearLayer; label: string }[] = [
  { layer: "all", label: "All" },
  { layer: "slide", label: "Slide" },
  { layer: "media", label: "Media" },
  { layer: "videoInput", label: "Video Input" },
  { layer: "announcements", label: "Announcements" },
  { layer: "props", label: "Props" },
  { layer: "messages", label: "Messages" },
  { layer: "audio", label: "Audio" },
];

/** Names of the documents actions point at (a missing one shows the link's own name hint). */
export interface ActionNames {
  timers?: Map<string, string>;
  messages?: Map<string, string>;
  props?: Map<string, string>;
  looks?: Map<string, string>;
  clearGroups?: Map<string, string>;
}

function named(map: Map<string, string> | undefined, link: { id: string; name?: string }): string {
  return map?.get(link.id) ?? (link.name ? `${link.name} (missing)` : "(missing)");
}

export function timerOpLabel(op: TimerOperation | undefined): string {
  const o = op ?? { type: "start" };
  switch (o.type) {
    case "start":
      return "Start";
    case "stop":
      return "Stop";
    case "reset":
      return "Reset";
    case "restart":
      return "Reset and start";
    case "set":
      return `Set to ${formatDuration(o.seconds)}`;
    case "adjust":
      return `${o.seconds < 0 ? "Subtract" : "Add"} ${formatDuration(Math.abs(o.seconds))}`;
    default:
      return "Unknown operation";
  }
}

/** One line describing what an action does. */
export function actionSummary(a: SlideAction, names: ActionNames = {}): string {
  const k = a.kind;
  switch (k.type) {
    case "media":
      return `${a.name ?? "Media"} (${k.layer ?? "background"})`;
    case "timer":
      return `${timerOpLabel(k.operation)}: ${named(names.timers, k.timer)}`;
    case "message":
      return `${k.show === false ? "Hide" : "Show"}: ${named(names.messages, k.message)}`;
    case "prop":
      return `${k.show === false ? "Hide" : "Show"}: ${named(names.props, k.prop)}`;
    case "look":
      return named(names.looks, k.look);
    case "clearLayer": {
      const t = (k.targets ?? []).map((x) => CLEAR_TARGETS.find((c) => c.layer === (x.layer ?? "all"))?.label ?? x.layer ?? "All");
      return t.length ? t.join(", ") : "Nothing";
    }
    case "clearGroup":
      return named(names.clearGroups, k.group);
    case "stageLayout": {
      const n = k.assignments?.length ?? 0;
      return n ? k.assignments!.map((x) => x.layout.name ?? x.layout.id.slice(0, 8)).join(", ") : "No layout";
    }
    case "macro":
      return k.target.name ?? k.target.id.slice(0, 8);
    case "videoInput":
      return a.name ?? "Video input";
    case "trigger":
      return k.target.type === "presentation" ? (k.target.presentation.name ?? "Presentation") : "Playlist item";
    case "unsupported":
      return `${k.sourceType} (not supported)`;
    default:
      return "Unknown action (from a newer version)";
  }
}

/** Kinds this app edits (the rest are listed read-only). */
export function isEditable(a: SlideAction): boolean {
  return (ADDABLE as readonly string[]).includes(a.kind.type) && a.kind.type !== "macro" && a.kind.type !== "stageLayout";
}

/** First choices for a new action (the first timer, message, …); null when there is none. */
export interface Firsts {
  timer?: { id: string; name?: string };
  message?: { id: string; name?: string };
  prop?: { id: string; name?: string };
  look?: { id: string; name?: string };
  clearGroup?: { id: string; name?: string };
}

/** A new enabled action of a kind, or null when its target list is empty. */
export function newAction(kind: AddableKind, id: string, first: Firsts): SlideAction | null {
  const link = (x: { id: string; name?: string } | undefined) => (x ? { id: x.id, ...(x.name ? { name: x.name } : {}) } : null);
  let k: ActionKind | null = null;
  switch (kind) {
    case "timer": {
      const l = link(first.timer);
      if (l) k = { type: "timer", timer: l, operation: { type: "start" } };
      break;
    }
    case "message": {
      const l = link(first.message);
      if (l) k = { type: "message", message: l };
      break;
    }
    case "prop": {
      const l = link(first.prop);
      if (l) k = { type: "prop", prop: l };
      break;
    }
    case "look": {
      const l = link(first.look);
      if (l) k = { type: "look", look: l };
      break;
    }
    case "clearGroup": {
      const l = link(first.clearGroup);
      if (l) k = { type: "clearGroup", group: l };
      break;
    }
    case "clearLayer":
      k = { type: "clearLayer", targets: [{ layer: "slide" }] };
      break;
    case "stageLayout":
    case "macro":
      return null;
  }
  return k ? { id, kind: k } : null;
}

// ---- ops ---------------------------------------------------------------------------------

export function addActionOp(slide: string, actions: SlideAction[], action: SlideAction): PresentationOp {
  const last = actions[actions.length - 1];
  return { type: "addSlideAction", slide, action, ...(last ? { after: last.id } : {}) };
}

export function removeActionOp(slide: string, action: string): PresentationOp {
  return { type: "removeSlideAction", slide, action };
}

export function setActionOp(slide: string, action: SlideAction): PresentationOp {
  return { type: "setSlideAction", slide, action };
}

/** Moves the action at `index` by `delta` places (null when it cannot move). */
export function moveActionOp(slide: string, actions: SlideAction[], index: number, delta: -1 | 1): PresentationOp | null {
  const to = index + delta;
  if (index < 0 || index >= actions.length || to < 0 || to >= actions.length) return null;
  const rest = actions.filter((_, i) => i !== index);
  const after = to === 0 ? undefined : rest[to - 1]!.id;
  return { type: "moveSlideAction", slide, action: actions[index]!.id, ...(after ? { after } : {}) };
}

/** Moves an action so it ends at position `to` (drag and drop); null when nothing moves. */
export function moveActionTo(slide: string, actions: SlideAction[], from: number, to: number): PresentationOp | null {
  if (from === to || from < 0 || from >= actions.length) return null;
  const rest = actions.filter((_, i) => i !== from);
  const at = Math.max(0, Math.min(to, rest.length));
  const after = at === 0 ? undefined : rest[at - 1]!.id;
  return { type: "moveSlideAction", slide, action: actions[from]!.id, ...(after ? { after } : {}) };
}
