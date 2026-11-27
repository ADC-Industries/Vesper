// The Reflow page's per-slide "…" menu and badges: what each entry does as edit ops, and what a
// slide shows (hotkey, media cue). Pure functions, unit tested; the menu itself is SlideMenu.svelte.
//
// Groups are the presentation's sections. "Start a group here" is `groupOps` (edit/reflow.ts);
// "Continue the group above" takes a group's first slide out of its group: the whole section
// moves into the previous one (`moveSlide` for each slide, then the empty section is deleted),
// as the canvas draws it.

import type { Presentation, PresentationOp, Section, Slide, SlideAction } from "$protocol";
import { hotkeyLabel } from "../../edit/ops";
import type { GroupChoice } from "../../edit/reflow";

/** Where a slide sits: its section, the section before, and its position. */
export function locate(p: Presentation, slideId: string): { si: number; k: number; section: Section; prev: Section | null } | null {
  const sections = p.sections ?? [];
  for (let si = 0; si < sections.length; si++) {
    const sec = sections[si]!;
    const k = (sec.slides ?? []).findIndex((s) => s.id === slideId);
    if (k >= 0) return { si, k, section: sec, prev: si > 0 ? sections[si - 1]! : null };
  }
  return null;
}

/** Whether "Continue the group above" applies: the slide starts a group that has one above it. */
export function canContinueAbove(p: Presentation, slideId: string): boolean {
  const at = locate(p, slideId);
  return !!at && at.k === 0 && !!at.prev;
}

/**
 * "Continue the group above": the slide's group ends, its slides join the group above (moved
 * after that group's last slide, in order) and the empty section goes.
 */
export function continueGroupOps(p: Presentation, slideId: string): PresentationOp[] {
  const at = locate(p, slideId);
  if (!at || at.k !== 0 || !at.prev) return [];
  const prevSlides = at.prev.slides ?? [];
  let after: string | undefined = prevSlides[prevSlides.length - 1]?.id;
  const ops: PresentationOp[] = [];
  for (const s of at.section.slides ?? []) {
    ops.push({ type: "moveSlide", slide: s.id, toSection: at.prev.id, ...(after ? { after } : {}) });
    after = s.id;
  }
  ops.push({ type: "deleteSection", section: at.section.id });
  return ops;
}

/** "Delete slide": the slide, and its section too when that would be left empty. */
export function deleteSlideOps(p: Presentation, slideId: string): PresentationOp[] {
  const at = locate(p, slideId);
  if (!at) return [];
  const ops: PresentationOp[] = [{ type: "deleteSlide", slide: slideId }];
  if ((at.section.slides ?? []).length === 1) ops.push({ type: "deleteSection", section: at.section.id });
  return ops;
}

/** The slide's media cues: its media actions (a slide normally has at most one). */
export function mediaCues(slide: Slide | undefined): SlideAction[] {
  return (slide?.actions ?? []).filter((a) => a.kind.type === "media");
}

/** "Remove media cue": every media action of the slide. */
export function removeCueOps(slide: Slide | undefined): PresentationOp[] {
  if (!slide) return [];
  return mediaCues(slide).map((a) => ({ type: "removeSlideAction", slide: slide.id, action: a.id }));
}

/** What the slide's header and thumbnail show besides its number. */
export interface SlideBadges {
  /** The slide's hotkey, else its group's (on the group's first slide), as a short label. */
  hotkey: string;
  /** The first media cue's name ("Media" when it has none), or "" without a cue. */
  cue: string;
  /** More than one media cue. */
  cueCount: number;
}

export function slideBadges(slide: Slide | undefined, section: Section | undefined, firstInSection: boolean): SlideBadges {
  const cues = mediaCues(slide);
  const hk = hotkeyLabel(slide?.hotkey) || (firstInSection ? hotkeyLabel(section?.hotkey) : "");
  return { hotkey: hk, cue: cues.length ? cues[0]!.name?.trim() || "Media" : "", cueCount: cues.length };
}

// ---- the menu model ---------------------------------------------------------------------------

export type SlideMenuAction = "continue" | "break" | "merge" | "removeCue" | "delete" | "newGroup" | "translate";

export interface SlideMenuEntry {
  id: string;
  /** A group to start here, or an action. */
  group?: GroupChoice;
  action?: SlideMenuAction;
  label?: string;
  /** A section heading (not focusable). */
  heading?: boolean;
  separator?: boolean;
  disabled?: boolean;
  danger?: boolean;
  /** The group this slide starts now. */
  checked?: boolean;
}

/**
 * The entries of a slide's menu: "Start a group here" (the presentation's groups, then the
 * global ones, then "New group…"), then continue above, slide break, merge up, remove media
 * cue (only with a cue) and delete.
 */
export function slideMenuEntries(opts: {
  choices: { sections: GroupChoice[]; global: GroupChoice[] };
  /** The name of the group this slide starts, or null when it continues one. */
  startsGroup: string | null;
  isFirst: boolean;
  canContinue: boolean;
  hasCue: boolean;
  /** Offer "Translate this slide…" (the box has translation models). */
  translate?: boolean;
}): SlideMenuEntry[] {
  const own = opts.startsGroup?.toLowerCase() ?? null;
  const group = (g: GroupChoice, i: number, kind: string): SlideMenuEntry => ({
    id: `${kind}:${i}`,
    group: g,
    label: g.name,
    checked: own !== null && g.name.toLowerCase() === own,
  });
  const out: SlideMenuEntry[] = [{ id: "h:groups", heading: true, label: "Start a group here" }];
  out.push(...opts.choices.sections.map((g, i) => group(g, i, "s")));
  if (opts.choices.sections.length && opts.choices.global.length) out.push({ id: "sep:global", separator: true });
  out.push(...opts.choices.global.map((g, i) => group(g, i, "g")));
  out.push({ id: "newGroup", action: "newGroup", label: "New group…" });
  out.push({ id: "sep:actions", separator: true });
  out.push({ id: "continue", action: "continue", label: "Continue the group above", disabled: !opts.canContinue });
  out.push({ id: "break", action: "break", label: "Insert slide break at cursor" });
  out.push({ id: "merge", action: "merge", label: "Merge with slide above", disabled: opts.isFirst });
  if (opts.hasCue) out.push({ id: "removeCue", action: "removeCue", label: "Remove media cue" });
  if (opts.translate) out.push({ id: "translate", action: "translate", label: "Translate this slide…" });
  out.push({ id: "delete", action: "delete", label: "Delete slide", danger: true });
  return out;
}

/** Whether an entry can take the focus (for `menuMove` / `menuTypeahead`). */
export const entryFocusable = (e: SlideMenuEntry): boolean => !e.heading && !e.separator && !e.disabled;
