// The transitions grid (Presentation and Slide tabs): every kind the server draws (plan G16),
// with a direction where it has one, the duration and the easing.

import type { Direction, Easing, Transition, TransitionKind } from "$protocol";
import { transitionChoice } from "../../edit/ops";

export type GridTransition = "cut" | "dissolve" | "push" | "wipe" | "zoom" | "slide";

export const TRANSITION_GRID: { id: GridTransition; label: string; drawn: boolean }[] = [
  { id: "cut", label: "Cut", drawn: true },
  { id: "dissolve", label: "Dissolve", drawn: true },
  { id: "push", label: "Push", drawn: true },
  { id: "wipe", label: "Wipe", drawn: true },
  { id: "zoom", label: "Zoom", drawn: true },
  { id: "slide", label: "Slide", drawn: true },
];

/** The grid cell a stored transition shows as (null: the default, or an imported one). */
export function gridChoice(t: Transition | undefined): GridTransition | null {
  const c = transitionChoice(t);
  return c === "default" || c === "other" ? null : c;
}

/** Directions offered per kind (arrows); a wipe can also open from the centre. */
export const DIRECTIONS: { id: Direction; label: string; glyph: string }[] = [
  { id: "left", label: "Left", glyph: "M19 12H5M11 6l-6 6 6 6" },
  { id: "right", label: "Right", glyph: "M5 12h14M13 6l6 6-6 6" },
  { id: "up", label: "Up", glyph: "M12 19V5M6 11l6-6 6 6" },
  { id: "down", label: "Down", glyph: "M12 5v14M6 13l6 6 6-6" },
  { id: "center", label: "From the centre", glyph: "M8 8h8v8H8zM4 4l3 3M20 4l-3 3M4 20l3-3M20 20l-3-3" },
];

export function directionsFor(kind: GridTransition | null): typeof DIRECTIONS {
  if (kind === "wipe") return DIRECTIONS;
  if (kind === "push" || kind === "slide") return DIRECTIONS.filter((d) => d.id !== "center");
  return [];
}

export const EASINGS: { id: Easing; label: string }[] = [
  { id: "linear", label: "Linear" },
  { id: "easeIn", label: "Ease in" },
  { id: "easeOut", label: "Ease out" },
  { id: "easeInOut", label: "Ease in and out" },
];

/** `t` with another direction (push / wipe / slide) or zoom way. */
export function withDirection(t: Transition, direction: Direction | "in" | "out"): Transition {
  const k = t.kind;
  if (k?.type === "zoom") return { ...t, kind: direction === "out" ? { type: "zoom", direction: "out" } : { type: "zoom" } };
  if (k && (k.type === "push" || k.type === "wipe" || k.type === "slide") && direction !== "in" && direction !== "out") {
    const kind: TransitionKind = k.type === "push" ? { type: "push", direction } : k.type === "wipe" ? { type: "wipe", direction } : { type: "slide", direction };
    return { ...t, kind };
  }
  return t;
}

/** The direction a stored transition shows as selected. */
export function directionOf(t: Transition | undefined): Direction | "in" | "out" | null {
  const k = t?.kind;
  if (!k) return null;
  if (k.type === "zoom") return k.direction === "out" ? "out" : "in";
  if (k.type === "push" || k.type === "wipe" || k.type === "slide") return k.direction;
  return null;
}
