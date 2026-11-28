<script lang="ts">
  // The Reflow page: a presentation's words as text, one block per slide in bands by group
  // (vertical label, colour bar), with live thumbnails on the right. Typing is saved as
  // debounced `setSlideText` ops (edit/queue.ts); Ctrl+Enter or Insert Slide Break splits a
  // slide at the caret; Backspace at the start of a slide merges it into the previous one; Tab
  // moves between slides. Each slide's "…" menu (also the menu key or Shift+F10 in its text)
  // starts a group, continues the group above, breaks, merges, removes the media cue or deletes
  // (slideMenu.ts). The text uses the canvas's fixed style, not the presentation's font.
  // "Second language" (batch 16) shows each slide's second-language text beside its text
  // (edit/language.ts: one `setSlideTranslation` per change), with the language code and a
  // "Paste two languages" dialog that fills both from pasted lyrics. Smart paste: SongSelect lyrics
  // pasted into a slide (a `CCLI Song #` line, edit/songselect.ts) offer to become a new song with
  // its metadata (the SongSelect dialog previews it first), or go in as plain text.
  import PageToolbar from "../../ui/PageToolbar.svelte";
  import Slider from "../../ui/Slider.svelte";
  import Check from "lucide-svelte/icons/check";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Film from "lucide-svelte/icons/film";
  import TypeIcon from "lucide-svelte/icons/type";
  import ImageIcon from "lucide-svelte/icons/image";
  import Loader from "lucide-svelte/icons/loader-circle";
  import CloudOff from "lucide-svelte/icons/cloud-off";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Languages from "lucide-svelte/icons/languages";
  import ClipboardPaste from "lucide-svelte/icons/clipboard-paste";
  import Dialog from "../../ui/Dialog.svelte";
  import { chordOps, chordProPaste, chordProSplit, COMMON_LANGUAGES, isMachineTranslation, looksLikeChordPro, MACHINE_NOTICE, pasteOps, presentationLanguage, proposedText, secondText, secondTextOp, splitTwoLanguages, type PasteMode, type TwoLanguageSplit } from "../../edit/language";
  import { TranslatePreview } from "../../edit/translatePreview.svelte";
  import TranslateDialog from "../../components/TranslateDialog.svelte";
  import TranslateBanner from "../../components/TranslateBanner.svelte";
  import Sparkles from "lucide-svelte/icons/sparkles";
  import type { Presentation, PresentationOp, Section, Slide } from "$protocol";
  import { onDestroy, tick, untrack } from "svelte";
  import { ApiRequestError, type Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { aspectRatio, thumbWidthFor } from "../../model";
  import { EditQueue, type Conflict, type SaveStatus } from "../../edit/queue";
  import { thumbVersions } from "../show/sections";
  import { DEFAULT_GROUPS, groupChoices, groupOps, mergeOps, reflowBlocks, slideHash, splitOps, type GroupChoice, type ReflowBlock } from "../../edit/reflow";
  import { uuid } from "../../edit/ids";
  import { useLive } from "../../live";
  import ConflictDialog from "../../components/ConflictDialog.svelte";
  import SlideMenu from "./SlideMenu.svelte";
  import PresenceBadge from "../../components/PresenceBadge.svelte";
  import LockBanner from "../../components/LockBanner.svelte";
  import Lock from "lucide-svelte/icons/lock";
  import { copyTyping, presence } from "../../state/presence.svelte";
  import { looksLikeSongSelect } from "../../edit/songselect";
  import { canContinueAbove, continueGroupOps, deleteSlideOps, removeCueOps, slideBadges, slideMenuEntries, type SlideMenuEntry } from "./slideMenu";

  let { conn }: { conn: Connection } = $props();

  // ---- second language --------------------------------------------------------------------------

  /** Second-language column shown (on by default when the presentation has one). */
  let dual = $state(false);
  /** The second language's code ("es"). */
  let language = $state("");
  /** Local second text per slide (what the second text areas show). Saved through the edit
   *  queue (`setSecond`), like the first language's typing. */
  let seconds = $state<Record<string, string>>({});
  let pasteOpen = $state(false);
  // Machine translation (docs/DUAL-LANGUAGE-PLAN.md §5): "Translate to…" (all slides) or a
  // slide's menu ▸ "Translate this slide…" → a preview job on the server → its proposals in the
  // second column, labelled, until Keep (one `setSlideTranslation` per slide) or Discard.
  let canTranslate = $derived(!!session.server?.capabilities.includes("translate"));
  let translateOpen = $state(false);
  /** The slides the dialog translates (null: all). */
  let translateScope = $state<string[] | null>(null);
  /** The preview job and its proposals (shown in the second column until Keep or Discard). */
  // svelte-ignore state_referenced_locally
  const mt = new TranslatePreview(conn, (j) => {
    // Only for the presentation it translated (a switch cancels it, this is the backstop).
    if (j.presentation !== pres?.id) return;
    dual = true;
    if (!language) language = j.to;
  });
  let job = $derived(mt.job);
  let proposals = $derived(mt.proposals);
  let proposing = $derived(mt.proposing);
  let pasteText = $state("");
  let pasteMode = $state<PasteMode>("alternating");
  let pasteSeparator = $state("/");
  let pastePrimary = $state(true);
  // svelte-ignore state_referenced_locally
  const connStatus = conn.status;
  const live = useLive();
  const liveSlideId = live.liveSlideId;

  let pres = $derived<Presentation | null>(session.open?.presentation ?? null);
  let blocks = $derived(pres ? reflowBlocks(pres) : []);
  let slideById = $derived(new Map<string, Slide>((pres?.sections ?? []).flatMap((s) => (s.slides ?? []).map((sl) => [sl.id, sl] as const))));
  let sectionById = $derived(new Map<string, Section>((pres?.sections ?? []).map((s) => [s.id, s] as const)));
  let groups = $derived.by(() => {
    const out: { sectionId: string; name: string; color: string | null; blocks: ReflowBlock[] }[] = [];
    for (const b of blocks) {
      const last = out[out.length - 1];
      if (last && last.sectionId === b.sectionId) last.blocks.push(b);
      else out.push({ sectionId: b.sectionId, name: b.sectionName, color: b.color, blocks: [b] });
    }
    return out;
  });
  let globalGroups = $derived<GroupChoice[]>(
    session.globalGroups.length
      ? session.globalGroups.map((g) => ({ name: g.name ?? "", color: g.color ?? null, globalGroup: { id: g.id, name: g.name ?? "" } }))
      : DEFAULT_GROUPS,
  );
  let choices = $derived(groupChoices(pres, globalGroups));

  /** Local text per slide (what the text areas show). */
  let texts = $state<Record<string, string>>({});
  let status = $state<SaveStatus>("saved");
  let conflict = $state<Conflict | null>(null);
  let textZoom = $state(17);
  let thumbSize = $state(170);
  /** The slide whose "…" menu is open. */
  let menuFor = $state<string | null>(null);
  let menuButtons: Record<string, HTMLElement | null> = $state({});
  /** When the menu key / Shift+F10 opened a menu (the browser's own context menu follows). */
  let keyMenuAt = 0;
  let active = $state<{ id: string; caret: number } | null>(null);
  let focusNext: { index: number; caret: number | "end" } | null = null;
  let editorEl = $state<HTMLElement | null>(null);

  // ---- the queue (one per open presentation) --------------------------------------------------

  let queue: EditQueue | null = null;
  let queueFor: string | null = null;
  let unsubs: (() => void)[] = [];

  function makeQueue(id: string): EditQueue {
    const q = new EditQueue({
      send: (ops) => conn.rest.editPresentation(id, { ops, clientId: session.editor?.clientId }),
      applied: (label, r, mergeKey) => session.editor?.recordPresentation(id, label, r, mergeKey),
      isOffline: (e) => e instanceof ApiRequestError && (e.code === "network" || e.code === "timeout" || e.code === "unavailable"),
      conflictText: (e) => {
        if (!(e instanceof ApiRequestError) || e.code !== "conflict") return null;
        const cur = e.current as { text?: unknown } | null | undefined;
        return typeof cur?.text === "string" ? cur.text : null;
      },
      failed: (label, e) => conn.pushNotice("error", `${label} could not be saved: ${e instanceof Error ? e.message : String(e)}`),
      isLocked: (e) => e instanceof ApiRequestError && e.code === "locked",
    });
    unsubs.push(
      q.subscribe((s) => (status = s)),
      q.onConflict((c) => (conflict = c)),
    );
    return q;
  }

  $effect(() => {
    const id = pres?.id ?? null;
    if (id === queueFor) return;
    for (const u of unsubs.splice(0)) u();
    // Typing in the presentation left behind is still sent.
    queue?.retire();
    queue = id ? makeQueue(id) : null;
    queueFor = id;
    texts = {};
    seconds = {};
    // A translation preview belongs to the presentation left behind: its job is cancelled
    // (Keep would send that presentation's slides here).
    untrack(() => mt.cancel());
    translateOpen = false;
    const all = (untrack(() => pres)?.sections ?? []).flatMap((s) => s.slides ?? []);
    language = presentationLanguage(all);
    dual = !!language;
  });

  // Server second texts → the second column (unless typed there and not saved yet).
  $effect(() => {
    const q = queue;
    const list = blocks;
    const byId = slideById;
    const old = untrack(() => seconds);
    const next: Record<string, string> = {};
    for (const b of list)
      next[b.slideId] = q?.hasPendingSecond(b.slideId) ? (q.pendingSecond(b.slideId) ?? old[b.slideId] ?? "") : secondText(byId.get(b.slideId), b.elementId);
    seconds = next;
  });

  // Server text → text areas (unless the operator has local edits there).
  $effect(() => {
    const q = queue;
    if (!q) return;
    const list = blocks;
    const old = untrack(() => texts);
    const next: Record<string, string> = {};
    for (const b of list) {
      q.setBase(b.slideId, b.text);
      next[b.slideId] = q.hasPending(b.slideId) ? (q.pendingText(b.slideId) ?? old[b.slideId] ?? b.text) : b.text;
    }
    texts = next;
    q.release();
  });

  // Undo waits for typing that has not been sent yet.
  session.reflowFlush = () => queue?.flush() ?? Promise.resolve();

  // The connection is back: retry queued edits now. (Not in `unsubs`: those are the current
  // queue's, dropped whenever another presentation opens.)
  const offStatus = connStatus.subscribe((s) => {
    if (s.phase === "online") queue?.kick();
  });

  onDestroy(() => {
    offStatus();
    for (const u of unsubs.splice(0)) u();
    // Retire, not just flush: typing kept from a lost lock (and conflicts nobody can answer
    // now) are reported with their text, never dropped (D10-1).
    queue?.retire();
    session.reflowFlush = null;
  });

  // ---- editing --------------------------------------------------------------------------------

  // ---- smart paste (SongSelect lyrics) ------------------------------------------------------------

  /** Pasted SongSelect lyrics held until the user picks: a new song, or plain text in the slide. */
  let smartPaste = $state<{ text: string; slideId: string; start: number; end: number } | null>(null);

  function onPaste(b: ReflowBlock, e: ClipboardEvent) {
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (lockedBy || !looksLikeSongSelect(text)) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLTextAreaElement;
    smartPaste = { text, slideId: b.slideId, start: el.selectionStart, end: el.selectionEnd };
  }

  function smartImport() {
    const sp = smartPaste;
    smartPaste = null;
    if (sp) shell.songSelect = { text: sp.text };
  }

  function smartAsText() {
    const sp = smartPaste;
    smartPaste = null;
    const el = sp && editorEl?.querySelector<HTMLTextAreaElement>(`[data-reflow-text="${sp.slideId}"]`);
    if (!sp || !el) return;
    el.focus();
    el.setRangeText(sp.text, sp.start, sp.end, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }

  function onInput(b: ReflowBlock, e: Event) {
    const el = e.currentTarget as HTMLTextAreaElement;
    texts[b.slideId] = el.value;
    active = { id: b.slideId, caret: el.selectionStart };
    queue?.setText(b.slideId, el.value);
  }

  function onSecondInput(b: ReflowBlock, e: Event) {
    const v = (e.currentTarget as HTMLTextAreaElement).value;
    seconds[b.slideId] = v;
    if (queue && pres) queue.setSecond(b.slideId, v, secondBuilder(pres.id, b.slideId));
  }

  /** Builds the op that saves a slide's second text, when it is sent: from the slide as the
   * editor knows it then while its presentation is still open, else as it was when typed
   * (another presentation opened meanwhile: the retired queue still sends it, R3-5). */
  function secondBuilder(pid: string, id: string): (text: string) => PresentationOp | null {
    const typedSlide = slideById.get(id);
    const typedElement = blocks.find((x) => x.slideId === id)?.elementId;
    const typedLanguage = language;
    return (text) => {
      const open = pres?.id === pid;
      const s = open ? slideById.get(id) : typedSlide;
      const el = open ? blocks.find((x) => x.slideId === id)?.elementId : typedElement;
      if (!s || !el) return null;
      return secondTextOp(s, el, open ? language : typedLanguage, text);
    };
  }

  /** A new language code: every slide with a second text takes it. */
  function setLanguage(code: string) {
    const c = code.trim();
    language = c;
    if (!pres || !c) return;
    const ops: PresentationOp[] = (pres.sections ?? [])
      .flatMap((sec) => sec.slides ?? [])
      .filter((sl) => sl.translation && sl.translation.language !== c)
      .map((sl) => ({ type: "setSlideTranslation", slide: sl.id, translation: { ...sl.translation, language: c } }));
    if (ops.length) void runOps("Second Language Code", ops);
  }

  function openTranslate(scope: string[] | null) {
    translateScope = scope;
    translateOpen = true;
  }

  function startTranslate(choice: { from: string; to: string }) {
    if (!pres) return;
    translateOpen = false;
    void mt.start({ presentation: pres.id, to: choice.to, from: choice.from, ...(translateScope ? { slides: translateScope } : {}) });
  }

  /** Keep: the proposals become the slides' second texts (marked as a machine translation). */
  /** Keep: the proposals become the slides' second texts (marked as a machine translation),
   *  for the slides still here; they stay offered if the edit is refused. */
  function keepTranslation() {
    const p = pres;
    if (!p || mt.job?.presentation !== p.id) return;
    void mt.keep(
      (ops, lang) => {
        if (lang && lang !== language) language = lang;
        return runOps("Machine Translation", ops);
      },
      (id) => slideById.has(id),
    );
  }

  onDestroy(() => mt.dispose());

  let pasteSplit = $derived<TwoLanguageSplit>(
    pasteMode === "line" || pasteMode === "inline" ? { type: "separator", separator: pasteMode === "line" ? "---" : pasteSeparator } : { type: "alternating" },
  );
  // Pasted ChordPro: the server reads it with the same split (its chords stay on the first
  // language), a moment after the typing stops.
  let pasteIsChordPro = $derived(looksLikeChordPro(pasteText));
  let pasteChordPro = $state.raw<ReturnType<typeof chordProPaste> | null>(null);
  let pasteError = $state("");
  let pasteRead = 0;
  $effect(() => {
    const text = pasteText;
    const split = chordProSplit(pasteMode, pasteSeparator);
    if (!pasteOpen || !pasteIsChordPro) {
      pasteChordPro = null;
      pasteError = "";
      return;
    }
    const n = ++pasteRead;
    // The last result belongs to the previous text or split: drop it, so the preview and the
    // Paste button wait for this one.
    pasteChordPro = null;
    pasteError = "";
    const t = setTimeout(() => {
      conn.rest
        .parseChordPro({ text, split })
        .then((r) => {
          if (n !== pasteRead) return;
          pasteChordPro = chordProPaste(r.presentation);
          pasteError = "";
        })
        .catch((e) => {
          if (n !== pasteRead) return;
          pasteChordPro = null;
          pasteError = e instanceof Error ? e.message : String(e);
        });
    }, 250);
    return () => clearTimeout(t);
  });
  let pastePairs = $derived(pasteIsChordPro ? (pasteChordPro?.pairs ?? []) : splitTwoLanguages(pasteText, pasteSplit));
  /** The second language's code: the column's, else the one the ChordPro file names. */
  let pasteLanguage = $derived(language || (pasteIsChordPro ? (pasteChordPro?.language ?? "") : ""));
  let pastePlan = $derived(pasteOps(blocks, slideById, pastePairs, pasteLanguage, pastePrimary));

  function applyPaste() {
    const plan = pastePlan;
    const chords = pasteIsChordPro && pasteChordPro ? chordOps(pasteChordPro.chords, plan.used, pastePrimary) : [];
    pasteOpen = false;
    pasteText = "";
    dual = true;
    if (!language && pasteLanguage) language = pasteLanguage;
    if (plan.ops.length) void runOps("Paste Two Languages", [...plan.ops, ...chords]);
    if (plan.leftover) conn.pushNotice("info", `${plan.leftover} pasted ${plan.leftover === 1 ? "slide has" : "slides have"} no slide to go to: add slides and paste again.`);
  }

  /** Sends `ops` through the edit queue; true once they were accepted. */
  async function runOps(label: string, ops: PresentationOp[]): Promise<boolean> {
    const q = queue;
    if (!q || !ops.length) return false;
    try {
      await q.enqueueOps(label, ops);
    } catch {
      /* reported by the queue */
      return false;
    }
    try {
      await session.reopen();
      await tick();
      const f = focusNext;
      focusNext = null;
      if (f) focusBlock(f.index, f.caret);
    } catch {
      /* reported by the queue */
    }
    return true;
  }

  function split(b: ReflowBlock, caret: number) {
    const text = texts[b.slideId] ?? b.text;
    focusNext = { index: b.index + 1, caret: 0 };
    void runOps("Insert Slide Break", splitOps(b.slideId, text, queue?.baseOf(b.slideId), caret));
  }

  function merge(b: ReflowBlock) {
    const prev = blocks[b.index - 1];
    if (!prev) return;
    const prevText = texts[prev.slideId] ?? prev.text;
    focusNext = { index: prev.index, caret: prevText.length + (prevText ? 1 : 0) };
    void runOps("Merge Slides", mergeOps(prev.slideId, b.slideId));
  }

  function onKey(b: ReflowBlock, e: KeyboardEvent) {
    const el = e.currentTarget as HTMLTextAreaElement;
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey && !mod && !e.altKey)) {
      e.preventDefault();
      e.stopPropagation();
      active = { id: b.slideId, caret: el.selectionStart };
      keyMenuAt = Date.now();
      menuFor = b.slideId;
    } else if (mod && e.key === "Enter") {
      e.preventDefault();
      split(b, el.selectionStart);
    } else if (e.key === "Backspace" && !mod && el.selectionStart === 0 && el.selectionEnd === 0 && b.index > 0) {
      e.preventDefault();
      merge(b);
    } else if (e.key === "Tab" && !mod) {
      e.preventDefault();
      focusBlock(b.index + (e.shiftKey ? -1 : 1), "end");
    } else if (mod && !e.altKey && (e.code === "KeyZ" || e.code === "KeyY")) {
      // The server's undo stack, not the text area's own.
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "KeyY" || e.shiftKey) void session.redo();
      else void session.undo();
    }
  }

  /** The browser's context menu that follows the menu key is ours instead; a right click keeps its own (spelling). */
  function onContextMenu(e: MouseEvent) {
    if (Date.now() - keyMenuAt < 800) e.preventDefault();
  }

  function track(b: ReflowBlock, e: Event) {
    const el = e.currentTarget as HTMLTextAreaElement;
    active = { id: b.slideId, caret: el.selectionStart };
  }

  function focusBlock(index: number, caret: number | "end") {
    const el = editorEl?.querySelectorAll<HTMLTextAreaElement>("textarea[data-reflow-text]")[index];
    if (!el) return;
    el.focus();
    const c = caret === "end" ? el.value.length : Math.min(caret, el.value.length);
    el.setSelectionRange(c, c);
    el.scrollIntoView({ block: "nearest" });
  }

  function insertBreak() {
    const a = active;
    const b = (a && blocks.find((x) => x.slideId === a.id)) ?? blocks[blocks.length - 1];
    if (!b) return;
    split(b, a && a.id === b.slideId ? a.caret : (texts[b.slideId] ?? b.text).length);
  }

  async function startGroup(b: ReflowBlock, picked: GroupChoice | "new") {
    if (!pres) return;
    let choice: GroupChoice | undefined = picked === "new" ? undefined : picked;
    if (picked === "new") {
      const name = await shell.askText("New Group", "", "Create", "Group name");
      if (!name) return focusBlock(b.index, active?.id === b.slideId ? active.caret : "end");
      const known = globalGroups.find((g) => g.name.toLowerCase() === name.toLowerCase());
      choice = known ?? { name, color: DEFAULT_GROUPS[(groups.length + 3) % DEFAULT_GROUPS.length]!.color };
    }
    if (!choice || !pres) return;
    focusNext = { index: b.index, caret: "end" };
    await runOps("Set Group", groupOps(pres, b.slideId, choice, uuid));
  }

  // ---- the "…" menu ---------------------------------------------------------------------------

  function menuEntries(b: ReflowBlock): SlideMenuEntry[] {
    return slideMenuEntries({
      choices,
      startsGroup: b.sectionStart ? b.sectionName : null,
      isFirst: b.index === 0,
      canContinue: !!pres && canContinueAbove(pres, b.slideId),
      hasCue: !!slideBadges(slideById.get(b.slideId), undefined, false).cue,
      translate: canTranslate && !lockedBy,
    });
  }

  function openMenu(b: ReflowBlock) {
    menuFor = menuFor === b.slideId ? null : b.slideId;
  }

  /** Closes the menu and puts the focus back in the slide's text (at its caret). */
  function closeMenu(b: ReflowBlock) {
    menuFor = null;
    void tick().then(() => focusBlock(b.index, active?.id === b.slideId ? active.caret : "end"));
  }

  function onMenu(b: ReflowBlock, e: SlideMenuEntry) {
    menuFor = null;
    if (e.group) return void startGroup(b, e.group);
    const p = pres;
    if (!p) return;
    switch (e.action) {
      case "newGroup":
        return void startGroup(b, "new");
      case "continue":
        focusNext = { index: b.index, caret: "end" };
        return void runOps("Continue Group", continueGroupOps(p, b.slideId));
      case "break":
        return split(b, active?.id === b.slideId ? active.caret : (texts[b.slideId] ?? b.text).length);
      case "merge":
        return merge(b);
      case "removeCue":
        focusNext = { index: b.index, caret: "end" };
        return void runOps("Remove Media Cue", removeCueOps(slideById.get(b.slideId)));
      case "translate":
        return void openTranslate([b.slideId]);
      case "delete":
        focusNext = { index: Math.max(0, Math.min(b.index, blocks.length - 2)), caret: "end" };
        return void runOps("Delete Slide", deleteSlideOps(p, b.slideId));
    }
  }

  function addSlide() {
    if (!pres) return;
    const sections = pres.sections ?? [];
    const last = sections[sections.length - 1];
    focusNext = { index: blocks.length, caret: 0 };
    if (!last) {
      const id = uuid();
      void runOps("New Slide", [
        { type: "addSection", section: { id, name: "Group" } },
        { type: "addSlide", section: id },
      ]);
      return;
    }
    const lastSlide = (last.slides ?? [])[(last.slides ?? []).length - 1];
    void runOps("New Slide", [{ type: "addSlide", section: last.id, ...(lastSlide ? { after: lastSlide.id } : {}) }]);
  }

  function resolve(choice: "mine" | "theirs") {
    const c = conflict;
    if (!c || !queue) return;
    queue.resolve(c.key, choice);
    if (choice === "theirs") texts[c.key] = c.theirs;
  }

  // ---- thumbnails -----------------------------------------------------------------------------

  let dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
  let aspect = $derived(aspectRatio(pres));
  let thumbW = $derived(thumbWidthFor(thumbSize, dpr));

  // The version in the URL: an edited slide loads its new thumbnail at once, and so do the
  // slides after a changed media cue (a thumbnail is drawn over the latest one before it).
  let versions = $derived(pres ? thumbVersions(pres) : new Map<string, string>());

  function thumbSrc(id: string): string {
    const s = slideById.get(id);
    if (!pres || !s) return "";
    return `${conn.rest.thumbnailUrl(pres.id, id, thumbW)}&v=${versions.get(id) ?? slideHash(s)}`;
  }

  function pickThumb(b: ReflowBlock) {
    focusBlock(b.index, "end");
  }

  /** A group's colour bar and thumbnail bar: its colour, neutral when it has none. */
  function bandColor(color: string | null, name: string): string {
    return color ?? (name ? "var(--group-neutral)" : "transparent");
  }

  let groupCount = $derived(groups.filter((g) => g.name).length);
  let summary = $derived(`${blocks.length} ${blocks.length === 1 ? "slide" : "slides"} · ${groupCount} ${groupCount === 1 ? "group" : "groups"}`);

  // ---- presence and the edit lock (G8) ----------------------------------------------------------

  $effect(() => {
    const id = pres?.id ?? null;
    presence.setOpen("reflow", id ? [{ kind: "presentation", id, place: "reflow" }] : null);
  });
  onDestroy(() => presence.setOpen("reflow", null));
  /** Someone else holds the lock: the text is read-only. */
  let lockedBy = $derived(presence.lockedBy("presentation", pres?.id));
  // Nobody else holds it any more (they closed it, or this client took it back): send the kept
  // typing.
  $effect(() => {
    if (!lockedBy && status === "locked") untrack(() => queue?.unlock());
  });
  /** Typing refused because someone else took over (kept, offered back). */
  let kept = $derived(status === "locked" ? ((queue as EditQueue | null)?.lockedTexts() ?? []) : []);

  async function copyKept() {
    const q = queue;
    if (!q) return;
    const list = q.lockedTexts();
    await copyTyping(list.map((t) => t.text).join("\n\n"));
    q.discardLocked();
    const next = { ...texts };
    const nextSecond = { ...seconds };
    for (const t of list) {
      if (t.second) {
        const b = blocks.find((x) => x.slideId === t.key);
        nextSecond[t.key] = secondText(slideById.get(t.key), b?.elementId ?? null);
      } else next[t.key] = q.baseOf(t.key) ?? next[t.key] ?? "";
    }
    texts = next;
    seconds = nextSecond;
  }

  let offline = $derived($connStatus.phase !== "online");
  let statusText = $derived(
    status === "locked" ? "Not saved: someone else is editing" : status === "conflict" ? "Conflict" : status === "offline" || (offline && status === "saving") ? "Offline: edits queued" : offline ? "Offline" : status === "saving" ? "Saving…" : "Saved",
  );
