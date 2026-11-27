<script lang="ts">
  // The Screens tab of the Screens & Looks page (plan 4.8): the Audience and Stage lists with
  // on / off switches, screen cards (EDITED, TEST), identify, test patterns, and the selected
  // screen's editor (name, colour, arrangement, role, a stage screen's mask, size, rate, alpha,
  // outputs with Hardware / Color / Corner Pin / Alpha Key tabs). Changes are staged and go live
  // on Apply (the page toolbar, through `screensState.editor`); conflicts and validation as before.
  import Switch from "../../ui/Switch.svelte";
  import Chip from "../../ui/Chip.svelte";
  import Plus from "lucide-svelte/icons/plus";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import type { DisplayInfo, MaskEntry, OutputBinding, OutputEntry, OutputState, OutputTransportInfo, ScreenEntry, ScreenState, TestPattern } from "$protocol";
  import { session } from "../../state/session.svelte";
  import { screensState } from "../../state/screens.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import { ApiRequestError } from "../../server";
  import { windowCentre } from "../../tauri";
  import ColorTab from "./ColorTab.svelte";
  import CornerPinTab from "./CornerPinTab.svelte";
  import ChunkError from "../../components/ChunkError.svelte";
  import { Dialog, Menu, Popover } from "../../ui";
  import { untrack } from "svelte";
  // The mask editor loads on first use (its own chunk, shared with the Looks tab).
  const maskEditorModule = () => import("./MaskEditor.svelte");
  import {
    ALPHA_MODES,
    LIMITS,
    PATTERNS,
    RATE_PRESETS,
    SIZE_PRESETS,
    alphaOverrideValue,
    colorToHex,
    createBody,
    displayFullscreen,
    displayLabel,
    displayModeOptions,
    displayRisk,
    displayModeValue,
    displayOf,
    duplicateOutputs,
    fieldSlot,
    freeDisplay,
    fpsLabel,
    hasScreenColor,
    hexToColor,
    isDirty,
    isNamed,
    isIdn,
    makeDraft,
    maskChoices,
    mustPark,
    modeOptions,
    newlyCovering,
    idnNames,
    newOutput,
    outputDirty,
    outputMode,
    outputsOf,
    parkedDoc,
    patternLabel,
    planApply,
    sameRate,
    screenAlpha,
    screenRate,
    screenRole,
    operatorScreens,
    screenSize,
    setAlphaOverride,
    serverIsLocal,
    setDisplay,
    setDisplayFullscreen,
    setDisplayMode,
    setOutputMode,
    setOutputTarget,
    setSourceName,
    setTransportKind,
    withRole,
    withScreenMask,
    sourceName,
    stripRevision,
    transportLabel,
    uniqueName,
    validateOutputs,
    validateScreen,
    OUTPUT_TARGETS,
    outputTarget,
    fillKeyHint,
    keyOutputFor,
    ARRANGEMENTS,
    arrangementOf,
    isTiled,
    setArrangement,
    setTile,
    tileLabel,
    tileRects,
    tileSizeOf,
    type DesktopPoint,
    type Draft,
    type DraftOutput,
    type FieldErrors,
  } from "../../state/screens";

  const live = useLive();
  const liveScreens = live.screens;
  const enabled = live.screensEnabled;
  const identify = live.identify;
  const liveDisplays = live.displays;

  // ---- data -------------------------------------------------------------------------------

  let entries = $state.raw<ScreenEntry[]>([]);
  let outputEntries = $state.raw<OutputEntry[]>([]);
  let transports = $state.raw<OutputTransportInfo[]>([]);
  let loadedDisplays = $state.raw<DisplayInfo[]>([]);
  /** The box's displays: live from the state (hot-plug), else as loaded. */
  let displays = $derived($liveDisplays.length ? $liveDisplays : loadedDisplays);
  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  let selectedId = $state<string | null>(screensState.previewScreen);
  let draft = $state<Draft | null>(null);

  interface Errors {
    screen: FieldErrors;
    outputs: Record<string, FieldErrors>;
    general: string | null;
  }
  const noErrors = (): Errors => ({ screen: {}, outputs: {}, general: null });
  let errors = $state<Errors>(noErrors());
  let conflict = $state<{ kind: "screen" | "output"; key: string; current: unknown } | null>(null);
  let busy = $state(false);
  let customSize = $state(false);

  interface Prompt {
    title: string;
    message: string;
    detail?: string[];
    ok: string;
    danger?: boolean;
    alt?: { label: string; run: () => void | Promise<void> };
    run: () => void | Promise<void>;
    /** Runs when the prompt is dismissed without its OK (Cancel, Esc, a click outside). */
    cancel?: () => void;
    testid: string;
  }
  let prompt = $state<Prompt | null>(null);
  /** Closes the prompt as cancelled. */
  function dismissPrompt() {
    const p = prompt;
    prompt = null;
    p?.cancel?.();
  }
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  let tpOpen = $state(false);
  let solidHex = $state("#2f7de1");
  let tpPattern = $state<(typeof PATTERNS)[number]["type"]>("colorBars");
  let tpScope = $state<"all" | "sel">("all");
  /** Each output card's tab. */
  let outTab = $state<Record<string, "hw" | "color" | "pin" | "alpha">>({});
  const OUT_TABS = [
    { id: "hw", label: "Hardware" },
    { id: "color", label: "Color" },
    { id: "pin", label: "Corner Pin" },
    { id: "alpha", label: "Alpha Key" },
  ] as const;

  let dirty = $derived(isDirty(draft));
  /** The server has per-screen colour correction and corner pin. */
  let adjustable = $derived(session.server?.capabilities?.includes("screenAdjust") ?? false);
  /** The server sends Fill/Key pairs (an output's target fill or key). */
  let fillKey = $derived(session.server?.capabilities?.includes("fillKey") ?? false);
  /** Arrangements, tiles, placeholders and identifying a free display (batch 12). */
  let layouts = $derived(session.server?.capabilities?.includes("screenLayouts") ?? false);

  /** A colour or corner pin change went live: the draft's base and those two fields follow
   * (other staged edits stay, and the next Apply sends the new revision). */
  function adjusted(r: ScreenEntry) {
    if (!draft || draft.base.id !== r.id) return;
    draft.base = r;
    draft.screen.color = r.color;
    draft.screen.cornerPin = r.cornerPin;
    if (r.color === undefined) delete draft.screen.color;
    if (r.cornerPin === undefined) delete draft.screen.cornerPin;
    entries = entries.map((e) => (e.id === r.id ? r : e));
  }
  let audience = $derived(entries.filter((e) => screenRole(e) === "audience"));
  let stage = $derived(entries.filter((e) => screenRole(e) === "stage"));
  let liveSel = $derived<ScreenState | null>(draft ? ($liveScreens.find((s) => s.id === draft!.screen.id) ?? null) : null);

  const FALLBACK_TRANSPORTS: OutputTransportInfo[] = [
    { kind: "idn", label: "IDN Open Protocol (compatible with NDI®)", supported: true, available: true, version: null, status: "", detail: null },
    { kind: "omt", label: "OMT", supported: true, available: false, version: null, status: "not built", detail: null },
    { kind: "display", label: "Display", supported: false, available: false, version: null, status: "not yet supported", detail: null },
    { kind: "sdi", label: "SDI", supported: false, available: false, version: null, status: "not yet supported", detail: null },
  ];
  let transportList = $derived(transports.length ? transports : FALLBACK_TRANSPORTS);

  function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  /** Loads overlap (each document event starts one, Apply ends with one): only the newest
   * one's answers count, so an older, slower one can't bring back what was there before. */
  let loadSeq = 0;
  /** A load that was asked to rebuild the draft was overtaken: the newest one rebuilds. */
  let rebuildWanted = false;

  async function load(rebuild: boolean) {
    const rest = session.conn?.rest;
    if (!rest) return;
    const seq = ++loadSeq;
    if (rebuild) rebuildWanted = true;
    try {
      const [s, o, t] = await Promise.all([
        rest.screenEntries(),
        rest.outputs(),
        rest.outputTransports().catch(() => [] as OutputTransportInfo[]),
      ]);
      if (seq !== loadSeq) return;
      // Older servers have no display list.
      void rest
        .displays()
        .then((d) => seq === loadSeq && (loadedDisplays = d))
        .catch(() => seq === loadSeq && (loadedDisplays = []));
      rebuild = rebuildWanted;
      rebuildWanted = false;
      // Signage channels' screens belong to their channels (Site ▸ Signage), H02-7.
      entries = operatorScreens(s);
      outputEntries = o;
      transports = t;
      screensState.setScreens(entries);
      loaded = true;
      loadError = null;
      if (!selectedId || !entries.some((e) => e.id === selectedId)) selectedId = entries[0]?.id ?? null;
      if (rebuild || !isDirty(draft) || !draft || draft.screen.id !== selectedId) rebuildDraft();
    } catch (e) {
      if (seq === loadSeq) loadError = `Could not load the screen configuration: ${errorText(e)}`;
    }
  }

  function rebuildDraft() {
    const e = entries.find((x) => x.id === selectedId);
    draft = e ? makeDraft(e, outputEntries) : null;
    errors = noErrors();
    conflict = null;
    maskOpen = false;
    const { width, height } = e ? screenSize(e) : { width: 1920, height: 1080 };
    customSize = !SIZE_PRESETS.some((p) => p.width === width && p.height === height);
  }

  $effect(() => {
    const conn = session.conn;
    if (!conn) return;
    void load(true);
    return conn.onDocumentChanged((kind) => {
      if (kind === "screen" || kind === "outputBinding" || kind === "look") void load(false);
    });
  });

  // ---- guarding unsaved changes -----------------------------------------------------------

  /** The changes are settled (applied or discarded): leave the page. */
  function close() {
    shell.resumeAfterGuard();
  }

  /** Runs `action` now, or after the operator decided what to do with unsaved changes. */
  function guarded(action: () => void | Promise<void>) {
    if (!dirty || !draft) {
      void action();
      return;
    }
    prompt = {
      title: "Unsaved changes",
      message: `"${draft.screen.name || "This screen"}" has changes that are not applied yet.`,
      ok: "Discard changes",
      danger: true,
      alt: {
        label: "Apply",
        run: async () => {
          if (await apply()) await action();
        },
      },
      run: async () => {
        rebuildDraft();
        await action();
      },
      testid: "unsaved-prompt",
    };
  }

  /** Esc, the close button, menus: `true` = may close now. */
  function mayClose(): boolean {
    if (prompt) {
      dismissPrompt();
      return false;
    }
    if (menu || tpOpen) {
      menu = null;
      tpOpen = false;
      return false;
    }
    if (!dirty) return true;
    guarded(close);
    return false;
  }

  /** Esc: closes this page's own prompt or menu (`true` when it closed one); never the page. */
  function escape(): boolean {
    if (prompt) {
      dismissPrompt();
      return true;
    }
    if (menu || tpOpen) {
      menu = null;
      tpOpen = false;
      return true;
    }
    return false;
  }

  /** Esc wherever the focus is (the shell's Esc only closes its own dialogs). */
  function onWindowKey(e: KeyboardEvent) {
    if (e.key !== "Escape" || e.defaultPrevented || shell.dialogOpen) return;
    if (escape()) e.preventDefault();
  }

  $effect(() => {
    screensState.guard = mayClose;
    screensState.escape = escape;
    return () => {
      if (screensState.guard === mayClose) screensState.guard = null;
      if (screensState.escape === escape) screensState.escape = null;
    };
  });

  // The page toolbar's Apply / Revert and unsaved-changes line read this.
  $effect(() => {
    const api = {
      get dirty() {
        return dirty;
      },
      get busy() {
        return busy;
      },
      apply,
      revert,
    };
    screensState.editor = api;
    return () => {
      if (screensState.editor === api) screensState.editor = null;
    };
  });

  async function runPrompt(fn: () => void | Promise<void>) {
    prompt = null;
    try {
      await fn();
    } catch (e) {
      errors.general = errorText(e);
    }
  }

  // ---- list actions ---------------------------------------------------------------------------

  function pick(id: string) {
    if (id === selectedId) return;
    guarded(() => {
      selectedId = id;
      rebuildDraft();
    });
  }

  function addScreen(role: "audience" | "stage") {
    guarded(async () => {
      const rest = session.conn?.rest;
      if (!rest) return;
      try {
        const r = await rest.createScreen({ kind: { type: role } });
        selectedId = r.id;
        await load(true);
      } catch (e) {
        errors.general = `Could not add a screen: ${errorText(e)}`;
      }
    });
  }

  function openMenu(id: string, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const t = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { id, x: e.type === "contextmenu" ? e.clientX : t.right - 4, y: e.type === "contextmenu" ? e.clientY : t.bottom };
  }

  function duplicate(id: string) {
    menu = null;
    guarded(async () => {
      const rest = session.conn?.rest;
      const src = entries.find((e) => e.id === id);
      if (!rest || !src) return;
      let made: ScreenEntry | null = null;
      try {
        const { id: _id, revision: _r, sortKey: _k, import: _i, ...doc } = src;
        const r = await rest.createScreen({ ...doc, name: uniqueName(`${src.name || "Screen"} copy`, entries.map((e) => e.name ?? "")) });
        made = r;
        const { copies, off, skipped } = duplicateOutputs(outputEntries, id, r.id, displays);
        for (const copy of copies) await rest.createOutput(createBody(copy));
        selectedId = r.id;
        await load(true);
        const notes = [
          ...(off.length ? [`its display output${off.length === 1 ? " is" : "s are"} on ${off.join(", ")}, switched off`] : []),
          ...(skipped.length ? [`no free display for ${skipped.join(", ")}`] : []),
        ];
        if (notes.length) session.conn?.pushNotice("info", `"${r.name}": ${notes.join("; ")}.`);
      } catch (e) {
        // No half-made copy (its outputs, some already sending, go with it).
        if (made) await rest.deleteScreen(made.id, made.revision).catch(() => {});
        await load(false);
        errors.general = `Could not duplicate the screen: ${errorText(e)}`;
      }
    });
  }

  function askDelete(id: string) {
    menu = null;
    const src = entries.find((e) => e.id === id);
    if (!src) return;
    const outs = outputsOf(outputEntries, id);
    prompt = {
      title: "Delete screen",
      message: `Delete "${src.name || "Screen"}"? ${outs.length ? `${outs.length === 1 ? "This output is" : `These ${outs.length} outputs are`} deleted with it, and looks forget it.` : "It has no outputs; looks forget it."} This cannot be undone.`,
      detail: outs.map((o) => `${transportLabel(o.transport?.type)}: ${sourceName(o) || "(no name)"}`),
      ok: "Delete",
      danger: true,
      run: async () => {
        const rest = session.conn?.rest;
        if (!rest) return;
        try {
          await rest.deleteScreen(id, src.revision);
          if (selectedId === id) selectedId = null;
          await load(true);
        } catch (e) {
          if (e instanceof ApiRequestError && e.status === 409) {
            errors.general = "Changed by another operator since you opened it; check it and delete again.";
            await load(false);
          } else errors.general = `Could not delete the screen: ${errorText(e)}`;
        }
      },
      testid: "delete-prompt",
    };
  }

  // ---- screen editors ----------------------------------------------------------------------

  function setSize(width: number, height: number) {
    if (!draft) return;
    draft.screen.size = { width, height };
  }

  function onSizePreset(v: string) {
    if (v === "custom") {
      customSize = true;
      return;
    }
    customSize = false;
    const [w, h] = v.split("x").map(Number);
    if (w && h) setSize(w, h);
  }

  function onRate(v: string) {
    if (!draft) return;
    const [num, den] = v.split("/").map(Number);
    if (num && den) draft.screen.frameRate = { num, den };
  }

  // A stage screen's own mask (looks don't route stage screens; docs/LOOKS-MASKS-PLAN.md §9):
  // the site's masks for the picker, loaded once a stage screen is shown and again when another
  // operator (or the mask editor) changes them.
  let masks = $state.raw<MaskEntry[] | null>(null);
  let maskOpen = $state(false);
  /** The mask editor ("Edit masks…"): open, with the mask to show first. */
  let maskEditor = $state<{ initial: string | null } | null>(null);
  let masksSeq = 0;
  function loadMasks() {
    const rest = session.conn?.rest;
    if (!rest) return;
    const seq = ++masksSeq;
    void rest
      .masks()
      .then((ms) => seq === masksSeq && (masks = ms))
      .catch(() => {});
  }
  let isStage = $derived(!!draft && screenRole(draft.screen) === "stage");
  $effect(() => {
    if (isStage && !masks) untrack(loadMasks);
  });
  $effect(() => {
    const conn = session.conn;
    if (!conn) return;
    return conn.onDocumentChanged((kind) => {
      if (kind === "mask" && masks) loadMasks();
    });
  });

  function pickMask(id: string) {
    maskOpen = false;
    if (!draft) return;
    if (id === "edit") {
      maskEditor = { initial: draft.screen.mask ?? null };
      return;
    }
    draft.screen = withScreenMask(draft.screen, id);
    const { mask: _m, ...rest } = errors.screen;
    errors.screen = rest;
  }

  function onRole(v: string) {
    if (!draft) return;
    draft.screen = withRole(draft.screen, draft.base, v);
  }

  function onColorToggle(on: boolean) {
    if (!draft) return;
    const c = hexToColor(colorToHex(draft.screen.background), on ? 1 : 0);
    if (!on && !draft.base.background) delete draft.screen.background;
    else draft.screen.background = on ? { ...c, a: 1 } : { ...(draft.screen.background ?? c), a: 0 };
  }

  function onColor(hex: string) {
    if (!draft) return;
    draft.screen.background = hexToColor(hex, 1);
  }

  // ---- outputs ---------------------------------------------------------------------------------

  function addOutput() {
    if (!draft) return;
    const mine = new Set(draft.outputs.map((o) => o.base?.id ?? o.key));
    const taken = [...idnNames(outputEntries, mine), ...draft.outputs.filter((o) => !o.deleted).map((o) => sourceName(o.doc))];
    // A second output makes a Single screen a Mirror (back is the operator's choice).
    if (layouts && arrangementOf(draft.screen).kind === "single" && draft.outputs.some((o) => !o.deleted)) arrange({ kind: "mirror" });
    draft.outputs.push(newOutput(draft.screen, taken, "idn"));
  }

  /** Adds the Key output that pairs the Fill output `fill` (next free display, same delivery). */
  function addKeyOutput(fill: DraftOutput) {
    if (!draft) return;
    const mine = new Set(draft.outputs.map((o) => o.base?.id ?? o.key));
    const taken = [...idnNames(outputEntries, mine), ...draft.outputs.filter((o) => !o.deleted).map((o) => sourceName(o.doc))];
    const k = keyOutputFor(fill.doc, taken, displays, [...displaysInUse(fill), displayOf(fill.doc)].filter(Boolean));
    draft.outputs.push(k);
    outTab[k.key] = "hw";
  }

  /** The screen's draft has a Key output (so a Fill doesn't offer to add one). */
  function hasKeyOutput(): boolean {
    return !!draft?.outputs.some((o) => !o.deleted && outputTarget(o.doc) === "key");
  }

  function removeOutput(o: DraftOutput) {
    if (!draft) return;
    if (!o.base) draft.outputs = draft.outputs.filter((x) => x.key !== o.key);
    else o.deleted = true;
    delete errors.outputs[o.key];
  }

  function outputStatus(o: DraftOutput): { text: string; title: string; cls: string } {
    if (!o.base) return { text: "New: not applied yet", title: "Press Apply to create this output.", cls: "pending" };
    if (!draft) return { text: "", title: "", cls: "" };
    const ls = liveSel;
    const pending = outputDirty(o) ? " (changes not applied)" : "";
    if (!ls) {
      const why = draft.base.enabled === false ? "Screen disabled" : "Screen not running";
      return { text: why + pending, title: "The screen is not rendering, so its outputs are stopped.", cls: "off" };
    }
    const st: OutputState | undefined = ls.outputs.find((x) => x.binding === o.base!.id);
    if (!st) {
      const off = o.base.enabled === false;
      return { text: (off ? "Disabled" : "Not running") + pending, title: off ? "This output is switched off." : "The server has not started this output.", cls: "off" };
    }
    const counts = `${st.framesSent.toLocaleString()} sent · ${st.framesDropped.toLocaleString()} dropped · ${st.errors.toLocaleString()} errors`;
    const mode = `${st.width} x ${st.height} @ ${fpsLabel(st.frameRate)}${st.delayFrames ? `, ${st.delayFrames} frames delay` : ""}`;
    if (st.kind === "placeholder") return { text: `Placeholder: no hardware yet${pending}`, title: `${st.name}: kept in the configuration, sends nothing until its hardware is chosen.`, cls: "placeholder" };
    const verb = st.kind === "display" ? "Showing" : "Sending";
    if (st.status === "ok") return { text: `${verb} ${mode} · ${counts}${pending}`, title: `${st.name}: ${verb.toLowerCase()} ${mode}. ${counts}.`, cls: "ok" };
    const detail = st.detail ?? "";
    const short = detail.split(/[.\n]/)[0]!.slice(0, 70);
    return {
      text: `${st.status === "error" ? "Error" : "Unavailable"}${short ? `: ${short}` : ""}${pending}`,
      title: `${st.name}: ${detail || st.status}. ${counts}.`,
      cls: st.status,
    };
  }

  // ---- arrangement ------------------------------------------------------------------------------

  function arrange(patch: Parameters<typeof setArrangement>[1], tile?: { width: number; height: number }) {
    if (!draft) return;
    draft.screen = setArrangement(draft.screen, patch, tile);
    const after = arrangementOf(draft.screen);
    const cells = isTiled(after) ? after.columns * after.rows : 0;
    // Outputs keep their tile while it exists; a screen that isn't tiled has none.
    for (const o of draft.outputs) {
      const t = o.doc.tile;
      if (t != null && t >= cells) o.doc = setTile(o.doc, null);
    }
  }

  /** The outputs of the draft that show `tile`. */
  function tileOutputs(tile: number): DraftOutput[] {
    return (draft?.outputs ?? []).filter((o) => !o.deleted && o.doc.tile === tile);
  }

  /** Adds a placeholder output for an empty tile (its hardware can be chosen later). */
  function addPlaceholder(tile: number) {
    if (!draft) return;
    const mine = new Set(draft.outputs.map((o) => o.base?.id ?? o.key));
    const taken = [...idnNames(outputEntries, mine), ...draft.outputs.filter((o) => !o.deleted).map((o) => sourceName(o.doc))];
    const o = newOutput(draft.screen, taken, "idn");
    const name = uniqueName(`${draft.screen.name?.trim() || "Screen"} ${tile + 1}`, taken);
    o.doc = setTile({ ...setTransportKind({ ...o.doc, name }, "placeholder"), name }, tile);
    draft.outputs.push(o);
    outTab[o.key] = "hw";
  }

  let identifying = $state<string | null>(null);
  /** Shows a free display's name on it for a few seconds. */
  async function identifyDisplay(id: string) {
    identifying = id;
    try {
      await session.conn?.rest.identifyDisplay(id);
    } catch (e) {
      errors.general = errorText(e);
    } finally {
      setTimeout(() => {
        if (identifying === id) identifying = null;
      }, 5000);
    }
  }

  /** Switches an output's transport; a new display output takes the first free display. */
  function changeTransport(o: DraftOutput, kind: string) {
    let doc = setTransportKind(o.doc, kind);
    if (kind === "display") {
      const d = freeDisplay(displays, displaysInUse(o));
      if (d) doc = setDisplay(doc, d);
    }
    o.doc = doc;
  }

  /** Connectors other outputs (stored or staged) use. */
  function displaysInUse(self: DraftOutput): string[] {
    const mine = new Set((draft?.outputs ?? []).map((x) => x.base?.id ?? x.key));
    return [
      ...outputEntries.filter((x) => !mine.has(x.id)).map((x) => displayOf(x)),
      ...(draft?.outputs ?? []).filter((x) => x.key !== self.key && !x.deleted).map((x) => displayOf(x.doc)),
    ].filter(Boolean);
  }

  /** Which other output uses a display ("Stage Monitor"), if any. */
  function displayUser(d: DisplayInfo, self: DraftOutput): string | null {
    const mine = (draft?.outputs ?? []).find((x) => x.key !== self.key && !x.deleted && displayOf(x.doc).toLowerCase() === d.id.toLowerCase());
    if (mine) return sourceName(mine.doc) || "another output";
    const selfId = self.base?.id;
    const other = outputEntries.find((x) => x.id !== selfId && d.boundTo.includes(x.id) && !(draft?.outputs ?? []).some((y) => y.base?.id === x.id));
    if (!other) return null;
    return entries.find((e) => e.id === other.screen)?.name ?? "another screen";
  }

  function transportTitle(kind: string | undefined): string {
    const t = transportList.find((x) => x.kind === kind);
    if (!t) return "Choose where this output goes";
    return [t.status, t.detail].filter(Boolean).join(" — ");
  }

  function outErr(o: DraftOutput, slot: string): string | undefined {
    return errors.outputs[o.key]?.[slot];
  }

  // ---- apply -----------------------------------------------------------------------------------

  const snap = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

  function placeError(e: unknown, step: { kind: "screen" | "output"; key: string }) {
    if (e instanceof ApiRequestError && e.status === 409) {
      conflict = { kind: step.kind, key: step.key, current: e.current ?? null };
      return;
    }
    if (e instanceof ApiRequestError && e.status === 400 && e.field) {
      if (step.kind === "screen") {
        const slot = fieldSlot(e.field);
        errors.screen = { ...errors.screen, [slot]: e.message };
      } else {
        const slot = e.field === "name" || e.field.startsWith("transport") ? "transport.sourceName" : fieldSlot(e.field);
        errors.outputs = { ...errors.outputs, [step.key]: { ...(errors.outputs[step.key] ?? {}), [slot]: e.message } };
      }
      return;
    }
    errors.general = errorText(e);
  }

  /** The middle of Oriel's window on the desktop (physical px), when Oriel runs on the
   * server's computer: from the desktop shell's window (physical px); "unknown" where that
   * can't be had (a browser's coordinates aren't physical with mixed scaling; Wayland has
   * none); null when Oriel is on another computer (it can't be on the server's monitors). */
  async function orielPoint(): Promise<DesktopPoint | "unknown" | null> {
    if (!serverIsLocal(session.baseUrl || window.location.href)) return null;
    return (await windowCentre()) ?? "unknown";
  }
  /** Where Oriel is, for the full screen note (refreshed while the page is open). */
  let here = $state<DesktopPoint | "unknown" | null>(null);
  $effect(() => {
    const refresh = () => void orielPoint().then((p) => (here = p));
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  });

  /** Full screen display outputs this Apply newly puts on the operator's own monitor (the one
   * Oriel is on, or the main one): "Main Projector → Display 1 (\\.\DISPLAY1) (the main monitor)". */
  async function riskyOutputs(d: Draft): Promise<string[]> {
    const covering = newlyCovering(d);
    if (!covering.length) return [];
    here = await orielPoint();
    const out: string[] = [];
    for (const o of covering) {
      const conn = displayOf(o.doc);
      const dsp = displays.find((x) => x.id.toLowerCase() === conn.toLowerCase());
      const why = displayRisk(dsp, here);
      if (dsp && why) out.push(`${d.screen.name || "This screen"} → ${displayLabel(dsp).split(" · ")[0]} (${why})`);
    }
    return out;
  }

  /** Asks before covering the operator's monitor; true = go ahead. */
  function confirmRisky(items: string[]): Promise<boolean> {
    return new Promise((resolve) => {
      prompt = {
        title: "Cover your own monitor?",
        message:
          "This output fills a monitor you are working on. It covers everything on it, Oriel included, until the output is changed or removed (it never takes the keyboard or mouse). Choose Show: In a window to try it out first.",
        detail: items,
        ok: "Show it full screen",
        danger: true,
        run: () => resolve(true),
        cancel: () => resolve(false),
        testid: "risky-display-prompt",
      };
    });
  }

  async function apply(): Promise<boolean> {
    const d = draft;
    const rest = session.conn?.rest;
    if (!d || !rest || busy) return false;
    const se = validateScreen(d.screen);
    const oe = validateOutputs(d.outputs, outputEntries);
    errors = { screen: se, outputs: oe, general: null };
    if (Object.keys(se).length || Object.keys(oe).length) {
      errors.general = "Fix the highlighted fields first.";
      return false;
    }
    const risky = await riskyOutputs(d);
    if (risky.length && !(await confirmRisky(risky))) return false;
    busy = true;
    conflict = null;
    const plan = planApply(d);
    const park = mustPark(d);
    /** Parked outputs as they were stored, until their new values are in. */
    const parked = new Map<string, OutputEntry>();
    let step: { kind: "screen" | "output"; key: string } = { kind: "screen", key: d.screen.id };
    try {
      if (plan.screen) {
        const r = await rest.updateScreen(snap(d.screen), d.base.revision);
        d.base = r;
        d.screen = snap(stripRevision(r));
      }
      for (const o of plan.deletes) {
        step = { kind: "output", key: o.key };
        await rest.deleteOutput(o.base!.id, o.base!.revision);
        d.outputs = d.outputs.filter((x) => x.key !== o.key);
      }
      // Outputs giving up a display or name another one takes (a swap): switched off and freed
      // first, as the server checks each request against what it has.
      for (const o of park) {
        step = { kind: "output", key: o.key };
        const was = o.base!;
        const r = await rest.updateOutput(parkedDoc(was, outputEntries), was.revision);
        parked.set(o.key, was);
        o.base = r;
      }
      for (const o of plan.creates) {
        step = { kind: "output", key: o.key };
        const r = await rest.createOutput(createBody(snap(o.doc)));
        d.outputs = d.outputs.map((x) => (x.key === o.key ? { key: r.id, base: r, doc: snap(stripRevision(r)), deleted: false } : x));
      }
      for (const o of plan.updates) {
        step = { kind: "output", key: o.key };
        const r = await rest.updateOutput(snap(o.doc), o.base!.revision);
        parked.delete(o.key);
        d.outputs = d.outputs.map((x) => (x.key === o.key ? { key: r.id, base: r, doc: snap(stripRevision(r)), deleted: false } : x));
      }
      busy = false;
      await load(true);
      return true;
    } catch (e) {
      placeError(e, step);
      // An output left parked (switched off, its display or name given up) goes back to what it
      // was, where that is still free; the draft keeps the operator's values to try again.
      for (const [key, was] of parked) {
        const o = d.outputs.find((x) => x.key === key);
        if (!o?.base) continue;
        await rest
          .updateOutput(stripRevision(was) as OutputBinding, o.base.revision)
          .then((r) => (o.base = r))
          .catch(() => {});
      }
      busy = false;
      await load(false);
      return false;
    } finally {
      busy = false;
    }
  }

  function revert() {
    rebuildDraft();
  }

  async function reloadAfterConflict() {
    conflict = null;
    await load(true);
  }

  async function overwrite() {
    const c = conflict;
    const d = draft;
    if (!c || !d) return;
    conflict = null;
    if (c.kind === "screen") {
      if (!c.current) {
        errors.general = "Another operator deleted this screen; there is nothing to overwrite.";
        await load(true);
        return;
      }
      d.base = c.current as ScreenEntry;
    } else {
      const o = d.outputs.find((x) => x.key === c.key);
      if (o) o.base = c.current ? (c.current as OutputEntry) : null;
    }
    await apply();
  }

  // ---- show commands ---------------------------------------------------------------------------

  function toggleIdentify() {
    void session.run({ type: "identifyScreens", enabled: !$identify });
  }

  /** The running screens a test pattern goes to: every one, or the selected one. */
  let tpTargets = $derived(tpScope === "all" ? $liveScreens : liveSel ? [liveSel] : []);
  let tpOn = $derived(tpTargets.some((s) => !!s.testPattern));

  function showPattern() {
    const p: TestPattern = tpPattern === "solid" ? { type: "solid", color: hexToColor(solidHex) } : ({ type: tpPattern } as TestPattern);
    for (const s of tpTargets) void session.run({ type: "setTestPattern", screen: s.id, pattern: p });
  }

  function patternsOff() {
    for (const s of tpTargets) if (s.testPattern) void session.run({ type: "setTestPattern", screen: s.id, pattern: null });
  }

  // ---- labels ----------------------------------------------------------------------------------

  function sizeKey(w: number, h: number) {
    return `${w}x${h}`;
  }

  function listLabel(e: ScreenEntry): { name: string; size: string } {
    const s = draft && draft.screen.id === e.id ? draft.screen : e;
    const { width, height } = screenSize(s);
    return { name: s.name || "(unnamed)", size: `${width} x ${height} · ${fpsLabel(screenRate(s))} fps${s.enabled === false ? " · disabled" : ""}` };
  }
