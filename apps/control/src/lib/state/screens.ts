// Pure logic of the Screen Configuration dialog: presets, labels, the staged draft of one
// screen and its outputs, client-side validation and the requests an Apply needs.
// No Svelte here, so it is unit-tested (screenConfig.test.ts).

import type {
  AlphaMode,
  Color,
  DisplayInfo,
  DisplayMode,
  FrameRate,
  MaskEntry,
  OutputAlpha,
  OutputBinding,
  OutputEntry,
  OutputTransport,
  Resolution,
  Screen,
  ScreenEntry,
  TestPattern,
} from "$protocol";

export interface RatePreset {
  label: string;
  rate: FrameRate;
}

export const RATE_PRESETS: RatePreset[] = [
  { label: "23.976", rate: { num: 24000, den: 1001 } },
  { label: "24", rate: { num: 24, den: 1 } },
  { label: "25", rate: { num: 25, den: 1 } },
  { label: "29.97", rate: { num: 30000, den: 1001 } },
  { label: "30", rate: { num: 30, den: 1 } },
  { label: "50", rate: { num: 50, den: 1 } },
  { label: "59.94", rate: { num: 60000, den: 1001 } },
  { label: "60", rate: { num: 60, den: 1 } },
];

export interface SizePreset {
  label: string;
  width: number;
  height: number;
}

export const SIZE_PRESETS: SizePreset[] = [
  { label: "1920 x 1080 (HD)", width: 1920, height: 1080 },
  { label: "1280 x 720", width: 1280, height: 720 },
  { label: "3840 x 2160 (4K)", width: 3840, height: 2160 },
  { label: "1080 x 1920 (portrait)", width: 1080, height: 1920 },
];

/** Limits the server enforces too (kept in step with crates/vesper-server validation). */
export const LIMITS = { minSide: 16, maxWidth: 7680, maxHeight: 7680, minFps: 1, maxFps: 120, maxDelay: 30, maxName: 100 };

export const ALPHA_MODES: { value: AlphaMode; label: string; hint: string }[] = [
  { value: "opaque", label: "Opaque", hint: "No alpha channel: transparent areas are sent as the screen colour or black." },
  { value: "straight", label: "Straight key", hint: "Sends alpha with unmultiplied colour (most keyers and vision mixers)." },
  { value: "premultiplied", label: "Premultiplied key", hint: "Sends alpha with colour already multiplied by alpha (some compositors expect this)." },
];

export const PATTERNS: { type: Exclude<TestPattern["type"], "unknown">; label: string; hint: string }[] = [
  { type: "colorBars", label: "Colour Bars", hint: "75 % bars: check colours and levels" },
  { type: "grid", label: "Grid", hint: "Grid, circle and safe areas: check scaling and geometry" },
  { type: "gradient", label: "Gradient Ramps", hint: "Smooth ramps: check banding and clipping" },
  { type: "solid", label: "Solid Colour", hint: "One colour over the whole screen" },
  { type: "alphaChecker", label: "Alpha Checker", hint: "Opaque / transparent squares: check a key output's alpha" },
  { type: "motion", label: "Motion", hint: "A sweeping bar and frame counter: check smoothness and frame rate" },
];

export function patternLabel(p: TestPattern | null | undefined): string {
  if (!p) return "Off";
  return PATTERNS.find((x) => x.type === p.type)?.label ?? "Test pattern";
}

export function fps(r: FrameRate): number {
  return r.num / Math.max(1, r.den);
}

export function fpsLabel(r: FrameRate): string {
  const preset = RATE_PRESETS.find((p) => sameRate(p.rate, r));
  if (preset) return preset.label;
  const v = fps(r);
  return Number.isInteger(v) ? String(v) : v.toFixed(3).replace(/0+$/, "");
}

export function sameRate(a: FrameRate, b: FrameRate): boolean {
  return a.num * b.den === b.num * a.den;
}

/** "1080p30", "720p59.94", or "1080x1920p30" for non-16:9 sizes. */
export function modeLabel(width: number, height: number, rate: FrameRate): string {
  const std = width * 9 === height * 16;
  return `${std ? height : `${width}x${height}`}p${fpsLabel(rate)}`;
}

export const SCREEN_DEFAULTS = { width: 1920, height: 1080, rate: { num: 30, den: 1 } as FrameRate };

export function screenSize(s: Screen): { width: number; height: number } {
  return { width: Math.round(s.size?.width ?? SCREEN_DEFAULTS.width), height: Math.round(s.size?.height ?? SCREEN_DEFAULTS.height) };
}

export function screenRate(s: Screen): FrameRate {
  return s.frameRate ?? SCREEN_DEFAULTS.rate;
}

/** A screen's role; a signage channel's screen is its own kind, never an operator screen (H02-7). */
export function screenRole(s: Screen): "audience" | "stage" | "signage" {
  const t = s.kind?.type;
  return t === "stage" ? "stage" : t === "signage" ? "signage" : "audience";
}

/** Operator screens (Configure Screens): signage channels' screens are made and kept by their
 * channel (Site ▸ Signage), never configured here (H02-7). */
export function operatorScreens<T extends Screen>(screens: T[]): T[] {
  return screens.filter((s) => screenRole(s) !== "signage");
}

/**
 * The screen with a new role. Only a stage screen has its own mask, so an audience screen
 * drops it (X2-3); a screen going back to Stage gets its stored stage settings again.
 */