</script>
<section class="reflow" aria-label="Reflow editor">
  <PageToolbar title={pres?.name ?? "Reflow"} ondone={() => shell.close()}>
    {#if pres}
      <PresenceBadge kind="presentation" id={pres.id} />
      <span class="summary">{summary}</span>
      <span class="save" class:saving={status === "saving"} class:off={status === "offline" || offline} class:bad={status === "conflict" || status === "locked"} data-save-status={status} role="status">
        {#if status === "locked"}<Lock size={14} />{:else if status === "conflict"}<TriangleAlert size={14} />{:else if status === "offline" || offline}<CloudOff size={14} />{:else if status === "saving"}<Loader size={14} class="spin" />{:else}<Check size={14} />{/if}
        {statusText}
      </span>
      <button type="button" class="tbtn" class:on={dual} aria-pressed={dual} onclick={() => (dual = !dual)} title="Show each slide's second-language text beside it"><Languages size={15} strokeWidth={1.8} aria-hidden="true" />Second language</button>
      {#if dual}
        <input class="lang" type="text" list="reflow-langs" value={language} placeholder="es" aria-label="Second language code" title="Language code of the second text (BCP 47, e.g. es, pt-BR)" spellcheck="false" onchange={(e) => setLanguage(e.currentTarget.value)} />
        <datalist id="reflow-langs">{#each COMMON_LANGUAGES as l (l.code)}<option value={l.code}>{l.name}</option>{/each}</datalist>
        <button type="button" class="tbtn" onclick={() => (pasteOpen = true)} disabled={!!lockedBy}><ClipboardPaste size={15} strokeWidth={1.8} aria-hidden="true" />Paste two languages…</button>
        {#if canTranslate}
          <button type="button" class="tbtn" onclick={() => void openTranslate(null)} disabled={!!lockedBy || proposing} title="Machine-translate the slides into a second language (offline): a preview you keep or discard"><Sparkles size={15} strokeWidth={1.8} aria-hidden="true" />Translate to…</button>
        {/if}
      {/if}
      <span class="size" title="Text size"><TypeIcon size={16} strokeWidth={1.8} aria-hidden="true" /><Slider bind:value={textZoom} min={12} max={26} label="Text size" width={110} /></span>
      <span class="size" title="Thumbnail size"><ImageIcon size={16} strokeWidth={1.8} aria-hidden="true" /><Slider bind:value={thumbSize} min={120} max={320} step={10} label="Thumbnail size" width={110} /></span>
    {/if}
  </PageToolbar>
  {#if pres}
    <LockBanner kind="presentation" id={pres.id} noun="presentation" kept={kept.length} oncopy={copyKept} ontaken={() => queue?.unlock()} />
  {/if}
  {#if pres}
    <TranslateBanner preview={mt} locked={!!lockedBy} onkeep={keepTranslation} />
  {/if}
  {#if pres && smartPaste}
    <div class="smartpaste" role="status" data-testid="reflow-songselect">
      <ClipboardPaste size={16} strokeWidth={1.8} aria-hidden="true" />
      <span class="sptext">This looks like a song from SongSelect. Make it a new song with its title, sections, writers and CCLI number?</span>
      <button type="button" class="tbtn" onclick={smartImport} data-testid="reflow-songselect-import">Import as New Song…</button>
      <button type="button" class="tbtn" onclick={smartAsText}>Paste as Text</button>
      <button type="button" class="tbtn" onclick={() => (smartPaste = null)}>Cancel</button>
    </div>
  {/if}

  {#if !pres}
    <div class="empty">
      {#if session.openLoading}<Loader size={18} class="spin" />{:else}Select a presentation in the library (or a playlist) to edit its words here.{/if}
    </div>
  {:else}
    <div class="body">
      <section class="texts" aria-label="Slide text">
        <div class="editor" class:ro={!!lockedBy} bind:this={editorEl} style:--ed-font={`${textZoom}px`}>
          {#each groups as g (g.sectionId + g.blocks[0]!.slideId)}
            {@const sec = sectionById.get(g.sectionId)}
            <div class="band" style:--gc={bandColor(g.color, g.name)} data-band={g.sectionId}>
              <div class="glabel" title={g.name || undefined}><span>{g.name}</span></div>
              <div class="gbar"></div>
              <div class="gblocks">
                {#each g.blocks as b (b.slideId)}
                  {@const badges = slideBadges(slideById.get(b.slideId), sec, b.sectionStart)}
                  {@const open = menuFor === b.slideId}
                  <div class="block" class:selected={active?.id === b.slideId} class:live={$liveSlideId === b.slideId} data-block={b.slideId}>
                    <div class="bhead">
                      <span class="num">{b.index + 1}.</span>
                      <span class="cue" title={badges.cue ? `Media cue: ${badges.cue}` : undefined}>{#if badges.cue}<Film size={12} strokeWidth={1.8} aria-hidden="true" />{badges.cue}{#if badges.cueCount > 1}<span class="more"> +{badges.cueCount - 1}</span>{/if}{/if}</span>
                      {#if badges.hotkey}<span class="hk" title={`Hotkey ${badges.hotkey}`} aria-label={`Hotkey ${badges.hotkey}`}>{badges.hotkey}</span>{/if}
                      <button
                        type="button"
                        class="more-btn"
                        class:open
                        bind:this={menuButtons[b.slideId]}
                        aria-label={`Options for slide ${b.index + 1}`}
                        aria-haspopup="menu"
                        aria-expanded={open}
                        title="Slide options (menu key or Shift+F10 in the text)"
                        onclick={() => openMenu(b)}
                      ><Ellipsis size={16} strokeWidth={1.8} /></button>
                    </div>
                    <div class="pair" class:dual>
                    <textarea
                      class="text"
                      rows="1"
                      spellcheck="true"
                      value={texts[b.slideId] ?? b.text}
                      readonly={!!lockedBy}
                      data-reflow-text={b.slideId}
                      aria-label={`Slide ${b.index + 1} text${b.sectionName ? `, ${b.sectionName}` : ""}`}
                      oninput={(e) => onInput(b, e)}
                      onpaste={(e) => onPaste(b, e)}
                      onkeydown={(e) => onKey(b, e)}
                      onkeyup={(e) => track(b, e)}
                      onclick={(e) => track(b, e)}
                      onfocus={(e) => track(b, e)}
                      oncontextmenu={onContextMenu}
                    ></textarea>
                    {#if dual && proposals[b.slideId]}
                      <textarea
                        class="text second machine"
                        rows="1"
                        readonly
                        lang={job?.to || language || undefined}
                        value={proposedText(proposals[b.slideId], b.elementId)}
                        title={MACHINE_NOTICE}
                        data-reflow-proposal={b.slideId}
                        aria-label={`Slide ${b.index + 1} machine translation (not saved)`}
                      ></textarea>
                    {:else if dual}
                      <div class="secondwrap">
                      {#if isMachineTranslation(slideById.get(b.slideId))}<span class="mttag" title="Machine translation, not checked yet: editing it clears this mark">MT</span>{/if}
                      <textarea
                        class="text second"
                        rows="1"
                        spellcheck="true"
                        lang={language || undefined}
                        value={seconds[b.slideId] ?? ""}
                        readonly={!!lockedBy || !b.elementId}
                        placeholder={b.elementId ? `Second language${language ? ` (${language})` : ""}` : "Type the slide's text first"}
                        data-reflow-second={b.slideId}
                        aria-label={`Slide ${b.index + 1} second-language text`}
                        oninput={(e) => onSecondInput(b, e)}
                        onfocus={(e) => track(b, e)}
                      ></textarea>
                      </div>
                    {/if}
                    </div>
                    {#if open}
                      <SlideMenu
                        entries={menuEntries(b)}
                        label={`Options for slide ${b.index + 1}`}
                        anchor={menuButtons[b.slideId] ?? null}
                        onselect={(e) => onMenu(b, e)}
                        onclose={() => closeMenu(b)}
                      />
                    {/if}
                  </div>
                {/each}
              </div>
            </div>
          {/each}
          <button type="button" class="add" onclick={addSlide}>+ New slide</button>
        </div>
      </section>

      <section class="thumbs" aria-label="Slide thumbnails">
        <div class="tgrid" style:--tw={`${thumbSize}px`}>
          {#each blocks as b (b.slideId)}
            {@const src = thumbSrc(b.slideId)}
            {@const badges = slideBadges(slideById.get(b.slideId), sectionById.get(b.sectionId), b.sectionStart)}
            <button
              type="button"
              class="thumb"
              class:selected={active?.id === b.slideId}
              class:live={$liveSlideId === b.slideId}
              style:--gc={bandColor(b.color, b.sectionName)}
              onclick={() => pickThumb(b)}
              aria-label={`Slide ${b.index + 1}${b.sectionName ? `, ${b.sectionName}` : ""}${badges.cue ? `, media cue ${badges.cue}` : ""}`}
              data-thumb={b.slideId}
            >
              <div class="frame" style:aspect-ratio={aspect}>
                {#if src}{#key src}<img {src} alt="" draggable="false" decoding="async" data-thumb-src={src} />{/key}{/if}
                {#if badges.cue || badges.hotkey}
                  <span class="tbadges">
                    {#if badges.cue}<span class="tcue" title={`Media cue: ${badges.cue}`}><Film size={11} strokeWidth={2} /></span>{/if}
                    {#if badges.hotkey}<span class="hk">{badges.hotkey}</span>{/if}
                  </span>
                {/if}
              </div>
              <div class="bar" class:plain={!b.sectionName && !b.color}>
                <span class="n">{b.index + 1}</span><span class="g">{badges.cue || (b.sectionStart ? b.sectionName : "")}</span>
              </div>
            </button>
          {/each}
        </div>
      </section>
    </div>

    <footer class="foot">
      <button type="button" class="brk" onclick={insertBreak} title="Splits the slide at the text cursor (Ctrl+Enter)">Insert Slide Break</button>
      <span class="hint">Ctrl+Enter splits · Backspace at the start of a slide merges it up · Tab: next slide · menu key or Shift+F10: slide options</span>
    </footer>

    {#if pasteOpen}
      <!-- A page-local dialog: no key in it reaches the show keys (Space, arrows, F1–F6), and
           Esc closes it (H11-6). -->
      <div class="pastewrap" role="presentation" onkeydown={(e) => e.stopPropagation()}>
      <Dialog title="Paste two languages" width={620} escape onclose={() => (pasteOpen = false)}>
        <div class="paste">
          <p class="phint">Paste lyrics with both languages, as text or ChordPro (its chords stay on the first language). A blank line starts the next slide; the slides fill this presentation's slides in order.</p>
          <div class="pmodes" role="radiogroup" aria-label="How the languages are separated">
            <label><input type="radio" bind:group={pasteMode} value="alternating" /> Lines alternate (first, second, first, …)</label>
            <label><input type="radio" bind:group={pasteMode} value="line" /> A line of <code>---</code> between the languages</label>
            <label><input type="radio" bind:group={pasteMode} value="inline" /> In each line, separated by <input class="sep" type="text" bind:value={pasteSeparator} aria-label="Separator" maxlength="5" /></label>
            {#if pasteIsChordPro}
              <label><input type="radio" bind:group={pasteMode} value="comments" /> ChordPro: the second language in italic comments, <code>{"{ci: …}"}</code> under the lines</label>
            {/if}
          </div>
          <textarea class="ptext" rows="9" bind:value={pasteText} placeholder={"Morning light is breaking\nLa luz del alba llega\n\nSing the song of morning\nCanta la canción"} aria-label="Pasted text" data-testid="paste-two-languages"></textarea>
          <label class="pprimary"><input type="checkbox" bind:checked={pastePrimary} /> Also replace each slide's first-language text</label>
          {#if pasteIsChordPro}
            <p class="phint" data-testid="paste-chordpro">{pasteError ? `The ChordPro text could not be read: ${pasteError}` : pasteChordPro ? `ChordPro: ${pasteChordPro.pairs.length} ${pasteChordPro.pairs.length === 1 ? "slide" : "slides"}, ${pasteChordPro.chords.filter((c) => c).length} with chords${pasteChordPro.language ? `, second language ${pasteChordPro.language}` : ""}${pastePrimary ? "" : " (chords are added only when the first-language text is replaced too)"}.` : "Reading the ChordPro text…"}</p>
          {/if}
          {#if pastePairs.length}
            <div class="ppreview" aria-label="Preview">
              {#each pastePairs.slice(0, 4) as [a, b2], i (i)}
                <div class="prow"><span class="pn">{i + 1}.</span><span class="pa">{a}</span><span class="pb">{b2 || "—"}</span></div>
              {/each}
              {#if pastePairs.length > 4}<div class="pmore">… {pastePairs.length - 4} more</div>{/if}
            </div>
            <p class="pcount">{pastePlan.used} of {blocks.length} slides filled{pastePlan.leftover ? ` · ${pastePlan.leftover} pasted ${pastePlan.leftover === 1 ? "slide is" : "slides are"} left over` : ""}{pasteLanguage ? "" : " · set the language code first"}</p>
          {/if}
        </div>
        {#snippet footer()}
          <button type="button" class="brk" onclick={() => (pasteOpen = false)}>Cancel</button>
          <button type="button" class="brk primary" disabled={!pastePlan.ops.length} onclick={applyPaste}>Paste</button>
        {/snippet}
      </Dialog>
      </div>
    {/if}

    {#if translateOpen}
      <TranslateDialog
        {conn}
        presentation={pres}
        title={translateScope ? "Translate this slide" : "Translate to another language"}
        what={translateScope ? "The slide's text" : `The text of all ${blocks.length} slides`}
        {language}
        onstart={startTranslate}
        onclose={() => (translateOpen = false)}
      />
    {/if}

    {#if conflict}
      {@const c = conflict}
      {@const n = blocks.findIndex((x) => x.slideId === c.key)}
      <ConflictDialog
        title="This slide was changed by someone else"
        message={`Slide ${n + 1} changed on the server while you were typing. Choose which text to keep.`}
        theirs={c.theirs}
        mine={c.mine}
        onTheirs={() => resolve("theirs")}
        onMine={() => resolve("mine")}
      />
    {/if}
  {/if}
</section>

<style>
  .smartpaste {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--line);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-panel));
    font-size: 13px;
  }
  .sptext {
    flex: 1 1 320px;
  }
  /* Someone else is editing (G8): the text is read-only. */
  .editor.ro textarea {
    opacity: 0.72;
    cursor: default;
  }
  .reflow {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-app);
  }
  .summary {
    color: var(--text-meta);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .tbtn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .tbtn.on {
    border-color: var(--accent, #8b7cf6);
    color: var(--accent-text, #c9c1ff);
  }
  .tbtn:disabled {
    opacity: 0.5;
  }
  .lang {
    width: 64px;
    padding: 3px 6px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-sunken, var(--surface-panel));
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .pair.dual {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px;
  }
  .text.second {
    font-style: normal;
    background: #e9e4d2;
    color: #4a3f1f;
  }
  .text.second::placeholder {
    color: #8b8572;
  }
  /* A machine translation shown before it is kept: dashed, tinted, read-only. */
  .text.second.machine {
    background: repeating-linear-gradient(135deg, #e6eef9 0 10px, #dde7f6 10px 20px);
    color: #1f3554;
    font-style: italic;
    outline: 2px dashed #5b84c4;
    outline-offset: -2px;
  }
  .secondwrap {
    position: relative;
    display: flex;
  }
  .secondwrap > .text {
    flex: 1;
  }
  .mttag {
    position: absolute;
    top: 3px;
    right: 4px;
    padding: 0 4px;
    border-radius: 3px;
    background: #5b84c4;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    pointer-events: auto;
    z-index: 1;
  }
  .pastewrap {
    display: contents;
  }
  .paste {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .phint,
  .pcount {
    margin: 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .pmodes {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-sm);
  }
  .pmodes label,
  .pprimary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  .sep {
    width: 44px;
    padding: 1px 4px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-sunken, var(--surface-panel));
    color: var(--text);
  }
  .ptext {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    padding: 6px 8px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-sunken, var(--surface-panel));
    color: var(--text);
    font: inherit;
  }
  .ppreview {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 170px;
    overflow: auto;
    padding: 6px 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    font-size: var(--fs-sm);
  }
  .prow {
    display: grid;
    grid-template-columns: 22px 1fr 1fr;
    gap: 8px;
    white-space: pre-line;
  }
  .pn,
  .pmore {
    color: var(--text-meta);
  }
  .pb {
    font-style: italic;
    color: var(--text-label);
  }
  .brk.primary {
    background: var(--accent, #8b7cf6);
    border-color: var(--accent, #8b7cf6);
    color: #fff;
  }
  .brk:disabled {
    opacity: 0.5;
  }
  .size {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
  }
  .save {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 10px;
    font-size: var(--fs-sm);
    color: var(--ok-text);
    background: var(--ok-bg);
    white-space: nowrap;
  }
  .save.saving {
    color: var(--text);
    background: var(--accent-sel);
  }
  .save.off {
    color: var(--live);
    background: var(--live-soft);
  }
  .save.bad {
    color: var(--danger-text);
    background: var(--danger-soft);
  }
  .empty {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--text-meta);
    font-style: italic;
  }
  .body {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
  }
  .texts {
    flex: 1 1 420px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--line);
    background: var(--surface-sunk);
  }
  .editor {
    flex: 1 1 0;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 12px 16px 4px;
  }
  .band {
    display: flex;
    align-items: stretch;
  }
  .glabel {
    flex: 0 0 22px;
    display: flex;
    justify-content: center;
  }
  .glabel span {
    position: sticky;
    top: 4px;
    align-self: flex-start;
    writing-mode: vertical-rl;
    transform: rotate(180deg);
    padding: 6px 0;
    font-size: var(--fs-xs);
    font-weight: 700;
    color: var(--text);
    white-space: nowrap;
  }
  .gbar {
    flex: 0 0 3px;
    background: var(--gc);
  }
  .gblocks {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .block {
    position: relative;
  }
  .bhead {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 4px 0 8px;
    background: var(--surface-panel);
  }
  .num {
    flex: none;
    min-width: 26px;
    color: var(--text-label);
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .block.live .num {
    color: var(--live);
  }
  .cue {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cue .more {
    color: var(--text-meta);
  }
  .hk {
    flex: none;
    min-width: 16px;
    height: 16px;
    padding: 0 3px;
    box-sizing: border-box;
    border-radius: var(--r-xs);
    background: var(--hotkey);
    color: #1a1030;
    font-size: 10px;
    font-weight: 800;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    white-space: nowrap;
  }
  .more-btn {
    flex: none;
    width: 24px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text);
  }
  .more-btn:hover,
  .more-btn.open {
    background: var(--accent-sel);
  }
  .more-btn:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: -2px;
  }
  /* The canvas's fixed text style: serif italic capitals on a light box (not the presentation's font). */
  .text {
    display: block;
    width: 100%;
    box-sizing: border-box;
    min-height: 1.9em;
    field-sizing: content;
    resize: none;
    margin: 0;
    padding: 4px 12px;
    border: 0;
    border-radius: 0;
    background: #dcdde2;
    color: #23252b;
    font-family: "Source Serif 4", Georgia, "Times New Roman", "DejaVu Serif", serif;
    font-style: italic;
    font-weight: 500;
    font-size: var(--ed-font);
    line-height: 1.35;
    text-transform: uppercase;
    text-align: center;
    user-select: text;
    overflow: hidden;
    outline: 2px solid transparent;
    outline-offset: -2px;
  }
  .block.selected .text {
    background: #e8e5ff;
    outline-color: #8b7cf6;
  }
  .text:focus {
    outline-color: #8b7cf6;
  }
  .add {
    display: block;
    margin: 10px 0 0 29px;
    width: calc(100% - 29px);
    padding: 8px;
    border: 1px dashed var(--line-strong);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-label);
  }
  .add:hover {
    background: var(--surface-panel);
    color: var(--text);
  }
  .thumbs {
    flex: 1 1 480px;
    min-width: 0;
    overflow-y: auto;
    padding: 10px 12px 16px;
    background: var(--surface-app);
  }
  .tgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, min(var(--tw), 100%));
    justify-content: start;
    gap: 12px;
  }
  .thumb {
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 2px solid var(--line);
    border-radius: 4px;
    background: var(--surface-well);
    overflow: hidden;
    text-align: left;
    color: var(--text);
  }
  .thumb.selected {
    border-color: var(--accent-ring);
  }
  .thumb.live:not(.selected) {
    border-color: var(--live);
  }
  .thumb:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: 1px;
  }
  .frame {
    position: relative;
    width: 100%;
    background: repeating-conic-gradient(#1d1f24 0% 25%, #15161a 0% 50%) 0 0 / 14px 14px;
    overflow: hidden;
  }
  .frame img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .tbadges {
    position: absolute;
    left: 5px;
    top: 5px;
    display: flex;
    gap: 3px;
  }
  .tcue {
    width: 16px;
    height: 16px;
    border-radius: var(--r-xs);
    background: #000;
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 16px;
    padding: 3px 6px;
    background: var(--gc);
    color: var(--text-strong);
    font-size: 11.5px;
    font-weight: 600;
  }
  .bar.plain {
    background: var(--group-neutral);
    color: var(--text);
  }
  .g {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .foot {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    background: var(--surface-panel);
    border-top: 1px solid var(--line);
  }
  .brk {
    flex: none;
    white-space: nowrap;
    padding: 5px 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .brk:hover {
    background: var(--surface-overlay);
  }
  .hint {
    min-width: 0;
    color: var(--text-meta);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
