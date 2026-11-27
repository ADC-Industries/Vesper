// Object builds (plan G18, docs/BUILDS-PLAN.md): the slide's build order, its steps as the
// server plays them, and what the editor's step preview hides. Mirrors `model::build`.

import type { Build, BuildStart, Direction, Slide, Transition, TransitionKind } from "$protocol";

export type TimedBuild = { index: number; step: number; start: number; duration: number };

function duration(b: Build): number {
  const t = b.transition;
  if (!t || !t.kind || t.kind.type === "cut") return 0;
  return Math.max(0, t.duration ?? 0);
}

/** The builds that name an element of the slide, placed in steps (0 = with the slide) and time. */
export function buildSteps(slide: Pick<Slide, "builds" | "elements">): TimedBuild[] {
  const ids = new Set((slide.elements ?? []).map((e) => e.id));
  const out: TimedBuild[] = [];
  let step = 0;
  const prev: [number, number][] = [[0, 0]];
  (slide.builds ?? []).forEach((b, index) => {
    if (!ids.has(b.element)) return;
    const delay = Math.max(0, b.delay ?? 0);
    const start: BuildStart = b.start ?? "onNext";
    let s: number;
    let at: number;
    if (start === "withSlide") {
      s = 0;
      at = delay;
    } else if (start === "withPrevious") {
      s = step;
      at = (prev[step]?.[0] ?? 0) + delay;
    } else if (start === "afterPrevious") {
      s = step;
      at = (prev[step]?.[1] ?? 0) + delay;
    } else {
      step += 1;
      prev.push([0, 0]);
      s = step;
      at = delay;
    }
    const d = duration(b);
    prev[s] = [at, at + d];
    out.push({ index, step: s, start: at, duration: d });
  });
  return out;
}

/** Number of `next` steps the builds take. */
export function stepCount(slide: Pick<Slide, "builds" | "elements">): number {
  return buildSteps(slide).reduce((m, t) => Math.max(m, t.step), 0);
}

/** Elements hidden when the slide is settled at `step` (in builds not yet played, out builds played). */
export function hiddenAt(slide: Pick<Slide, "builds" | "elements">, step: number): string[] {
  const hidden = new Set<string>();
  for (const t of buildSteps(slide)) {
    const b = slide.builds?.[t.index];
    if (!b) continue;
    const done = t.step <= step;
    const out = b.mode === "out";
    if (out ? done : !done) hidden.add(b.element);
    else hidden.delete(b.element);
  }
  return [...hidden];
}

export const TRIGGERS: { id: BuildStart; label: string }[] = [
  { id: "onNext", label: "On next" },
  { id: "withPrevious", label: "With previous" },
  { id: "afterPrevious", label: "After previous" },
  { id: "withSlide", label: "With the slide" },
];

export type BuildEffect = "cut" | "dissolve" | "push" | "slide" | "zoom" | "wipe";

export const EFFECTS: { id: BuildEffect; label: string }[] = [
  { id: "dissolve", label: "Dissolve" },
  { id: "push", label: "Push" },
  { id: "slide", label: "Slide" },
  { id: "zoom", label: "Zoom" },
  { id: "wipe", label: "Wipe (plays as a dissolve)" },
  { id: "cut", label: "Cut" },
];

/** The effect a build's transition shows as (imported vendor kinds play as a dissolve). */
export function effectOf(t: Transition | undefined): BuildEffect {
  const k = t?.kind?.type;
  if (k === "cut" || k === "push" || k === "slide" || k === "zoom" || k === "wipe") return k;
  return k === undefined ? "cut" : "dissolve";
}

/** A transition of `effect`, keeping the duration (and direction where both have one). */
export function withEffect(t: Transition | undefined, effect: BuildEffect): Transition {
  const duration = t?.duration && t.duration > 0 ? t.duration : 0.5;
  const k = t?.kind;
  const dir = k && (k.type === "push" || k.type === "slide" || k.type === "wipe") ? k.direction : "left";
  switch (effect) {
    case "cut":
      return { kind: { type: "cut" } };
    case "dissolve":
      return { kind: { type: "dissolve" }, duration };
    case "zoom":
      return { kind: { type: "zoom" }, duration, easing: "easeOut" };
    case "push":
    case "slide":
    case "wipe": {
      const direction: Direction = dir === "center" && effect !== "wipe" ? "left" : dir;
      const kind: TransitionKind = effect === "push" ? { type: "push", direction } : effect === "slide" ? { type: "slide", direction } : { type: "wipe", direction };
      return { kind, duration, easing: "easeInOut" };
    }
  }
}

/** A new build of `element`: in or out on next, a half-second dissolve. */
export function newBuild(element: string, mode: "in" | "out"): Build {
  return { element, ...(mode === "out" ? { mode: "out" as const } : {}), transition: { kind: { type: "dissolve" }, duration: 0.5 } };
}

/** `builds` with the one at `i` moved by `d` (−1 up, +1 down). */
export function moveBuild(builds: Build[], i: number, d: number): Build[] {
  const j = i + d;
  if (j < 0 || j >= builds.length) return builds;
  const out = builds.slice();
  const a = out[i];
  const b = out[j];
  if (!a || !b) return builds;
  out[i] = b;
  out[j] = a;
  return out;
}

/** "Step 2 · Title in" style summary of a build row. */
export function buildLabel(b: Build, name: string): string {
  return `${name || "Object"} ${b.mode === "out" ? "out" : "in"}`;
}