export function withRole(s: Screen, base: Screen, role: string): Screen {
  // A signage channel's screen keeps its role (the server refuses another).
  if (screenRole(base) === "signage") return s;
  if (role === "stage") {
    const back = base.kind?.type === "stage";
    const n: Screen = { ...s, kind: back ? base.kind : { type: "stage" } };
    if (back && base.mask && !n.mask) n.mask = base.mask;
    return n;
  }
  const { mask: _m, ...rest } = s;
  return { ...rest, kind: { type: "audience" } };
}

/** One choice of the stage mask picker (a `Menu` item). */
export interface MaskChoice {
  id: string;
  label: string;
  checked?: boolean;
  separator?: boolean;
}

/**
 * A stage screen's mask picker: the button's label and the menu (None, the site's masks, a
 * missing one kept so the operator sees what the screen names, "Edit masks…"). `masks` is null
 * while the list is loading: the button then says "Mask" for a set mask.
 */
export function maskChoices(masks: readonly MaskEntry[] | null, current: string | null | undefined): { label: string; missing: boolean; items: MaskChoice[] } {
  const cur = current ?? null;
  const found = cur ? masks?.find((m) => m.id === cur) : undefined;
  const missing = !!cur && !!masks && !found;
  const label = !cur ? "None" : found ? found.name || "Unnamed mask" : masks ? "Missing mask" : "Mask";
  const items: MaskChoice[] = [
    { id: "none", label: "None", checked: !cur },
    ...(masks ?? []).map((m) => ({ id: m.id, label: m.name || "Unnamed mask", checked: m.id === cur })),
  ];
  if (missing) items.push({ id: cur!, label: "Missing mask", checked: true });
  items.push({ id: "sep", label: "", separator: true }, { id: "edit", label: "Edit masks…" });
  return { label, missing, items };
}

/** The screen with `mask` as its own mask (null or "none": no mask). Only a stage screen has one. */
export function withScreenMask(s: Screen, mask: string | null): Screen {
  const { mask: _m, ...rest } = s;
  return mask && mask !== "none" && s.kind?.type === "stage" ? { ...rest, mask } : rest;
}

export function screenAlpha(s: Screen): AlphaMode {
  return s.alpha && s.alpha !== "unknown" ? s.alpha : "opaque";
}

// ---- colours ---------------------------------------------------------------------------

const hex2 = (v: number) =>
  Math.round(Math.min(1, Math.max(0, v)) * 255)
    .toString(16)
    .padStart(2, "0");

export function colorToHex(c: Color | undefined | null): string {
  if (!c) return "#000000";
  return `#${hex2(c.r)}${hex2(c.g)}${hex2(c.b)}`;
}

export function hexToColor(hex: string, a = 1): Color {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  if (!m) return { r: 0, g: 0, b: 0, a };
  const v = (s: string) => Math.round((parseInt(s, 16) / 255) * 1000) / 1000;
  return { r: v(m[1]!), g: v(m[2]!), b: v(m[3]!), a };
}

/** The screen colour is "on" when the background is not fully transparent. */
export function hasScreenColor(s: Screen): boolean {
  return !!s.background && (s.background.a ?? 1) > 0;
}

// ---- output modes ------------------------------------------------------------------------

export const FOLLOW = "follow";

export interface ModeOption {
  value: string;
  label: string;
}

export function encodeMode(res: Resolution | undefined, rate: FrameRate | undefined): string {
  if (!res && !rate) return FOLLOW;
  return `${res ? `${res.width}x${res.height}` : "screen"}@${rate ? `${rate.num}/${rate.den}` : "screen"}`;
}

export function decodeMode(v: string): { resolutionOverride?: Resolution; frameRateOverride?: FrameRate } {
  if (v === FOLLOW) return {};
  const [res, rate] = v.split("@");
  const out: { resolutionOverride?: Resolution; frameRateOverride?: FrameRate } = {};
  if (res && res !== "screen") {
    const [w, h] = res.split("x").map(Number);
    if (w && h) out.resolutionOverride = { width: w, height: h };
  }
  if (rate && rate !== "screen") {
    const [n, d] = rate.split("/").map(Number);
    if (n && d) out.frameRateOverride = { num: n, den: d };
  }
  return out;
}

/** "Follow screen (1080p30)", then 1080 / 720 / 2160 at every preset rate, plus the current
 * value when it is not one of those. */
export function modeOptions(screen: Screen, current: string): ModeOption[] {
  const { width, height } = screenSize(screen);
  const opts: ModeOption[] = [{ value: FOLLOW, label: `Follow screen (${modeLabel(width, height, screenRate(screen))})` }];
  for (const [w, h] of [
    [1920, 1080],
    [1280, 720],
    [3840, 2160],
  ] as const) {
    for (const p of RATE_PRESETS) {
      opts.push({ value: encodeMode({ width: w, height: h }, p.rate), label: modeLabel(w, h, p.rate) });
    }
  }
  if (!opts.some((o) => o.value === current)) {
    const m = decodeMode(current);
    const r = m.resolutionOverride ?? { width, height };
    opts.push({ value: current, label: `${modeLabel(r.width, r.height, m.frameRateOverride ?? screenRate(screen))} (custom)` });
  }
  return opts;
}

export function outputMode(o: OutputBinding): string {
  return encodeMode(o.resolutionOverride, o.frameRateOverride);
}

export function setOutputMode(o: OutputBinding, v: string): OutputBinding {
  const next: OutputBinding = { ...o };
  delete next.resolutionOverride;
  delete next.frameRateOverride;
  return { ...next, ...decodeMode(v) };
}

