// The Looks tab's pure logic (plan 4.8, docs/LOOKS-MASKS-PLAN.md): the layer matrix of a look
// (one entry per audience screen; a screen it doesn't list shows every layer), the theme
// override, reordering by sort keys, and whether a draft differs from the stored look.
import type { LanguageDisplay, LayerVisibility, Look, ScreenLook, TemplateOverride } from "$protocol";

/** The layer rows of the matrix, top to bottom as on the canvas board. */
export const LAYER_ROWS: { key: keyof LayerVisibility; label: string; colour?: string }[] = [
  { key: "captions", label: "Captions" },
  { key: "messages", label: "Messages", colour: "var(--layer-messages)" },
  { key: "props", label: "Props", colour: "var(--layer-props)" },
  { key: "announcements", label: "Announcements" },
  { key: "slide", label: "Slide", colour: "var(--layer-slide)" },
  { key: "media", label: "Media", colour: "var(--layer-media)" },
  { key: "videoInput", label: "Video input" },
];

/** The look's entry for `screen`, or null when it doesn't list it (every layer shown). */
export function entryOf(look: Look | null | undefined, screen: string): ScreenLook | null {
  return look?.screens?.find((s) => s.screen === screen) ?? null;
}

/** Whether `look` draws `layer` on `screen` (no look: every layer). */
export function shows(look: Look | null | undefined, screen: string, layer: keyof LayerVisibility): boolean {
  return entryOf(look, screen)?.layers?.[layer] !== false;
}

/** An entry that changes nothing (every layer, no override, no mask, automatic language). */
function isDefault(e: ScreenLook): boolean {
  const hidden = Object.values(e.layers ?? {}).some((v) => v === false);
  return !hidden && !e.templateOverride && !e.mask && (e.language ?? "auto") === "auto";
}

/** `look` with `screen`'s entry changed by `edit` (created when missing, dropped when it ends
 * up changing nothing). Never mutates `look`. */
export function withEntry(look: Look, screen: string, edit: (e: ScreenLook) => ScreenLook): Look {
  const list = look.screens ?? [];
  const i = list.findIndex((s) => s.screen === screen);
  const cur: ScreenLook = list[i] ?? { screen };
  const next = edit({ ...cur, layers: { ...(cur.layers ?? {}) } });
  const screens = i >= 0 ? list.map((s, j) => (j === i ? next : s)) : [...list, next];
  return { ...look, screens: screens.filter((s) => !isDefault(s)) };
}

/** `look` with `layer` shown or hidden on `screen` (a shown layer is left out: the default). */
export function withLayer(look: Look, screen: string, layer: keyof LayerVisibility, on: boolean): Look {
  return withEntry(look, screen, (e) => {
    const layers = { ...(e.layers ?? {}) };
    if (on) delete layers[layer];
    else layers[layer] = false;
    return { ...e, layers };
  });
}

/** `look` with `screen`'s theme override set (null: none). */
export function withTheme(look: Look, screen: string, t: TemplateOverride | null): Look {
  return withEntry(look, screen, (e) => {
    const n = { ...e };
    if (t) n.templateOverride = t;
    else delete n.templateOverride;
    return n;
  });
}

/** `look` with `screen`'s mask set (null: none). */
export function withMask(look: Look, screen: string, mask: string | null): Look {
  return withEntry(look, screen, (e) => {
    const n = { ...e };
    if (mask) n.mask = mask;
    else delete n.mask;
    return n;
  });
}

/** How `look` shows second-language text on `screen` (batch 16; `auto` when not set). */
export function languageOf(look: Look | null | undefined, screen: string): LanguageDisplay {
  return entryOf(look, screen)?.language ?? "auto";
}

/** `look` with `screen`'s second-language display set (`auto`: the default, left out). */
export function withLanguage(look: Look, screen: string, language: LanguageDisplay): Look {
  return withEntry(look, screen, (e) => {
    const n = { ...e };
    if (language !== "auto") n.language = language;
    else delete n.language;
    return n;
  });
}