</script>

<svelte:window onkeydown={onWindowKey} />

{#snippet group(title: string, role: "audience" | "stage", list: ScreenEntry[])}
  <div class="ghead">
    <Switch
      checked={$enabled[role]}
      label={`${title} screens on`}
      data-testid={`role-switch-${role}`}
      onchange={() => session.toggleScreens(role)}
      title={`${title} screens ${$enabled[role] ? "on" : "off"} (${role === "audience" ? "Ctrl+1" : "Ctrl+2"})`}
    />
    <span class="gtitle">{title}</span>
    <button class="add" data-testid={`add-${role}`} onclick={() => addScreen(role)} title={`Add a ${title.toLowerCase()} screen (1920 x 1080, 30 fps, opaque)`} aria-label={`Add ${title} screen`}><Plus size={20} /></button>
  </div>
  <ul class="slist">
    {#each list as s (s.id)}
      {@const l = listLabel(s)}
      <li class="srow" class:sel={selectedId === s.id}>
        <button class="sitem" data-testid="screen-item" data-screen-id={s.id} onclick={() => pick(s.id)} oncontextmenu={(e) => openMenu(s.id, e)}>
          <span class="sname">{l.name}</span>
          <span class="ssize">{l.size}</span>
          {#if (draft?.screen.id === s.id && dirty) || $liveScreens.find((x) => x.id === s.id)?.testPattern}
            <span class="chips">
              {#if draft?.screen.id === s.id && dirty}<span title="Not applied yet" data-testid="edited-chip"><Chip kind="edited">EDITED</Chip></span>{/if}
              {#if $liveScreens.find((x) => x.id === s.id)?.testPattern}<span title={`Test pattern: ${patternLabel($liveScreens.find((x) => x.id === s.id)?.testPattern)}`}><Chip kind="test">TEST</Chip></span>{/if}
            </span>
          {/if}
        </button>
        <button class="more" data-testid="item-menu-btn" aria-label={`More for ${l.name}`} title="Duplicate or delete" onclick={(e) => openMenu(s.id, e)}><Ellipsis size={16} /></button>
      </li>
    {:else}
      <li class="none">{loaded ? `No ${title.toLowerCase()} screens` : "Loading…"}</li>
    {/each}
  </ul>
{/snippet}

<div class="page">
  <div class="cfg">
    <div class="left">
      <!-- Space follows the number of screens in each group (a long audience list is not
           squeezed by an empty stage group). -->
      <div class="half" style:flex-grow={Math.max(1, audience.length)}>{@render group("Audience", "audience", audience)}</div>
      <div class="half" style:flex-grow={Math.max(1, stage.length)}>{@render group("Stage", "stage", stage)}</div>
      <div class="identify">
        <span>Identify Screens</span>
        <Switch
          checked={$identify}
          label="Identify screens"
          data-testid="identify-switch"
          title="Show each screen's name, role and outputs on its outputs (turns off after 60 s)"
          onchange={toggleIdentify}
        />
      </div>
      <div class="tp">
        <button
          data-testid="tp-button"
          class:active={$liveScreens.some((x) => x.testPattern)}
          disabled={!$liveScreens.length}
          aria-expanded={tpOpen}
          title={$liveScreens.length ? "Show a test pattern on the screens" : "No screen is running"}
          onclick={() => (tpOpen = !tpOpen)}
        >{tpOpen ? "Hide Test Patterns" : "Test Patterns"}{$liveScreens.some((x) => x.testPattern) ? " · on" : ""}</button>
        {#if tpOpen}
          <div class="tpanel" data-testid="tp-picker">
            <label class="tfld">Pattern
              <select bind:value={tpPattern} data-testid="tp-pattern">
                {#each PATTERNS as p (p.type)}<option value={p.type} title={p.hint}>{p.label}</option>{/each}
              </select>
            </label>
            {#if tpPattern === "solid"}
              <label class="tfld row">Colour <input type="color" bind:value={solidHex} aria-label="Solid colour" /></label>
            {/if}
            <label class="tfld">Show on
              <select bind:value={tpScope} data-testid="tp-scope">
                <option value="all">Every screen</option>
                <option value="sel" disabled={!liveSel}>This screen{liveSel ? ` (${liveSel.name})` : " (not running)"}</option>
              </select>
            </label>
            {#if tpOn}
              <button class="tpgo off" data-testid="tp-off" onclick={patternsOff}>Turn off test pattern</button>
            {:else}
              <button class="tpgo" data-testid="tp-show" disabled={!tpTargets.length} onclick={showPattern}>Show test pattern</button>
            {/if}
          </div>
        {/if}
      </div>
    </div>

    <div class="main">
      <div class="scroll">
        {#if conflict}
          <div class="banner conflict" data-testid="conflict-banner" role="alert">
            <TriangleAlert size={16} />
            <span>Changed by another operator. {conflict.kind === "screen" ? "This screen" : "An output"} was {conflict.current ? "edited" : "deleted"} since you opened it.</span>
            <button onclick={reloadAfterConflict} title="Discard your changes and load theirs">Reload</button>
            <button onclick={overwrite} title="Replace their version with yours">Overwrite</button>
          </div>
        {/if}
        {#if loadError}<div class="banner err" role="alert">{loadError}</div>{/if}
        {#if errors.general}<div class="banner err" data-testid="general-error" role="alert">{errors.general}</div>{/if}

        {#if draft}
          {@const sz = screenSize(draft.screen)}
          {@const rate = screenRate(draft.screen)}
          {@const alpha = screenAlpha(draft.screen)}
          {@const hasColor = hasScreenColor(draft.screen)}
          <div class="mhead">
            <div class="namebox">
              <input
                class="mtitle"
                class:bad={!!errors.screen.name}
                data-testid="screen-name"
                aria-label="Screen name"
                maxlength={LIMITS.maxName}
                value={draft.screen.name ?? ""}
                oninput={(e) => (draft!.screen.name = e.currentTarget.value)}
              />
              {#if errors.screen.name}<div class="ferr">{errors.screen.name}</div>{/if}
              <div class="msize">{sz.width} x {sz.height} · {fpsLabel(rate)} fps · {ALPHA_MODES.find((a) => a.value === alpha)?.label}{draft.screen.enabled === false ? " · disabled" : ""}</div>
            </div>
            <label class="color" title={hasColor ? "Colour drawn behind every layer" : "Transparent behind every layer (opaque screens show black)"}>
              <input type="checkbox" data-testid="screen-color-toggle" checked={hasColor} onchange={(e) => onColorToggle(e.currentTarget.checked)} /> Screen Color
              <input
                type="color"
                class="swatch"
                class:off={!hasColor}
                data-testid="screen-color"
                value={colorToHex(draft.screen.background)}
                disabled={!hasColor}
                title={hasColor ? "Pick the screen colour" : "Tick Screen Color to choose a colour"}
                oninput={(e) => onColor(e.currentTarget.value)}
              />
            </label>
          </div>

          <div class="tools">
            {#if !layouts}
            <label class="fld"><span>Arrangement</span>
              <select value="single" title="Mirror, Grouped and Edge Blend need a newer server: this one sends each screen to its outputs as one picture">
                <option value="single">Single</option>
                <option disabled>Mirror (not available yet)</option>
                <option disabled>Grouped (not available yet)</option>
                <option disabled>Edge Blend (not available yet)</option>
              </select>
            </label>
            {/if}
            <label class="fld"><span>Role</span>
              <select data-testid="screen-role" value={screenRole(draft.screen)} onchange={(e) => onRole(e.currentTarget.value)}>
                <option value="audience">Audience</option>
                <option value="stage">Stage</option>
              </select>
            </label>
            {#if isStage}
              {@const mc = maskChoices(masks, draft.screen.mask)}
              <div class="fld anchor" class:bad={!!errors.screen.mask || mc.missing}>
                <span>Mask</span>
                <button
                  class="pick"
                  class:set={!!draft.screen.mask}
                  data-testid="screen-mask"
                  aria-label={`Mask: ${mc.label}`}
                  aria-haspopup="menu"
                  aria-expanded={maskOpen}
                  onclick={() => (maskOpen = !maskOpen)}
                  title={mc.missing ? "This mask was deleted: choose another or None" : "Drawn over this stage screen's layout"}
                  ><span class="plabel">{mc.label}</span><ChevronDown size={14} /></button
                >
                {#if maskOpen}
                  <Popover role="none" width={220} onclose={() => (maskOpen = false)}>
                    <Menu label={`Mask on ${draft.screen.name || "this screen"}`} items={mc.items} onselect={pickMask} onclose={() => (maskOpen = false)} />
                  </Popover>
                {/if}
              </div>
            {/if}
            <label class="fld" class:bad={!!errors.screen.size}><span>Size</span>
              <select data-testid="screen-size" value={customSize ? "custom" : sizeKey(sz.width, sz.height)} onchange={(e) => onSizePreset(e.currentTarget.value)}>
                {#each SIZE_PRESETS as p (p.label)}<option value={sizeKey(p.width, p.height)}>{p.label}</option>{/each}
                <option value="custom">Custom…</option>
              </select>
            </label>
            {#if customSize}
              <span class="custom">
                <input type="number" class="num" data-testid="screen-width" aria-label="Width" min={LIMITS.minSide} max={LIMITS.maxWidth} step="2" value={sz.width} oninput={(e) => setSize(Number(e.currentTarget.value), sz.height)} />
                x
                <input type="number" class="num" data-testid="screen-height" aria-label="Height" min={LIMITS.minSide} max={LIMITS.maxHeight} step="2" value={sz.height} oninput={(e) => setSize(sz.width, Number(e.currentTarget.value))} />
              </span>
            {/if}
            <label class="fld" class:bad={!!errors.screen.frameRate}><span>Frame rate</span>
              <select data-testid="screen-rate" value={`${rate.num}/${rate.den}`} onchange={(e) => onRate(e.currentTarget.value)}>
                {#each RATE_PRESETS as p (p.label)}<option value={`${p.rate.num}/${p.rate.den}`}>{p.label} fps</option>{/each}
                {#if !RATE_PRESETS.some((p) => sameRate(p.rate, rate))}<option value={`${rate.num}/${rate.den}`}>{fpsLabel(rate)} fps</option>{/if}
              </select>
            </label>
            <label class="fld"><span>Alpha</span>
              <select data-testid="screen-alpha" value={alpha} onchange={(e) => (draft!.screen.alpha = e.currentTarget.value as typeof alpha)}>
                {#each ALPHA_MODES as a (a.value)}<option value={a.value} title={a.hint}>{a.label}</option>{/each}
              </select>
            </label>
            <label class="chk" title="A disabled screen keeps its settings and outputs but renders and sends nothing">
              <input type="checkbox" data-testid="screen-enabled" checked={draft.screen.enabled !== false} onchange={(e) => (draft!.screen.enabled = e.currentTarget.checked)} /> Enabled
            </label>
          </div>
          <div class="hint">
            {ALPHA_MODES.find((a) => a.value === alpha)?.hint}
            {#if errors.screen.size}<span class="ferr">{errors.screen.size}</span>{/if}
            {#if errors.screen.frameRate}<span class="ferr">{errors.screen.frameRate}</span>{/if}
            {#if errors.screen.alpha}<span class="ferr">{errors.screen.alpha}</span>{/if}
            {#if errors.screen.background}<span class="ferr">{errors.screen.background}</span>{/if}
            {#if errors.screen.mask}<span class="ferr" data-testid="screen-mask-error">Mask: {errors.screen.mask}</span>{/if}
          </div>

          {@const shown = draft.outputs.filter((o) => !o.deleted)}
          {@const arr = arrangementOf(draft.screen)}
          {#if layouts}
            {@const tile = tileSizeOf(arr, sz)}
            <div class="arrange" data-testid="arrangement">
              <span class="lab">Arrangement</span>
              <div class="seg" role="radiogroup" aria-label="Arrangement">
                {#each ARRANGEMENTS as a (a.kind)}
                  <button role="radio" aria-checked={arr.kind === a.kind} class:on={arr.kind === a.kind} title={a.hint} data-testid={`arrangement-${a.kind}`}
                    onclick={() => arrange(a.kind === "grouped" || a.kind === "edgeBlend" ? { kind: a.kind, columns: isTiled(arr) ? arr.columns : 2 } : { kind: a.kind })}>{a.label}</button>
                {/each}
              </div>
              {#if isTiled(arr)}
                <label class="fld"><span>Across</span>
                  <input type="number" class="num s" data-testid="arrangement-columns" min="1" max="8" value={arr.columns} onchange={(e) => arrange({ columns: Math.max(1, Math.min(8, Math.round(Number(e.currentTarget.value)))) })} />
                </label>
                <label class="fld"><span>Down</span>
                  <input type="number" class="num s" data-testid="arrangement-rows" min="1" max="8" value={arr.rows} onchange={(e) => arrange({ rows: Math.max(1, Math.min(8, Math.round(Number(e.currentTarget.value)))) })} />
                </label>
                <label class="fld" title="The size of each output (display or projector)"><span>Each output</span>
                  <span class="custom">
                    <input type="number" class="num" data-testid="arrangement-tile-width" aria-label="Tile width" step="2" value={Math.round(tile.width)} onchange={(e) => arrange({}, { width: Number(e.currentTarget.value), height: Math.round(tile.height) })} />
                    x
                    <input type="number" class="num" data-testid="arrangement-tile-height" aria-label="Tile height" step="2" value={Math.round(tile.height)} onchange={(e) => arrange({}, { width: Math.round(tile.width), height: Number(e.currentTarget.value) })} />
                  </span>
                </label>
                {#if arr.kind === "grouped"}
                  <label class="fld" title="Picture hidden behind the frames between two displays, in screen pixels (across, down)"><span>Bezel</span>
                    <span class="custom">
                      <input type="number" class="num s" data-testid="arrangement-bezel-x" aria-label="Bezel across" min="0" max="2000" step="2" value={arr.bezel.x} onchange={(e) => arrange({ bezel: { x: Number(e.currentTarget.value), y: arr.bezel.y } })} />
                      <input type="number" class="num s" data-testid="arrangement-bezel-y" aria-label="Bezel down" min="0" max="2000" step="2" value={arr.bezel.y} onchange={(e) => arrange({ bezel: { x: arr.bezel.x, y: Number(e.currentTarget.value) } })} />
                    </span>
                  </label>
                {:else}
                  <label class="fld" title="Pixels two neighbouring projectors both show (across, down)"><span>Overlap</span>
                    <span class="custom">
                      <input type="number" class="num s" data-testid="arrangement-overlap-x" aria-label="Overlap across" min="0" step="2" value={arr.overlap.x} onchange={(e) => arrange({ overlap: { x: Number(e.currentTarget.value), y: arr.overlap.y } })} />
                      <input type="number" class="num s" data-testid="arrangement-overlap-y" aria-label="Overlap down" min="0" step="2" value={arr.overlap.y} onchange={(e) => arrange({ overlap: { x: arr.overlap.x, y: Number(e.currentTarget.value) } })} />
                    </span>
                  </label>
                  <label class="fld" title="The projectors' gamma: the blend fades the light, not the signal, so the overlap is as bright as the rest (2.2 for most projectors)"><span>Gamma</span>
                    <input type="number" class="num s" data-testid="arrangement-gamma" min="1" max="3" step="0.1" value={arr.blend.gamma} onchange={(e) => arrange({ blend: { gamma: Number(e.currentTarget.value), curve: arr.blend.curve } })} />
                  </label>
                  <label class="fld" title="The shape of the fade: 1 is a straight ramp, higher values are softer in the middle"><span>Curve</span>
                    <input type="number" class="num s" data-testid="arrangement-curve" min="1" max="4" step="0.5" value={arr.blend.curve} onchange={(e) => arrange({ blend: { gamma: arr.blend.gamma, curve: Number(e.currentTarget.value) } })} />
                  </label>
                {/if}
              {/if}
            </div>
            <div class="hint">
              <span>{ARRANGEMENTS.find((a) => a.kind === arr.kind)?.hint}</span>{#if isTiled(arr)} <span>The screen is {sz.width} x {sz.height}.</span>{/if}
              {#if errors.screen.arrangement}<span class="ferr" data-testid="arrangement-error">{errors.screen.arrangement}</span>{/if}
            </div>
          {/if}
          <div class="canvas">
            {#if layouts && isTiled(arr)}
              <div class="wall" data-testid="arrangement-canvas" style:aspect-ratio={`${sz.width} / ${sz.height}`}>
                {#each tileRects(arr, sz) as t (t.index)}
                  {@const outs = tileOutputs(t.index)}
                  {@const first = outs[0]}
                  {@const st = first ? outputStatus(first) : null}
                  {@const ph = first?.doc.transport?.type === "placeholder"}
                  <button class="tile" class:empty={!first} class:ph data-testid="arrangement-tile"
                    style:left={`${t.x * 100}%`} style:top={`${t.y * 100}%`} style:width={`${t.w * 100}%`} style:height={`${t.h * 100}%`}
                    title={first ? `${tileLabel(arr, t.index)}: ${st?.title ?? ""}` : `${tileLabel(arr, t.index)}: no output yet. Click to add a placeholder for it.`}
                    onclick={() => (first ? document.querySelector(`[data-output-key="${first.key}"]`)?.scrollIntoView({ block: "nearest" }) : addPlaceholder(t.index))}>
                    <span class="tnum">{t.index + 1}</span>
                    {#if first}
                      <span class="tname">{sourceName(first.doc) || "(no name)"}{outs.length > 1 ? ` +${outs.length - 1}` : ""}</span>
                      <span class="ostat {ph ? 'placeholder' : st?.cls}">{ph ? "Placeholder · no hardware yet" : `${transportLabel(first.doc.transport?.type)} · ${st?.cls === "ok" ? "sending" : st?.cls === "pending" ? "new" : st?.cls}`}</span>
                    {:else}
                      <span class="ostat">Empty · add a placeholder</span>
                    {/if}
                  </button>
                {/each}
                {#if arr.kind === "edgeBlend"}
                  {#each tileRects(arr, sz) as t (t.index)}
                    {#if t.x > 0}<span class="blendzone" style:left={`${t.x * 100}%`} style:top={`${t.y * 100}%`} style:width={`${(arr.overlap.x / sz.width) * 100}%`} style:height={`${t.h * 100}%`}></span>{/if}
                    {#if t.y > 0}<span class="blendzone" style:left={`${t.x * 100}%`} style:top={`${t.y * 100}%`} style:width={`${t.w * 100}%`} style:height={`${(arr.overlap.y / sz.height) * 100}%`}></span>{/if}
                  {/each}
                {/if}
              </div>
              {#if shown.some((o) => o.doc.tile == null)}
                <p class="noout">{shown.filter((o) => o.doc.tile == null).length} output(s) show the whole screen.</p>
              {/if}
            {:else if shown.length}
              <div class="outs" style:--n={shown.length}>
                {#each shown as o (o.key)}
                  {@const st = outputStatus(o)}
                  <div class="obox" class:disabled={o.doc.enabled === false} style:aspect-ratio={`${sz.width} / ${sz.height}`} title={st.title}>
                    <span>{sourceName(o.doc) || "(no name)"}</span>
                    <span class="ostat {st.cls}">{transportLabel(o.doc.transport?.type)}{outputTarget(o.doc) !== "full" ? ` · ${outputTarget(o.doc) === "fill" ? "Fill" : "Key"}` : ""} · {st.cls === "ok" ? "sending" : st.cls === "pending" ? "new" : st.cls}</span>
                  </div>
                {/each}
              </div>
            {:else}
              <p class="noout">This screen has no outputs yet. Add one below.</p>
            {/if}
          </div>

          <div class="outsec">
            <div class="oshead">
              <span class="ostitle">Outputs</span>
              <button class="addout" data-testid="add-output" onclick={addOutput} title="Send this screen somewhere (IDN Open Protocol on the network, compatible with NDI®)"><Plus size={14} /> Add output</button>
            </div>
            {#each draft.outputs as o (o.key)}
              {#if o.deleted}
                <div class="gone">“{sourceName(o.doc)}” will be deleted when you apply. <button onclick={() => (o.deleted = false)}>Undo</button></div>
              {:else}
                {@const st = outputStatus(o)}
                {@const kind = o.doc.transport?.type}
                {@const named = isNamed(kind)}
                {@const tab = outTab[o.key] ?? "hw"}
                <div class="box" data-testid="output-card" data-output-key={o.key}>
                  <div class="bhead">
                    <span class="dname">{sourceName(o.doc) || "(no name)"}</span>
                    <label class="chk" title="A disabled output keeps its settings but sends nothing">
                      <input type="checkbox" data-testid="output-enabled" checked={o.doc.enabled !== false} onchange={(e) => (o.doc.enabled = e.currentTarget.checked)} /> Enabled
                    </label>
                    <button class="del" data-testid="output-delete" title="Delete this output (on Apply)" aria-label="Delete output" onclick={() => removeOutput(o)}><Trash2 size={15} /></button>
                  </div>
                  <div class="otabs" role="tablist" aria-label="Output settings">
                    {#each OUT_TABS as t (t.id)}
                      <button role="tab" aria-selected={tab === t.id} class:on={tab === t.id} onclick={() => (outTab[o.key] = t.id)} data-testid={`output-tab-${t.id}`}>{t.label}</button>
                    {/each}
                  </div>
                  {#if (tab === "color" || tab === "pin") && !adjustable}
                    <div class="otab-off" data-testid="output-tab-unavailable">
                      {tab === "color" ? "Colour correction" : "Corner pin"} needs a newer server.
                    </div>
                  {:else if tab === "color"}
                    <ColorTab entry={draft.base} onapplied={adjusted} />
                  {:else if tab === "pin"}
                    <CornerPinTab entry={draft.base} onapplied={adjusted} />
                  {:else if tab === "alpha"}
                    <div class="otab-alpha">
                      <span class="lab">Alpha type</span>
                      <select data-testid="alpha-override" class:bad={!!outErr(o, "alphaOverride")} value={alphaOverrideValue(o.doc)} onchange={(e) => (o.doc = setAlphaOverride(o.doc, e.currentTarget.value))} title="Send this output with different alpha than the screen">
                        <option value="follow">Follow screen ({ALPHA_MODES.find((a) => a.value === alpha)?.label})</option>
                        {#each ALPHA_MODES as a (a.value)}<option value={a.value}>{a.label}</option>{/each}
                      </select>
                      {#if outErr(o, "alphaOverride")}<div class="ferr">{outErr(o, "alphaOverride")}</div>{/if}
                      <p class="ahint">{ALPHA_MODES.find((a) => a.value === (alphaOverrideValue(o.doc) === "follow" ? alpha : alphaOverrideValue(o.doc)))?.hint}</p>
                    </div>
                  {:else}
                  <div class="cols">
                    <div class="col">
                      <span class="lab">Output</span>
                      <select data-testid="transport-select" class:bad={!!outErr(o, "transport")} value={kind ?? "unknown"} title={transportTitle(kind)} onchange={(e) => changeTransport(o, e.currentTarget.value)}>
                        {#if !kind || kind === "unknown"}<option value="unknown">Choose…</option>{/if}
                        {#each transportList as t (t.kind)}
                          <option value={t.kind} disabled={!t.supported && t.kind !== kind} title={[t.status, t.detail].filter(Boolean).join(" — ")}>
                            {t.kind === "display" ? "System display" : t.label}{!t.supported ? " (not available yet)" : !t.available ? " (unavailable here)" : t.version ? ` (${t.version})` : ""}
                          </option>
                        {/each}
                        {#if layouts}
                          <option value="placeholder" title="An output slot without hardware yet: kept in the configuration, sends nothing">Placeholder (no hardware yet)</option>
                        {:else}
                          <option value="placeholder" disabled title="Placeholder outputs need a newer server">Placeholder (not available yet)</option>
                        {/if}
                      </select>
                      {#if outErr(o, "transport")}<div class="ferr">{outErr(o, "transport")}</div>{/if}
                      {#if layouts && isTiled(arr)}
                        <div class="inner">
                          <label><span>Shows:</span>
                            <select data-testid="output-tile" value={o.doc.tile == null ? "" : String(o.doc.tile)} title="The part of the screen this output shows" onchange={(e) => (o.doc = setTile(o.doc, e.currentTarget.value === "" ? null : Number(e.currentTarget.value)))}>
                              <option value="">The whole screen</option>
                              {#each tileRects(arr, sz) as t (t.index)}<option value={String(t.index)}>{tileLabel(arr, t.index)}</option>{/each}
                            </select>
                          </label>
                          {#if outErr(o, "tile")}<div class="ferr" data-testid="tile-error">{outErr(o, "tile")}</div>{/if}
                        </div>
                      {/if}
                      {#if kind === "placeholder"}
                        <div class="tnote" data-testid="placeholder-note">A placeholder keeps this output's place until its display or projector is there: choose its hardware above when it is. It sends nothing.</div>
                      {/if}
                      {#if transportList.find((t) => t.kind === kind && !t.available)}
                        <div class="tnote" title={transportTitle(kind)}>{transportList.find((t) => t.kind === kind)?.status}</div>
                      {/if}
                      {#if kind === "display"}
                        {@const conn = displayOf(o.doc)}
                        {@const cur = displays.find((d) => d.id.toLowerCase() === conn.toLowerCase())}
                        <div class="inner">
                          <label><span>Display:</span>
                            <select data-testid="display-select" class:bad={!!outErr(o, "transport.sourceName")} value={conn} title="The monitor or projector attached to the server box" onchange={(e) => {
                              const d = displays.find((x) => x.id === e.currentTarget.value);
                              if (d) o.doc = setDisplay(o.doc, d);
                            }}>
                              {#if !conn}<option value="">Choose…</option>{/if}
                              {#if conn && !cur}<option value={conn}>{conn} · not found</option>{/if}
                              {#each displays as d (d.id)}
                                {@const user = displayUser(d, o)}
                                <option value={d.id}>{displayLabel(d)}{user ? ` · used by ${user}` : ""}</option>
                              {/each}
                            </select>
                          </label>
                          {#if outErr(o, "transport.sourceName")}<div class="ferr" data-testid="display-error">{outErr(o, "transport.sourceName")}</div>{/if}
                          {#if layouts && cur?.connected && !cur.boundTo.length}
                            <button class="identbtn" data-testid="display-identify" disabled={identifying === cur.id} title="Show this display's name on it for a few seconds, to find which one it is (it isn't used by any screen yet)" onclick={() => identifyDisplay(cur.id)}>{identifying === cur.id ? "Showing its name…" : "Identify this display"}</button>
                          {/if}
                          {#if cur?.source === "window" || !displayFullscreen(o.doc)}
                            <label><span>Show:</span>
                              <select data-testid="display-fullscreen" value={displayFullscreen(o.doc) ? "full" : "window"} disabled={!conn} title="Full screen covers the whole monitor (no frame, pointer hidden over it, never takes the keyboard or mouse); a window can be moved and resized, for rehearsal" onchange={(e) => (o.doc = setDisplayFullscreen(o.doc, e.currentTarget.value === "full"))}>
                                <option value="full">Full screen</option>
                                <option value="window">In a window</option>
                              </select>
                            </label>
                          {/if}
                          {#if cur && displayFullscreen(o.doc) && displayRisk(cur, here)}
                            <div class="tnote" data-testid="display-risk">Full screen here covers {displayRisk(cur, here)}: you will be asked to confirm when you apply.</div>
                          {/if}
                          {#if cur?.source !== "window"}
                            <label><span>Mode:</span>
                              <select data-testid="display-mode" value={displayModeValue(o.doc)} disabled={!conn} title="The mode the display is set to; the screen is scaled to it, keeping its shape" onchange={(e) => (o.doc = setDisplayMode(o.doc, e.currentTarget.value))}>
                                {#each displayModeOptions(cur, displayModeValue(o.doc)) as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
                              </select>
                            </label>
                          {/if}
                          {#if !displays.length}
                            <div class="tnote" data-testid="display-none">This server has no attached displays.</div>
                          {:else if cur && !cur.connected}
                            <div class="tnote" data-testid="display-unplugged">{cur.id} is not connected: the output starts when a display is plugged in.</div>
                          {/if}
                        </div>
                      {:else}
                      <div class="inner">
                        <label><span>Name:</span>
                          <input
                            data-testid="source-name"
                            class:bad={!!outErr(o, "transport.sourceName")}
                            value={sourceName(o.doc)}
                            disabled={!named && kind !== "placeholder"}
                            title={named ? "The name receivers see on the network" : kind === "placeholder" ? "What this output slot is called until its hardware is chosen" : "This transport has no name"}
                            oninput={(e) => (o.doc = setSourceName(o.doc, e.currentTarget.value))}
                          />
                        </label>
                        {#if outErr(o, "transport.sourceName")}<div class="ferr" data-testid="source-name-error">{outErr(o, "transport.sourceName")}</div>{/if}
                        {#if isIdn(kind)}
                          <label><span>Groups:</span>
                            <input
                              placeholder="public"
                              title="Groups, comma separated (empty = public)"
                              value={(o.doc.transport?.type === "idn" ? (o.doc.transport.groups ?? []) : []).join(", ")}
                              onchange={(e) => {
                                const g = e.currentTarget.value.split(",").map((x) => x.trim()).filter(Boolean);
                                const t = o.doc.transport;
                                if (t?.type === "idn") o.doc.transport = g.length ? { ...t, groups: g } : { type: t.type, sourceName: t.sourceName };
                              }}
                            />
                          </label>
                        {/if}
                        {#if kind !== "placeholder"}
                        <label><span>Mode:</span>
                          <select data-testid="mode-select" class:bad={!!(outErr(o, "resolutionOverride") || outErr(o, "frameRateOverride"))} value={outputMode(o.doc)} onchange={(e) => (o.doc = setOutputMode(o.doc, e.currentTarget.value))}>
                            {#each modeOptions(layouts && isTiled(arr) && o.doc.tile != null ? { ...draft.screen, size: tileSizeOf(arr, sz) } : draft.screen, outputMode(o.doc)) as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
                          </select>
                        </label>
                        {/if}
                        {#if outErr(o, "resolutionOverride") || outErr(o, "frameRateOverride")}<div class="ferr">{outErr(o, "resolutionOverride") ?? outErr(o, "frameRateOverride")}</div>{/if}
                      </div>
                      {/if}
                      <div class="inner row" class:bad={!!outErr(o, "videoDelay")}>
                        <span>Video Delay</span>
                        <input
                          type="number"
                          class="num"
                          data-testid="delay-input"
                          min="0"
                          max={LIMITS.maxDelay}
                          value={o.doc.videoDelay ?? 0}
                          title="Hold the picture back this many frames (0–30) to line up with delayed audio or cameras"
                          oninput={(e) => {
                            const v = Number(e.currentTarget.value);
                            if (v) o.doc.videoDelay = v;
                            else delete o.doc.videoDelay;
                          }}
                        />
                        <span>frames</span>
                      </div>
                      {#if outErr(o, "videoDelay")}<div class="ferr">{outErr(o, "videoDelay")}</div>{/if}
                    </div>
                    <div class="col">
                      <span class="lab">Output Target</span>
                      {#if fillKey}
                        {@const tgt = outputTarget(o.doc)}
                        <select data-testid="output-target" class:bad={!!outErr(o, "target")} value={tgt} title={OUTPUT_TARGETS.find((t) => t.value === tgt)?.hint} onchange={(e) => (o.doc = setOutputTarget(o.doc, e.currentTarget.value))}>
                          {#each OUTPUT_TARGETS as t (t.value)}<option value={t.value} title={t.hint}>{t.label}</option>{/each}
                        </select>
                        {#if outErr(o, "target")}<div class="ferr">{outErr(o, "target")}</div>{/if}
                        {#if tgt === "fill" && !hasKeyOutput()}
                          <button class="addkey" data-testid="add-key-output" onclick={() => addKeyOutput(o)} title="Add a second output of this screen that sends the key, on the next free display"><Plus size={13} /> Add the key output</button>
                        {/if}
                        {#if fillKeyHint(o.doc, alpha)}<div class="fkhint" data-testid="fill-key-hint">{fillKeyHint(o.doc, alpha)}</div>{/if}
                      {:else}
                        <select value="full" title="Fill and Key need a newer server: an output sends the full picture">
                          <option value="full">Full</option>
                          <option disabled>Fill (needs a newer server)</option>
                          <option disabled>Key (needs a newer server)</option>
                        </select>
                      {/if}
                      {#if isNamed(kind)}
                        <label class="chk" title="Send the program audio (the master bus) with this output's video">
                          <input
                            type="checkbox"
                            data-testid="output-embed-audio"
                            checked={o.doc.embedAudio === true}
                            onchange={(e) => {
                              if (e.currentTarget.checked) o.doc.embedAudio = true;
                              else delete o.doc.embedAudio;
                            }}
                          /> Embed audio
                        </label>
                      {/if}
                      {#each Object.entries(errors.outputs[o.key] ?? {}).filter(([k]) => !["transport", "transport.sourceName", "videoDelay", "resolutionOverride", "frameRateOverride", "alphaOverride", "target"].includes(k)) as [, msg] (msg)}
                        <div class="ferr">{msg}</div>
                      {/each}
                    </div>
                  </div>
                  {/if}
                  <div class="status {st.cls}" data-testid="output-status" title={st.title}>{st.text}</div>
                </div>
              {/if}
            {/each}
          </div>
        {:else if loaded}
          <p class="noout">{entries.length ? "Select a screen." : "This server has no screens yet. Press + next to Audience to add one."}</p>
        {/if}
      </div>

    </div>

    {#if prompt}
      {@const p = prompt}
      <div class="pback" role="presentation" onclick={dismissPrompt}></div>
      <div class="prompt" role="alertdialog" aria-label={p.title} data-testid={p.testid}>
        <div class="ptitle">{p.title}</div>
        <p>{p.message}</p>
        {#if p.detail?.length}<ul>{#each p.detail as d (d)}<li>{d}</li>{/each}</ul>{/if}
        <div class="pbtns">
          <button data-testid="prompt-cancel" onclick={dismissPrompt}>{p.testid === "unsaved-prompt" ? "Keep editing" : "Cancel"}</button>
          {#if p.alt}{@const alt = p.alt}<button data-testid="prompt-alt" onclick={() => runPrompt(alt.run)}>{alt.label}</button>{/if}
          <button class:danger={p.danger} data-testid="prompt-ok" onclick={() => runPrompt(p.run)}>{p.ok}</button>
        </div>
      </div>
    {/if}
  </div>
</div>

{#if menu}
  {@const m = menu}
  <div class="mback" role="presentation" onclick={() => (menu = null)} oncontextmenu={(e) => { e.preventDefault(); menu = null; }}></div>
  <div class="cmenu" role="menu" style:left={`${m.x}px`} style:top={`${m.y}px`}>
    <button role="menuitem" data-testid="menu-duplicate" onclick={() => duplicate(m.id)}>Duplicate</button>
    <button role="menuitem" data-testid="menu-delete" class="danger" onclick={() => askDelete(m.id)}>Delete…</button>
  </div>
{/if}

{#if maskEditor}
  {#await maskEditorModule() then M}
    <M.default
      initial={maskEditor.initial}
      onclose={() => (maskEditor = null)}
      onchanged={(m: MaskEntry[]) => (masks = m)}
    />
  {:catch e}
    <Dialog title="Mask editor" width={460} onclose={() => (maskEditor = null)}><ChunkError error={e} what="The mask editor" /></Dialog>
  {/await}
{/if}

<style>
  .page {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-panel);
  }
  .cfg {
    position: relative;
    flex: 1;
    display: grid;
    grid-template-columns: 280px 1fr;
    min-height: 0;
  }
  .left {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--list);
    border-right: 1px solid var(--line);
  }
  .half {
    flex: 1 1 0;
    min-height: 100px;
    display: flex;
    flex-direction: column;
  }
  .ghead {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 46px;
    padding: 0 12px;
    background: var(--work-2);
  }
  .gtitle {
    flex: 1;
    font-size: 22px;
    font-weight: 700;
  }
  .add {
    border: 0;
    background: transparent;
    color: var(--text-2);
    display: grid;
    padding: 2px;
    border-radius: 3px;
  }
  .add:hover {
    background: var(--surface-raised);
    color: var(--text-strong);
  }
  .slist {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 2px;
  }
  .srow {
    position: relative;
    display: flex;
    border: 1px solid transparent;
    border-radius: 2px;
  }
  .srow.sel {
    background: var(--line-strong);
    border-color: var(--line-hover);
  }
  .sitem {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 6px 10px;
    border: 0;
    background: transparent;
    color: var(--text-strong);
    text-align: left;
  }
  .sname {
    font-size: 17px;
    font-weight: 700;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ssize {
    color: var(--text-2);
    font-size: 13px;
  }
  .more {
    flex: none;
    align-self: center;
    border: 0;
    background: transparent;
    color: var(--text-label);
    padding: 4px;
    margin-right: 4px;
    border-radius: 3px;
    display: grid;
    opacity: 0.5;
  }
  .srow:hover .more,
  .srow.sel .more,
  .more:focus-visible {
    opacity: 1;
  }
  .more:hover {
    background: var(--line-strong);
    color: var(--text-strong);
  }
  .none {
    padding: 8px 12px;
    color: var(--text-meta);
    font-style: italic;
  }
  .identify {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 48px;
    padding: 0 12px;
    background: var(--work-2);
    border-top: 1px solid var(--line);
  }
  .tp {
    position: relative;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 12px 12px;
    background: var(--work-2);
  }
  .tpanel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
  }
  .tfld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .tfld.row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
  .tfld select {
    color: var(--text);
    font-size: var(--fs);
  }
  .tfld input[type="color"] {
    width: 46px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-sm);
    background: var(--surface-panel);
  }
  .tpgo {
    height: 32px;
    border: 0;
    border-radius: var(--r);
    background: var(--info);
    color: #0e1a2e;
    font-weight: 700;
  }
  .tpgo.off {
    background: var(--surface-overlay);
    color: var(--text);
    border: 1px solid var(--line-strong);
  }
  .tpgo:disabled {
    opacity: 0.5;
  }
  .chips {
    display: flex;
    gap: 4px;
    margin-top: 3px;
  }
  .otabs {
    display: flex;
    justify-content: center;
    gap: 2px;
    margin: 2px 0 10px;
  }
  .otabs button {
    min-width: 104px;
    padding: 6px 14px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-2);
  }
  .otabs button:hover {
    color: var(--text);
  }
  .otabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent-ring);
    background: var(--surface-raised);
  }
  .otab-off {
    padding: 24px 12px;
    text-align: center;
    color: var(--text-label);
  }
  .otab-alpha {
    max-width: 420px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ahint {
    margin: 0;
    color: var(--text-label);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .tp > button {
    width: 100%;
    height: 28px;
    border-radius: 3px;
    border: 1px solid var(--line-strong);
    background: var(--line-strong);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tp > button.active {
    background: var(--accent-dim);
    border-color: transparent;
  }
  .tp > button:disabled {
    opacity: 0.55;
  }
  .cmenu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 5px 12px;
    border: 0;
    background: transparent;
    color: var(--text-strong);
    text-align: left;
  }
  .cmenu button:hover {
    background: var(--menu-hover);
  }
  .main {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--chrome);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .banner {
    /* Sticky: visible even when the pane is scrolled down to an output. */
    position: sticky;
    top: 8px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 14px 0;
    padding: 8px 12px;
    border-radius: 3px;
    box-shadow: 0 2px 8px rgb(0 0 0 / 45%);
  }
  .banner.conflict {
    background: var(--live-soft);
    border: 1px solid var(--warn);
  }
  .banner.conflict span {
    flex: 1;
  }
  .banner.err {
    background: var(--danger-bg);
    border: 1px solid var(--danger);
  }
  .banner button {
    padding: 3px 12px;
    border-radius: 3px;
    border: 1px solid #aaa;
    background: var(--line-strong);
    color: var(--text-strong);
  }
  .mhead {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 14px;
    background: var(--list);
  }
  .namebox {
    flex: 1;
    min-width: 0;
  }
  .mtitle {
    width: 100%;
    max-width: 460px;
    font-size: 24px;
    font-weight: 700;
    color: var(--text-strong);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 0 4px;
    margin-left: -5px;
  }
  .mtitle:hover,
  .mtitle:focus {
    border-color: var(--line-strong);
    background: var(--surface-panel);
  }
  .bad,
  .mtitle.bad {
    border-color: var(--danger) !important;
  }
  .ferr {
    color: var(--danger-text);
    font-size: 12px;
    margin-top: 2px;
  }
  .msize {
    color: var(--text-2);
    font-size: 14px;
  }
  .color {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    margin-top: 6px;
  }
  .swatch {
    width: 70px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--line-strong);
    background: transparent;
  }
  .swatch.off {
    opacity: 0.35;
  }
  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px 14px;
    padding: 6px 14px 4px;
    background: var(--work-2);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 11px;
    color: var(--text-label);
  }
  .fld select {
    min-width: 110px;
    min-height: 24px;
    padding: 1px 6px;
    font-size: 13px;
  }
  .fld.bad select {
    border-color: var(--danger);
  }
  .anchor {
    position: relative;
  }
  /* The stage mask picker: a button that looks like the row's selects and opens a menu. */
  .pick {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    min-width: 130px;
    max-width: 200px;
    min-height: 24px;
    padding: 1px 6px;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .plabel {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .pick:not(.set) {
    color: var(--text-meta);
  }
  .fld.bad .pick {
    border-color: var(--danger);
  }
  .custom {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-bottom: 2px;
  }
  .num {
    width: 72px;
    text-align: right;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text);
    padding-bottom: 3px;
  }
  .hint {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    padding: 2px 14px 8px;
    background: var(--work-2);
    color: var(--text-label);
    font-size: 12px;
  }
  .canvas {
    flex: none;
    height: clamp(130px, 24vh, 250px);
    display: grid;
    place-items: center;
    padding: 16px 20px;
    background: var(--work-2);
    border-top: 1px solid var(--line);
  }
  .outs {
    display: flex;
    gap: 16px;
    height: 100%;
    max-width: 100%;
  }
  .obox {
    height: 100%;
    max-width: calc((100% - 16px * (var(--n) - 1)) / var(--n));
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    padding: 12px 6px 0;
    border: 3px solid var(--text);
    background: var(--surface-raised);
    color: var(--text-strong);
    font-size: 16px;
    text-align: center;
    overflow: hidden;
  }
  .obox.disabled {
    border-style: dashed;
    border-color: var(--line-hover);
    color: var(--text-label);
  }
  .ostat {
    font-size: 12px;
    color: var(--text-2);
  }
  .ostat.ok {
    color: var(--ok-text);
  }
  .ostat.unavailable,
  .ostat.error {
    color: var(--warn);
  }
  .noout {
    color: var(--text-label);
    padding: 16px;
  }
  .arrange {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px 14px;
    padding: 6px 14px 4px;
    background: var(--work-2);
    border-top: 1px solid var(--line);
  }
  .arrange .lab {
    align-self: center;
    font-size: 12px;
    color: var(--text-label);
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--line);
    border-radius: var(--r);
    overflow: hidden;
  }
  .seg button {
    min-height: 26px;
    padding: 2px 12px;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-raised);
    border: none;
    border-right: 1px solid var(--line);
  }
  .seg button:last-child {
    border-right: none;
  }
  .seg button.on {
    color: var(--on-accent, #fff);
    background: var(--accent);
  }
  .num.s {
    width: 56px;
  }
  .wall {
    position: relative;
    height: 100%;
    max-width: 100%;
    max-height: 100%;
    background: var(--work-3, var(--work-2));
    outline: 1px dashed var(--line);
  }
  .tile {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 4px;
    border: 2px solid var(--text);
    background: var(--surface-raised);
    color: var(--text-strong);
    font-size: 14px;
    text-align: center;
    overflow: hidden;
    cursor: pointer;
  }
  .tile.ph {
    border-style: dashed;
    border-color: var(--text-label);
    color: var(--text-2);
  }
  .tile.empty {
    border: 2px dashed var(--line-hover);
    background: transparent;
    color: var(--text-label);
  }
  .tile .tnum {
    position: absolute;
    top: 3px;
    left: 6px;
    font-size: 11px;
    color: var(--text-label);
  }
  .ostat.placeholder {
    color: var(--text-label);
  }
  .blendzone {
    position: absolute;
    pointer-events: none;
    background: repeating-linear-gradient(135deg, transparent 0 4px, color-mix(in srgb, var(--warn) 45%, transparent) 4px 6px);
  }
  .identbtn {
    align-self: flex-start;
    margin-top: 4px;
    padding: 2px 10px;
    font-size: 12px;
  }
  .outsec {
    padding: 10px 14px 16px;
  }
  .oshead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .ostitle {
    font-size: 18px;
  }
  .addout {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 12px;
    border-radius: 3px;
    border: 1px solid var(--line-strong);
    background: var(--accent);
    color: var(--text-strong);
  }
  .gone {
    padding: 8px 12px;
    margin-bottom: 10px;
    border: 1px dashed var(--line-strong);
    color: var(--text-2);
  }
  .gone button {
    margin-left: 8px;
    border: 1px solid var(--line-hover);
    background: var(--surface-overlay);
    color: var(--text-strong);
    border-radius: 3px;
    padding: 1px 10px;
  }
  .box {
    border: 2px solid var(--line-strong);
    padding: 10px 14px 14px;
    margin-bottom: 12px;
  }
  .bhead {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 8px;
  }
  .dname {
    flex: 1;
    min-width: 0;
    font-size: 17px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .del {
    border: 0;
    background: transparent;
    color: var(--text-2);
    display: grid;
    padding: 4px;
    border-radius: 3px;
  }
  .del:hover {
    background: var(--danger-bg);
    color: var(--text-strong);
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .lab {
    color: var(--text);
  }
  .col > select {
    min-height: 28px;
    background-color: var(--line-strong);
    border-color: var(--line-strong);
  }
  .col > select:disabled {
    opacity: 0.75;
  }
  .tnote {
    color: var(--warn);
    font-size: 12px;
  }
  .addkey {
    display: flex;
    align-items: center;
    align-self: flex-start;
    gap: 4px;
    margin-top: 6px;
    padding: 3px 10px;
    border-radius: 3px;
    border: 1px solid var(--line-strong);
    background: var(--surface-overlay);
    color: var(--text-strong);
  }
  .fkhint {
    margin-top: 6px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.35;
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 2px solid var(--line-strong);
    border-radius: 6px;
  }
  .inner label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .inner label > span {
    width: 52px;
    flex: none;
    text-align: right;
  }
  .inner input,
  .inner select {
    flex: 1;
    min-width: 0;
  }
  .inner.row {
    flex-direction: row;
    align-items: center;
  }
  .inner .num {
    width: 64px;
    flex: none;
  }
  .status {
    margin-top: 6px;
    padding: 6px 8px;
    border-radius: 3px;
    background: var(--surface-panel);
    font-size: 12px;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status.ok {
    color: var(--ok-text);
  }
  .status.error,
  .status.unavailable {
    color: var(--warn);
  }
  .status.pending {
    color: var(--accent-link);
  }
  .pback {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 10;
  }
  .prompt {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100% - 40px));
    padding: 16px 18px;
    background: var(--surface-raised);
    border: 1px solid var(--line-hover);
    border-radius: 6px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    z-index: 11;
  }
  .ptitle {
    font-size: 16px;
    font-weight: 700;
  }
  .prompt ul {
    margin: 0 0 8px;
    padding-left: 20px;
    color: var(--text);
  }
  .pbtns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .pbtns button {
    min-width: 80px;
    height: 28px;
    border-radius: 3px;
    border: 1px solid var(--line-hover);
    background: var(--line-strong);
    color: var(--text-strong);
  }
  .pbtns button.danger,
  .cmenu button.danger:hover {
    background: var(--danger-bg);
    border-color: var(--danger);
  }
  .mback {
    position: fixed;
    inset: 0;
    z-index: 70;
  }
  .cmenu {
    position: fixed;
    min-width: 150px;
    display: flex;
    flex-direction: column;
    padding: 4px 0;
    background: var(--menu-bg);
    border: 1px solid var(--line-hover);
    border-radius: 4px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    z-index: 71;
  }
  @media (max-width: 1100px), (max-height: 820px) {
    .cfg {
      grid-template-columns: 240px 1fr;
    }
    .gtitle {
      font-size: 19px;
    }
    .cols {
      grid-template-columns: 1fr;
    }
  }
</style>