export function alphaOverrideValue(o: OutputBinding): string {
  return o.alphaOverride?.type ?? "follow";
}

export function setAlphaOverride(o: OutputBinding, v: string): OutputBinding {
  const next: OutputBinding = { ...o };
  delete next.alphaOverride;
  if (v === "opaque" || v === "straight" || v === "premultiplied") next.alphaOverride = { type: v } as OutputAlpha;
  return next;
}

// ---- transports ---------------------------------------------------------------------------

/** The network video transport (IDN Open Protocol, compatible with NDI®): named sources
 * that receivers list as "HOST (name)", so names must be unique on the server. */
export function isIdn(kind: string | undefined): kind is "idn" {
  return kind === "idn";
}

/** Transports with a source name receivers see. */
export function isNamed(kind: string | undefined): boolean {
  return isIdn(kind) || kind === "omt";
}

/** The name receivers see (IDN source name, OMT name), or the binding's name. */
export function sourceName(o: OutputBinding): string {
  const t = o.transport;
  if (t?.type === "idn") return t.sourceName;
  if (t?.type === "omt") return t.name;
  return o.name ?? "";
}

export function setSourceName(o: OutputBinding, name: string): OutputBinding {
  const t = o.transport;
  let transport: OutputTransport | undefined = t;
  if (t?.type === "idn") transport = { ...t, sourceName: name };
  else if (t?.type === "omt") transport = { ...t, name };
  return { ...o, name, transport };
}

/** Switches the transport type, keeping the name. */
export function setTransportKind(o: OutputBinding, kind: string): OutputBinding {
  const name = sourceName(o);
  let transport: OutputTransport;
  switch (kind) {
    case "idn": {
      const t = o.transport;
      const groups = t?.type === "idn" ? t.groups : undefined;
      transport = groups?.length ? { type: "idn", sourceName: name, groups } : { type: "idn", sourceName: name };
      break;
    }
    case "omt":
      transport = { type: "omt", name };
      break;
    case "display":
      transport = { type: "display" };
      break;
    case "sdi":
      transport = { type: "sdi", device: "" };
      break;
    case "placeholder":
      transport = { type: "placeholder" };
      break;
    default:
      transport = { type: "unknown" };
  }
  return { ...o, transport };
}

export function transportLabel(kind: string | undefined): string {
  switch (kind) {
    case "idn":
      return "IDN";
    case "omt":
      return "OMT";
    case "display":
      return "Display";
    case "sdi":
      return "SDI";
    case "placeholder":
      return "Placeholder";
    default:
      return "None";
  }
}

// ---- displays -------------------------------------------------------------------------------

/** The connector a display output uses ("" = none chosen). */
export function displayOf(o: OutputBinding): string {
  const t = o.transport;
  return t?.type === "display" ? (t.selector?.connector ?? "") : "";
}

/** "1920×1080 @ 60 Hz", "1920×1080 @ 59.94 Hz". */
export function displayModeLabel(m: DisplayMode): string {
  const hz = m.refreshMhz / 1000;
  return `${m.width}×${m.height} @ ${m.refreshMhz % 1000 === 0 ? hz.toFixed(0) : hz.toFixed(2)} Hz`;
}

/** Select value of a display mode ("" = the display's preferred mode). */
export function displayModeKey(m: DisplayMode | null | undefined): string {
  return m ? `${m.width}x${m.height}@${m.refreshMhz}` : "";
}

function parseDisplayMode(v: string): DisplayMode | null {
  const m = /^(\d+)x(\d+)@(\d+)$/.exec(v);
  return m ? { width: Number(m[1]), height: Number(m[2]), refreshMhz: Number(m[3]) } : null;
}

/** The output's display mode as a select value. */
export function displayModeValue(o: OutputBinding): string {
  const t = o.transport;
  return t?.type === "display" ? displayModeKey(t.mode) : "";
}

/** Sets (or, with "", clears) the display mode. */
export function setDisplayMode(o: OutputBinding, v: string): OutputBinding {
  const t = o.transport;
  if (t?.type !== "display") return o;
  const { mode: _old, ...rest } = t;
  const mode = parseDisplayMode(v);
  return { ...o, transport: mode ? { ...rest, mode } : rest };
}

/** Binds the output to a display (by connector, with the model to find it again), keeping its
 * mode when the new display offers it, and naming it after the display. */
export function setDisplay(o: OutputBinding, d: DisplayInfo): OutputBinding {
  const t = o.transport;
  const old = t?.type === "display" ? t.mode : undefined;
  const keep = old && d.modes.some((m) => displayModeKey(m) === displayModeKey(old));
  const selector = d.name && d.name !== d.id ? { connector: d.id, name: d.name } : { connector: d.id };
  const windowed = t?.type === "display" && t.fullscreen === false ? { fullscreen: false } : {};
  return { ...o, name: d.name, transport: keep ? { type: "display", selector, ...windowed, mode: old } : { type: "display", selector, ...windowed } };
}

/** The display output covers its monitor (the default) rather than showing a window. */
export function displayFullscreen(o: OutputBinding): boolean {
  const t = o.transport;
  return t?.type !== "display" || t.fullscreen !== false;
}

/** Full screen (the default, left out) or a plain window for rehearsal. */
export function setDisplayFullscreen(o: OutputBinding, full: boolean): OutputBinding {
  const t = o.transport;
  if (t?.type !== "display") return o;
  const { fullscreen: _old, ...rest } = t;
  return { ...o, transport: full ? rest : { ...rest, fullscreen: false } };
}