/** A comparable form of a look: entries without empty layer maps, in screen order. */
function canon(l: Look): string {
  const screens = (l.screens ?? [])
    .map((s) => {
      const c: Record<string, unknown> = { screen: s.screen };
      if (s.layers && Object.values(s.layers).some((v) => v === false)) {
        c.layers = Object.fromEntries(Object.entries(s.layers).filter(([, v]) => v === false).sort());
      }
      if (s.templateOverride) c.templateOverride = s.templateOverride;
      if (s.mask) c.mask = s.mask;
      if (s.language && s.language !== "auto") c.language = s.language;
      return c;
    })
    .sort((a, b) => String(a.screen).localeCompare(String(b.screen)));
  return JSON.stringify({ name: (l.name ?? "").trim(), screens, transition: l.transition ?? null });
}

/** Whether the draft differs from the stored look (name, matrix, transition). */
export function isDirty(draft: Look | null, stored: Look | null): boolean {
  if (!draft || !stored) return false;
  return canon(draft) !== canon(stored);
}

/**
 * A click on a look in the list (`id`; the Live box: null, follow the live look) that resolves
 * to the look already selected: the new `picked`, without asking about unsaved edits. A draft
 * with unsaved edits stays pinned to its look (`selId`), so a later live-look change can't drop
 * it (D08-3, E08-1). `undefined`: another look, a real switch (ask first when dirty).
 */
export function reselect(id: string | null, activeLook: string | null, selId: string, dirty: boolean): string | null | undefined {
  if ((id ?? activeLook ?? "") !== selId) return undefined;
  return dirty ? selId : id;
}

/** `looks` with `look` in place of the stored one of its ID (this client's own write). */
export function withStored(looks: Look[], look: Look): Look[] {
  return looks.map((l) => (l.id === look.id ? look : l));
}

// ---- ordering (fractional sort keys, as `model::sort_key_between`) ----------------------------

const DIGITS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const digit = (c: string | undefined) => (c === undefined ? 0 : Math.max(0, DIGITS.indexOf(c)));

function midpoint(a: string, b: string | null): string {
  if (b !== null) {
    let n = 0;
    while (n < b.length && (a[n] ?? "0") === b[n]) n++;
    if (n > 0) return b.slice(0, n) + midpoint(a.slice(n), b.slice(n));
  }
  const da = digit(a[0]);
  const db = b !== null && b.length ? digit(b[0]) : DIGITS.length;
  if (db > da + 1) return DIGITS.charAt(Math.floor((da + db) / 2));
  if (b !== null && b.length > 1) return b.charAt(0);
  return DIGITS.charAt(da) + midpoint(a.slice(1), null);
}

const valid = (k: string) => !k.endsWith("0") && [...k].every((c) => DIGITS.includes(c));

/** A sort key strictly between `before` and `after` (null: an open end). */
export function sortKeyBetween(before: string | null, after: string | null): string {
  const a = before ?? "";
  if (valid(a) && (after === null || valid(after))) return midpoint(a, after);
  return `${a}V`;
}

/** The new sort key that moves look `id` one place up (`-1`) or down (`1`) in `looks` (sorted),
 * or null when it can't move. */
export function movedKey(looks: Look[], id: string, dir: -1 | 1): string | null {
  const i = looks.findIndex((l) => l.id === id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= looks.length) return null;
  const key = (k: number) => (k >= 0 && k < looks.length ? (looks[k]?.sortKey ?? "") : null);
  // Up: between the one above the neighbour and the neighbour; down: between the neighbour
  // and the one below it.
  const [lo, hi] = dir < 0 ? [key(j - 1), key(j)] : [key(j), key(j + 1)];
  if (lo !== null && hi !== null && lo >= hi) return null;
  return sortKeyBetween(lo || null, hi);
}

/** A free "Look N" name. */
export function freeName(looks: Look[], base = "Look"): string {
  const taken = new Set(looks.map((l) => (l.name ?? "").trim().toLowerCase()));
  for (let n = 1; ; n++) if (!taken.has(`${base} ${n}`.toLowerCase())) return `${base} ${n}`;
}