/** A point on the desktop, physical px. */
export interface DesktopPoint {
  x: number;
  y: number;
}

/** Why covering this display would cover the operator's own work, if it would: it is the
 * monitor Oriel itself is on (`here`: the middle of Oriel's window, when Oriel runs on the
 * server's computer), or the desktop's main monitor. */
export function displayRisk(d: DisplayInfo | undefined, here: DesktopPoint | "unknown" | null): string | null {
  if (!d) return null;
  // Oriel runs on the server's computer, but where its window is can't be known (Wayland, a
  // browser: its coordinates aren't the desktop's physical pixels with mixed scaling): any of
  // the desktop's monitors may be the one it is on (not a display driven directly, KMS).
  if (here === "unknown") return d.primary ? "the main monitor" : d.source === "window" ? "a monitor Oriel may be on" : null;
  const size = d.preferred ?? d.modes[0];
  if (here && d.position && size) {
    const { x, y } = d.position;
    if (here.x >= x && here.x < x + size.width && here.y >= y && here.y < y + size.height) return "the monitor Oriel is on";
  }
  if (d.primary) return "the main monitor";
  return null;
}

/** Whether the server runs on this computer (its address is a loopback one). */
export function serverIsLocal(baseUrl: string): boolean {
  try {
    const h = new URL(baseUrl).hostname.replace(/^\[|\]$/g, "");
    return h === "localhost" || h === "::1" || /^127\./.test(h);
  } catch {
    return false;
  }
}

export interface DisplayModeOption {
  value: string;
  label: string;
}

/** "Preferred (…)" and the display's modes; the current one is kept even if it's not offered. */
export function displayModeOptions(d: DisplayInfo | undefined, current: string): DisplayModeOption[] {
  const pref = d?.preferred ?? d?.modes[0];
  const opts: DisplayModeOption[] = [{ value: "", label: pref ? `Preferred (${displayModeLabel(pref)})` : "Preferred" }];
  for (const m of d?.modes ?? []) opts.push({ value: displayModeKey(m), label: displayModeLabel(m) });
  const cur = parseDisplayMode(current);
  if (cur && !opts.some((o) => o.value === current)) opts.push({ value: current, label: `${displayModeLabel(cur)} (not offered)` });
  return opts;
}

/** A display's line in the picker: name, connector, resolution, connection. */
export function displayLabel(d: DisplayInfo): string {
  const pref = d.preferred ?? d.modes[0];
  const parts = [d.name !== d.id ? `${d.name} (${d.id})` : d.id];
  if (!d.connected) parts.push("not connected");
  else if (pref) parts.push(`${pref.width}×${pref.height}`);
  if (d.position && d.connected) parts.push(`at ${d.position.x}, ${d.position.y}`);
  if (d.primary) parts.push("main monitor");
  return parts.join(" · ");
}

/** The first connected display that no output (other than `self`) uses; the main monitor
 * (the operator's, on a one-PC setup) only when there is no other. */
export function freeDisplay(displays: DisplayInfo[], used: string[]): DisplayInfo | undefined {
  const u = new Set(used.map((c) => c.toLowerCase()));
  const free = displays.filter((d) => d.connected && !u.has(d.id.toLowerCase()));
  return free.find((d) => !d.primary) ?? free[0];
}

// ---- fill / key ------------------------------------------------------------------------------

/** What an output sends: the picture, or one half of a Fill/Key pair for a hardware keyer. */
export const OUTPUT_TARGETS: { value: "full" | "fill" | "key"; label: string; hint: string }[] = [
  { value: "full", label: "Full", hint: "The whole picture." },
  { value: "fill", label: "Fill", hint: "The colour for a keyer's fill input, over black. Pair it with a Key output on another display." },
  { value: "key", label: "Key", hint: "The transparency as a grey picture (white = solid) for a keyer's key input. Pair it with a Fill output." },
];

/** The output's target as a select value ("full" also for a target this Oriel doesn't know). */
export function outputTarget(o: OutputBinding): "full" | "fill" | "key" {
  return o.target === "fill" || o.target === "key" ? o.target : "full";
}

/** Sets the target ("full", the default, is left out). */
export function setOutputTarget(o: OutputBinding, v: string): OutputBinding {
  const next: OutputBinding = { ...o };
  delete next.target;
  if (v === "fill" || v === "key") next.target = v;
  return next;
}

/** A hint under a Fill or Key output: what the keyer must be set to, and what to check. */
export function fillKeyHint(o: OutputBinding, screenAlpha: AlphaMode): string | null {
  const t = outputTarget(o);
  if (t === "full") return null;
  const alpha = o.alphaOverride?.type ?? screenAlpha;
  const notes: string[] = [];
  if (alpha === "opaque") notes.push("This screen is opaque: the key is solid white. Give the screen alpha (Straight key).");
  else if (t === "fill")
    notes.push(alpha === "premultiplied" ? "Shaped fill (premultiplied): turn the keyer's Pre Multiplied Key on." : "Unshaped fill (straight): turn the keyer's Pre Multiplied Key off.");
  if (isIdn(o.transport?.type)) notes.push("IDN already carries alpha: one Straight key output is usually enough.");
  return notes.join(" ") || null;
}

/** A new Key output pairing `fill`: the same screen, transport kind and delivery, the next free
 * display (display outputs) or a unique source name ("<name> Key"). */
export function keyOutputFor(fill: OutputBinding, taken: string[], displays: DisplayInfo[], used: string[]): DraftOutput {
  const kind = fill.transport?.type ?? "idn";
  const name = uniqueName(`${sourceName(fill) || fill.name || "Output"} Key`, taken);
  let doc: OutputBinding = {
    schemaVersion: 2,
    id: `new-${++tmp}`,
    name,
    screen: fill.screen,
    target: "key",
    transport: kind === "display" ? { type: "display" } : isIdn(kind) ? { type: "idn", sourceName: name } : { type: "omt", name },
  };
  if (kind === "display") {
    const d = freeDisplay(displays, used);
    if (d) doc = { ...setDisplay(doc, d), name };
  }
  for (const k of ["resolutionOverride", "frameRateOverride", "videoDelay"] as const) if (fill[k] !== undefined) (doc as Record<string, unknown>)[k] = fill[k];
  return { key: doc.id, base: null, doc, deleted: false };
}

// ---- the draft ------------------------------------------------------------------------------

export interface DraftOutput {
  /** Stable key for the UI (the binding ID, or a temporary one for a new output). */
  key: string;
  /** The stored entry (`null` = new, not created yet). */
  base: OutputEntry | null;
  doc: OutputBinding;
  /** Marked for deletion on Apply. */
  deleted: boolean;
}

export interface Draft {
  base: ScreenEntry;
  screen: Screen;
  outputs: DraftOutput[];
}

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

export function stripRevision<T extends { revision: string }>(e: T): Omit<T, "revision"> {
  const { revision: _r, ...rest } = e;
  return rest;
}

export function outputsOf(all: OutputEntry[], screen: string): OutputEntry[] {
  return all.filter((o) => o.screen === screen);
}

export function makeDraft(entry: ScreenEntry, outputs: OutputEntry[]): Draft {
  return {
    base: entry,
    screen: clone(stripRevision(entry)) as Screen,
    outputs: outputsOf(outputs, entry.id).map((o) => ({
      key: o.id,
      base: o,
      doc: clone(stripRevision(o)) as OutputBinding,
      deleted: false,
    })),
  };
}

/** JSON with keys sorted (so field order does not count as a change). */
export function canonical(v: unknown): string {
  return JSON.stringify(v, (_k, x: unknown) =>
    x && typeof x === "object" && !Array.isArray(x)
      ? Object.fromEntries(Object.entries(x as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
      : x,
  );
}

/** A screen with every field that equals its default removed (the server omits them too),
 * so "set to the default" and "not set" compare equal. */
export function normScreen(s: Screen): Record<string, unknown> {
  const x: Record<string, unknown> = { ...(s as unknown as Record<string, unknown>) };
  delete x.revision;
  if (x.enabled === true) delete x.enabled;
  if (x.alpha === "opaque") delete x.alpha;
  if (!x.name) delete x.name;
  if (!x.sortKey) delete x.sortKey;
  if (s.frameRate && s.frameRate.num === 30 && s.frameRate.den === 1) delete x.frameRate;
  if (s.size && s.size.width === 1920 && s.size.height === 1080) delete x.size;
  if (s.kind?.type === "audience") delete x.kind;
  const b = s.background;
  if (b && !b.r && !b.g && !b.b && !(b.a ?? 1)) delete x.background;
  if (x.color === undefined) delete x.color;
  if (x.cornerPin === undefined) delete x.cornerPin;
  return x;
}

export function normOutput(o: OutputBinding): Record<string, unknown> {
  const x: Record<string, unknown> = { ...(o as unknown as Record<string, unknown>) };
  delete x.revision;
  if (x.enabled === true) delete x.enabled;
  if (!x.videoDelay) delete x.videoDelay;
  if (x.target === "full") delete x.target;
  if (x.embedAudio === false) delete x.embedAudio;
  if (!x.name) delete x.name;
  const t = o.transport;
  if (t?.type === "idn" && t.groups && !t.groups.length) x.transport = { type: t.type, sourceName: t.sourceName };
  return x;
}

export function screenDirty(d: Draft): boolean {
  return canonical(normScreen(d.screen)) !== canonical(normScreen(d.base));
}

export function outputDirty(o: DraftOutput): boolean {
  if (!o.base) return !o.deleted;
  if (o.deleted) return true;
  return canonical(normOutput(o.doc)) !== canonical(normOutput(o.base));
}

export function isDirty(d: Draft | null): boolean {
  return !!d && (screenDirty(d) || d.outputs.some(outputDirty));
}

export interface Plan {
  screen: boolean;
  deletes: DraftOutput[];
  creates: DraftOutput[];
  updates: DraftOutput[];
}

/** What an Apply sends: the screen (when changed), then deletes, creates and updates of
 * outputs (deletes first, so a freed source name can be reused by another output). */
export function planApply(d: Draft): Plan {
  return {
    screen: screenDirty(d),
    deletes: d.outputs.filter((o) => o.base && o.deleted),
    creates: d.outputs.filter((o) => !o.base && !o.deleted),
    updates: d.outputs.filter((o) => o.base && !o.deleted && outputDirty(o)),
  };
}

/** The display connector or network name an output holds on the server ("display:dvi-1",
 * "IDN:main"); the server refuses a second output holding the same. Null when it holds neither. */
function heldBy(o: OutputBinding): string | null {
  const t = o.transport;
  if (t?.type === "display") {
    const c = displayOf(o).toLowerCase();
    return c ? `display:${c}` : null;
  }
  if (t?.type === "idn") return `IDN:${t.sourceName.trim().toLowerCase()}`;
  if (t?.type === "omt") return `OMT:${t.name.trim().toLowerCase()}`;
  return null;
}

/**
 * The updates of an Apply that must give up their display or name before the others take it:
 * the server checks each request against what is stored, so a swap (A to DISPLAY2, B to
 * DISPLAY1) or a create taking a display an update moves away from is refused otherwise.
 */
export function mustPark(d: Draft): DraftOutput[] {
  const plan = planApply(d);
  const wanted = new Set([...plan.creates, ...plan.updates].map((o) => heldBy(o.doc)).filter((k): k is string => !!k));
  return plan.updates.filter((o) => {
    const held = heldBy(o.base!);
    return !!held && held !== heldBy(o.doc) && wanted.has(held);
  });
}

/** An output as it is stored, switched off and holding no display or name another output may
 * want: the first half of a swap (the second half sends its new values). */
export function parkedDoc(o: OutputEntry, all: OutputEntry[]): OutputBinding {
  const doc = { ...(stripRevision(o) as OutputBinding), enabled: false };
  const t = doc.transport;
  if (t?.type === "display") return { ...doc, transport: { ...t, selector: { connector: `parked-${o.id}` } } };
  if (isNamed(t?.type)) {
    const taken = all.filter((x) => x.id !== o.id).map((x) => sourceName(x));
    return setSourceName(doc, uniqueName(`${sourceName(doc)} (moving)`, taken));
  }
  return doc;
}

/**
 * The outputs of a duplicated screen: named ones (IDN, OMT) get a new unique name; a display
 * output takes another free display and starts switched off (it would cover a monitor), or is
 * left out when no display is free (the server refuses one display for two outputs).
 */
export function duplicateOutputs(
  all: OutputEntry[],
  from: string,
  to: string,
  displays: DisplayInfo[],
): { copies: OutputBinding[]; off: string[]; skipped: string[] } {
  const taken = all.filter((o) => isNamed(o.transport?.type)).map((o) => sourceName(o));
  const used = all.map((o) => displayOf(o)).filter(Boolean);
  const copies: OutputBinding[] = [];
  const off: string[] = [];
  const skipped: string[] = [];
  for (const o of outputsOf(all, from)) {
    let copy: OutputBinding = { ...(stripRevision(o) as OutputBinding), screen: to };
    if (isNamed(copy.transport?.type)) {
      const n = uniqueName(`${sourceName(copy)} copy`, taken);
      taken.push(n);
      copy = setSourceName(copy, n);
    } else if (copy.transport?.type === "display") {
      const d = freeDisplay(displays, used);
      const label = o.name || displayOf(o);
      if (!d) {
        skipped.push(label);
        continue;
      }
      used.push(d.id);
      copy = { ...setDisplay(copy, d), enabled: false };
      off.push(d.name || d.id);
    }
    copies.push(copy);
  }
  return { copies, off, skipped };
}

/**
 * The display outputs this Apply newly makes cover their monitor: full screen and running now,
 * but not before (a new output, another display, a window made full screen, or the output or
 * its screen switched back on).
 */
export function newlyCovering(d: Draft): DraftOutput[] {
  if (d.screen.enabled === false) return [];
  const screenWasOff = d.base.enabled === false;
  return d.outputs.filter((o) => {
    if (o.deleted || o.doc.transport?.type !== "display" || !displayFullscreen(o.doc) || o.doc.enabled === false) return false;
    if (!o.base || screenWasOff || o.base.enabled === false) return true;
    return !(displayOf(o.base) === displayOf(o.doc) && displayFullscreen(o.base));
  });
}

/** Body of a POST for a new output (no ID: the server assigns it). */
export function createBody(o: OutputBinding): Partial<OutputBinding> & Pick<OutputBinding, "screen"> {
  const { id: _id, schemaVersion: _v, ...rest } = o;
  return rest;
}

let tmp = 0;
export function newOutput(screen: Screen, taken: string[], kind = "idn"): DraftOutput {
  const name = uniqueName(screen.name?.trim() || "Output", taken);
  const doc: OutputBinding = {
    schemaVersion: 2,
    id: `new-${++tmp}`,
    name,
    screen: screen.id,
    transport: isIdn(kind) ? { type: "idn", sourceName: name } : { type: "omt", name },
  };
  return { key: doc.id, base: null, doc, deleted: false };
}

/** `base`, else `base 2`, `base 3`, … (case-insensitive against `taken`). */
export function uniqueName(base: string, taken: string[]): string {
  const t = new Set(taken.map((n) => n.trim().toLowerCase()));
  if (!t.has(base.toLowerCase())) return base;
  for (let i = 2; ; i++) {
    const n = `${base} ${i}`;
    if (!t.has(n.toLowerCase())) return n;
  }
}

/** Every IDN source name in use on the server, excluding the given output keys / IDs. */
export function idnNames(all: OutputEntry[], except: Set<string>): string[] {
  return all.filter((o) => !except.has(o.id) && isIdn(o.transport?.type)).map((o) => sourceName(o));
}

// ---- validation (the server validates again; these give instant feedback) ------------------

export type FieldErrors = Record<string, string>;

export function validateScreen(s: Screen): FieldErrors {
  const e: FieldErrors = {};
  const name = s.name?.trim() ?? "";
  if (!name) e.name = "Give the screen a name.";
  else if (name.length > LIMITS.maxName) e.name = `At most ${LIMITS.maxName} characters.`;
  const { width, height } = screenSize(s);
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < LIMITS.minSide || height < LIMITS.minSide)
    e.size = `Width and height must be at least ${LIMITS.minSide} px.`;
  else if (width > LIMITS.maxWidth || height > LIMITS.maxHeight) e.size = `At most ${LIMITS.maxWidth} x ${LIMITS.maxHeight}.`;
  else if (width % 2 || height % 2) e.size = "Width and height must be even numbers.";
  const arr = validateArrangement(s);
  if (arr) e.arrangement = arr;
  const r = screenRate(s);
  if (!(r.num > 0 && r.den > 0) || fps(r) < LIMITS.minFps || fps(r) > LIMITS.maxFps)
    e.frameRate = `Frame rate must be between ${LIMITS.minFps} and ${LIMITS.maxFps} fps.`;
  return e;
}

/** Per output key. `others`: stored outputs of every screen (for duplicate IDN names). */
export function validateOutputs(outputs: DraftOutput[], others: OutputEntry[]): Record<string, FieldErrors> {
  const out: Record<string, FieldErrors> = {};
  const live = outputs.filter((o) => !o.deleted);
  const mine = new Set(outputs.map((o) => o.base?.id ?? o.key));
  const elsewhere = new Set(idnNames(others, mine).map((n) => n.trim().toLowerCase()));
  const seen = new Map<string, string>();
  const usedDisplays = new Set(
    others
      .filter((x) => !mine.has(x.id))
      .map((x) => displayOf(x).toLowerCase())
      .filter(Boolean),
  );
  const seenDisplays = new Set<string>();
  for (const o of live) {
    const e: FieldErrors = {};
    const t = o.doc.transport;
    if (!t || t.type === "unknown") e.transport = "Choose where this output goes.";
    if (isNamed(t?.type)) {
      const n = sourceName(o.doc).trim();
      if (!n) e["transport.sourceName"] = "Give the output a name.";
      else if (isIdn(t?.type)) {
        const k = n.toLowerCase();
        if (elsewhere.has(k) || seen.has(k)) e["transport.sourceName"] = `Another output is already called "${n}".`;
        seen.set(k, o.key);
      }
    }
    if (t?.type === "display") {
      const c = displayOf(o.doc).toLowerCase();
      if (!c) e["transport.sourceName"] = "Choose a display.";
      else if (usedDisplays.has(c) || seenDisplays.has(c)) e["transport.sourceName"] = "Another output already uses this display.";
      if (c) seenDisplays.add(c);
    }
    const d = o.doc.videoDelay ?? 0;
    if (!Number.isInteger(d) || d < 0 || d > LIMITS.maxDelay) e.videoDelay = `0 to ${LIMITS.maxDelay} frames.`;
    if (Object.keys(e).length) out[o.key] = e;
  }
  return out;
}

/** Maps a server field path to the field the dialog shows it next to. */
export function fieldSlot(field: string | null | undefined): string {
  if (!field) return "";
  if (field.startsWith("size")) return "size";
  if (field.startsWith("arrangement")) return "arrangement";
  if (field === "transport.name") return "transport.sourceName";
  if (field.startsWith("transport.")) return "transport.sourceName";
  if (field === "name") return "name";
  return field;
}

// ---- arrangements (Single / Mirror / Grouped / Edge Blend) -----------------------------------

export type ArrangementKindValue = "single" | "mirror" | "grouped" | "edgeBlend";

export const ARRANGEMENTS: { kind: ArrangementKindValue; label: string; hint: string }[] = [
  { kind: "single", label: "Single", hint: "One output shows the whole screen." },
  { kind: "mirror", label: "Mirror", hint: "Every output shows the whole screen." },
  { kind: "grouped", label: "Grouped", hint: "The screen spans a grid of displays; each output shows its tile. The bezel is the part of the picture hidden behind the frames between two displays." },
  { kind: "edgeBlend", label: "Edge Blend", hint: "The screen spans overlapping projectors; each output shows its tile and fades the edges it shares with a neighbour, so the overlap is as bright as the rest." },
];

/** A screen's arrangement with every field filled in (defaults as the server's). */
export interface FullArrangement {
  kind: ArrangementKindValue;
  columns: number;
  rows: number;
  bezel: { x: number; y: number };
  overlap: { x: number; y: number };
  blend: { gamma: number; curve: number };
}

export function arrangementOf(s: Screen): FullArrangement {
  const a = s.arrangement ?? {};
  const kind = (["single", "mirror", "grouped", "edgeBlend"] as const).find((k) => k === a.kind) ?? "single";
  return {
    kind,
    columns: a.columns ?? 1,
    rows: a.rows ?? 1,
    bezel: { x: a.bezel?.x ?? 0, y: a.bezel?.y ?? 0 },
    overlap: { x: a.overlap?.x ?? 0, y: a.overlap?.y ?? 0 },
    blend: { gamma: a.blend?.gamma ?? 2.2, curve: a.blend?.curve ?? 2 },
  };
}

export function isTiled(a: FullArrangement): boolean {
  return a.kind === "grouped" || a.kind === "edgeBlend";
}

/** Gap between two tiles across and down: the bezel, or minus the overlap. */
function gapOf(a: FullArrangement): [number, number] {
  if (a.kind === "grouped") return [a.bezel.x, a.bezel.y];
  if (a.kind === "edgeBlend") return [-a.overlap.x, -a.overlap.y];
  return [0, 0];
}

/** One tile's size on a canvas (the inverse of `canvasForTile`). */
export function tileSizeOf(a: FullArrangement, size: { width: number; height: number }): { width: number; height: number } {
  const [gx, gy] = gapOf(a);
  return {
    width: (size.width - (a.columns - 1) * gx) / a.columns,
    height: (size.height - (a.rows - 1) * gy) / a.rows,
  };
}

/** The canvas (the screen's size) for tiles of `tile` px. */
export function canvasForTile(a: FullArrangement, tile: { width: number; height: number }): { width: number; height: number } {
  const [gx, gy] = gapOf(a);
  return {
    width: Math.round(a.columns * tile.width + (a.columns - 1) * gx),
    height: Math.round(a.rows * tile.height + (a.rows - 1) * gy),
  };
}

/** The stored form: only what differs from the defaults (Single is not stored at all). */
function storedArrangement(a: FullArrangement): Screen["arrangement"] {
  if (a.kind === "single") return undefined;
  const out: NonNullable<Screen["arrangement"]> = { kind: a.kind };
  if (isTiled(a)) {
    if (a.columns !== 1) out.columns = a.columns;
    if (a.rows !== 1) out.rows = a.rows;
    if (a.kind === "grouped" && (a.bezel.x || a.bezel.y)) out.bezel = { ...a.bezel };
    if (a.kind === "edgeBlend") {
      if (a.overlap.x || a.overlap.y) out.overlap = { ...a.overlap };
      if (a.blend.gamma !== 2.2 || a.blend.curve !== 2) out.blend = { ...a.blend };
    }
  }
  return out;
}

/**
 * The screen with its arrangement changed by `patch`, keeping the tile size (so adding a column
 * of 1920 × 1080 displays widens the canvas by one display): the screen's size is recomputed
 * from the tile size. Single / Mirror keep one tile, the screen's size.
 */
export function setArrangement(s: Screen, patch: Partial<FullArrangement>, tile?: { width: number; height: number }): Screen {
  const cur = arrangementOf(s);
  const size = screenSize(s);
  const curTile = isTiled(cur) ? tileSizeOf(cur, size) : { ...size };
  const next: FullArrangement = { ...cur, ...patch };
  if (!isTiled(next)) {
    next.columns = 1;
    next.rows = 1;
  }
  const t = tile ?? { width: Math.round(curTile.width), height: Math.round(curTile.height) };
  const out: Screen = { ...s, arrangement: storedArrangement(next) };
  if (out.arrangement === undefined) delete out.arrangement;
  if (isTiled(next)) out.size = canvasForTile(next, t);
  else if (isTiled(cur)) out.size = { width: t.width, height: t.height };
  return out;
}

export interface TileRect {
  index: number;
  /** Fractions of the canvas (0–1). */
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The tiles of a tiled arrangement as fractions of the canvas, row-major. */
export function tileRects(a: FullArrangement, size: { width: number; height: number }): TileRect[] {
  if (!isTiled(a)) return [];
  const t = tileSizeOf(a, size);
  const [gx, gy] = gapOf(a);
  const out: TileRect[] = [];
  for (let r = 0; r < a.rows; r++)
    for (let c = 0; c < a.columns; c++)
      out.push({
        index: r * a.columns + c,
        x: (c * (t.width + gx)) / size.width,
        y: (r * (t.height + gy)) / size.height,
        w: t.width / size.width,
        h: t.height / size.height,
      });
  return out;
}

/** "Tile 2 (row 1, column 2)". */
export function tileLabel(a: FullArrangement, index: number): string {
  const r = Math.floor(index / a.columns) + 1;
  const c = (index % a.columns) + 1;
  return a.rows > 1 ? `Tile ${index + 1} (row ${r}, column ${c})` : `Tile ${index + 1}`;
}

/** The output with its tile set (`null` = the whole screen). */
export function setTile(o: OutputBinding, tile: number | null): OutputBinding {
  const out: OutputBinding = { ...o };
  if (tile === null) delete out.tile;
  else out.tile = tile;
  return out;
}

/** The tiles' problems Oriel can see before Apply (the server checks the same). */
export function validateArrangement(s: Screen): string | null {
  const a = arrangementOf(s);
  if (!isTiled(a)) return null;
  if (a.columns < 1 || a.columns > 8 || a.rows < 1 || a.rows > 8) return "1 to 8 tiles across and down.";
  if (a.bezel.x < 0 || a.bezel.y < 0 || a.bezel.x > 2000 || a.bezel.y > 2000) return "The bezel is 0 to 2000 px.";
  if (a.overlap.x < 0 || a.overlap.y < 0) return "The overlap cannot be negative.";
  const t = tileSizeOf(a, screenSize(s));
  if (t.width < 16 || t.height < 16) return "The tiles don't fit the screen.";
  if (a.kind === "edgeBlend" && (a.overlap.x >= t.width || a.overlap.y >= t.height)) return "The overlap must be smaller than a tile.";
  // Three or more tiles in a line: over half a tile, three projectors overlap (F08-3).
  if (a.kind === "edgeBlend" && ((a.columns >= 3 && a.overlap.x > t.width / 2) || (a.rows >= 3 && a.overlap.y > t.height / 2)))
    return "With three or more tiles in a row or column, the overlap is at most half a tile.";
  if (a.blend.gamma < 1 || a.blend.gamma > 3) return "The blend gamma is 1 to 3.";
  if (a.blend.curve < 1 || a.blend.curve > 4) return "The blend curve is 1 to 4.";
  return null;
}
