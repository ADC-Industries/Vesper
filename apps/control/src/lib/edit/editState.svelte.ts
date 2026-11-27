// State of the Edit view (one instance while the view is open): the slide being edited, the
// selection, the canvas view, toggles, and the edit pipeline. Every change goes to the server
// as ops (one batch = one undo step); the view then shows the server's document and render.

import type { Element, FontList, Presentation, PresentationOp, Slide, TextBox, TextRange } from "$protocol";
import { ApiRequestError } from "../server";
import { session } from "../state/session.svelte";
import { lockedTypingNotice } from "../state/presenceLogic";
import type { Guide, Rect, View } from "./geometry";
import type { ElementClipboard } from "./elements";
import { allSlides, saveTextOps, textStyleOps } from "./ops";
import type { CharStyle, ParagraphStyle } from "$protocol";

export type InspectorTab = "presentation" | "slide" | "actions" | "build" | "shape" | "text" | "arrange";

/** The tabs that belong to a selection (they give way to Slide when it is gone). */
const OBJECT_TABS: InspectorTab[] = ["shape", "text", "arrange"];

/** A text save refused because someone else changed the text (the Reflow conflict dialog). */
export interface TextConflict {
  slide: string;
  element: string;
  mine: string;
  theirs: string;
  /** Sends the operator's version anyway. */
  keepMine: () => Promise<void>;
}

/** An edit with typing refused because someone else holds the edit lock (G8): kept, offered
 *  back (take editing back to send it, or copy the text). */
export interface LockedEdit {
  label: string;
  ops: PresentationOp[];
  presentation: string;
  /** The typing in it. */
  text: string;
}

/** The typing a batch of ops carries (a text save), or null. */
export function typedText(ops: PresentationOp[]): string | null {
  const box = ops.find((o) => o.type === "replaceTextBox");
  if (box && box.type === "replaceTextBox") return (box.textBox.paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join("")).join("\n");
  const set = ops.find((o) => o.type === "setSlideText");
  return set && set.type === "setSlideText" ? set.text : null;
}

/** The in-place text editor's hooks (set by the canvas while a text box is being edited). */
export interface TextEditorHooks {
  /** The edited text box when it differs from the server's, else null. */
  draft(): TextBox | null;
  /** The element's plain text when editing started (the save's precondition). */
  before(): string;
  /** Rebuild the editor from the server's text box (after a save), keeping the selection. */
  reload(): void;
}

/** The app's element clipboard (in memory; survives switching slides and presentations). */
let clipboard: ElementClipboard | null = null;
export function getClipboard(): ElementClipboard | null {
  return clipboard;
}
export function setClipboard(c: ElementClipboard | null) {
  clipboard = c;
}

export class EditState {
  slideId = $state<string | null>(null);
  /** Selected element IDs (in selection order). */
  selection = $state<string[]>([]);
  view = $state<View>({ zoom: 0.5, panX: 0, panY: 0 });
  /** "fit" and "width" keep fitting when the viewport changes; "manual" after zoom / pan. */
  viewMode = $state<"fit" | "width" | "manual">("fit");
  checker = $state(false);
  /** Rulers along the canvas (the formatting bar's toggle). */
  rulers = $state(true);
  grid = $state(false);
  gridSize = $state(40);
  snap = $state(true);
  /** Pointer position on the canvas, model px (status bar). */
  pointer = $state<{ x: number; y: number } | null>(null);
  guides = $state.raw<Guide[]>([]);
  /** Bounds shown while dragging / resizing (and until the server's document catches up). */
  preview = $state.raw<Map<string, Rect> | null>(null);
  /** The element whose text is being edited in place. */
  editingText = $state<string | null>(null);
  /** Selected characters in the in-place editor (code points), or null. */
  textRange = $state.raw<TextRange | null>(null);
  textHooks: TextEditorHooks | null = null;
  /** Saves and closes the in-place text editor (set by the canvas). */
  finishText: (() => Promise<void>) | null = null;
  info = $state<string | null>(null);
  tab = $state<InspectorTab>("presentation");
  /** The Build tab's step preview: the canvas shows the slide settled at this build step (null: every element). */
  buildPreview = $state<number | null>(null);
  notesOpen = $state(false);
  /** The canvas draws the text's chords (batch 16; the editor's picture only). */
  showChords = $state(false);
  fonts = $state.raw<FontList | null>(null);
  conflict = $state.raw<TextConflict | null>(null);
  /** Typing refused by someone else's edit lock, kept until sent or copied (G8). */
  lockedEdits = $state.raw<LockedEdit[]>([]);
  busy = $state(false);
  /** Elements this client deleted (their disappearance is not news). */
  private deletedHere = new Set<string>();
  private chain: Promise<unknown> = Promise.resolve();
  /** The editor closed (reportLocked ran): typing refused after that is reported at once (E10-1). */
  private closed = false;
  private infoTimer: ReturnType<typeof setTimeout> | null = null;

  get presentation(): Presentation | null {
    return session.open?.presentation ?? null;
  }

  get slide(): Slide | null {
    const id = this.slideId;
    return allSlides(this.presentation).find((s) => s.id === id) ?? null;
  }

  get elements(): Element[] {
    return this.slide?.elements ?? [];
  }

  get selected(): Element[] {
    const els = this.elements;
    return this.selection.map((id) => els.find((e) => e.id === id)).filter((e): e is Element => !!e);
  }

  /** The single selected element, if exactly one. */
  get single(): Element | null {
    const s = this.selected;
    return s.length === 1 ? s[0]! : null;
  }

  canvasSize() {
    const c = this.presentation?.canvasSize;
    return { width: c?.width ?? 1920, height: c?.height ?? 1080 };
  }

  showInfo(msg: string) {
    this.info = msg;
    if (this.infoTimer) clearTimeout(this.infoTimer);
    this.infoTimer = setTimeout(() => (this.info = null), 6000);
  }

  select(ids: string[]) {
    this.selection = ids;
    if (ids.length) {
      const one = ids.length === 1 ? this.elements.find((e) => e.id === ids[0]) : null;
      // Several objects: the Arrange tab (align, distribute, match size, order).
      // The Build tab stays (its rows select their objects).
      if (this.tab === "build") return;
      if (ids.length > 1) this.tab = "arrange";
      else if (this.tab !== "shape" && this.tab !== "text") this.tab = one?.kind?.type === "text" ? "text" : "shape";
      if (this.tab === "text" && !this.selected.some((e) => e.kind?.type === "text")) this.tab = "shape";
    } else if (OBJECT_TABS.includes(this.tab)) {
      this.tab = "slide";
    }
  }

  /** Another document opened: no slide, selection or in-place editing; no step preview (D10-7). */
  startOver() {
    this.slideId = null;
    this.selection = [];
    this.editingText = null;
    this.buildPreview = null;
  }

  selectSlide(id: string | null) {
    if (id === this.slideId) return;
    this.slideId = id;
    this.selection = [];
    this.editingText = null;
    this.textRange = null;
    this.buildPreview = null;
    if (OBJECT_TABS.includes(this.tab)) this.tab = "slide";
  }

  /**
   * After a new document: drops selected elements that are gone (quietly, with an info line
   * when someone else deleted them) and keeps a slide selected.
   */
  reconcile(p: Presentation | null) {
    const slides = allSlides(p);
    if (!slides.length) {
      this.slideId = null;
      this.selection = [];
      this.buildPreview = null;
      return;
    }
    if (!this.slideId || !slides.some((s) => s.id === this.slideId)) {
      const was = this.slideId;
      this.slideId = slides[0]!.id;
      this.selection = [];
      // The step preview was the old slide's (D10-7).
      this.buildPreview = null;
      if (was) this.showInfo("The slide you were editing was deleted.");
      return;
    }
    const ids = new Set(this.elements.map((e) => e.id));
    const gone = this.selection.filter((id) => !ids.has(id));
    if (gone.length) {
      this.selection = this.selection.filter((id) => ids.has(id));
      if (this.editingText && !ids.has(this.editingText)) this.editingText = null;
      const elsewhere = gone.filter((id) => !this.deletedHere.has(id));
      if (elsewhere.length) this.showInfo(elsewhere.length === 1 ? "The selected object was deleted by another operator." : "Selected objects were deleted by another operator.");
      if (!this.selection.length && OBJECT_TABS.includes(this.tab)) this.tab = "slide";
    }
    for (const id of gone) this.deletedHere.delete(id);
  }

  markDeleted(ids: string[]) {
    for (const id of ids) this.deletedHere.add(id);
  }

  /**
   * Sends ops as one batch (one undo step), one batch at a time in order, then refetches the
   * document. Failures become a toast; a text conflict opens the conflict dialog.
   */
  run(label: string, ops: PresentationOp[], opts: { keepPreview?: boolean; presentation?: string } = {}): Promise<boolean> {
    const p = this.presentation;
    const editor = session.editor;
    if (!p || !editor || !ops.length) {
      if (!opts.keepPreview) this.preview = null;
      return Promise.resolve(false);
    }
    // `presentation`: the one the ops were made for (typing saved after another one opened).
    const id = opts.presentation ?? p.id;
    const job = this.chain.then(async () => {
      this.busy = true;
      try {
        await editor.editPresentation(id, label, ops);
        await session.reopen();
        return true;
      } catch (e) {
        if (e instanceof ApiRequestError && e.code === "locked" && typedText(ops) !== null) {
          // Someone else took over editing: the typing is kept and offered back. After the
          // editor closed (in-place typing saved on close lands here later), nothing would
          // offer it, so it's reported with its text at once (E10-1).
          this.lockedEdits = [...this.lockedEdits, { label, ops, presentation: id, text: typedText(ops)! }];
          if (this.closed) this.reportLocked();
        } else if (!this.conflictFrom(e, ops, label, id)) {
          const msg = e instanceof Error ? e.message : String(e);
          session.conn?.pushNotice("error", `${label} failed: ${msg}`);
        }
        await session.reopen();
        return false;
      } finally {
        this.busy = false;
        // A drag preview stays until the canvas has the new render.
        if (!opts.keepPreview) this.preview = null;
        this.guides = [];
      }
    });
    this.chain = job.catch(() => {});
    return job;
  }

  /**
   * An edit of a text element's text settings. While its text is being edited in place, the
   * typing so far is saved in the same batch first, and the editor reloads afterwards.
   */
  async editText(el: Element, label: string, ops: (target: Element) => PresentationOp[]): Promise<boolean> {
    const slide = this.slideId;
    if (!slide || el.kind?.type !== "text") return false;
    const editing = this.editingText === el.id ? this.textHooks : null;
    const draft = editing?.draft() ?? null;
    const target: Element = draft ? { ...el, kind: { ...el.kind, text: draft } } : el;
    const all = [...(draft && editing ? saveTextOps(slide, el.id, editing.before(), draft) : []), ...ops(target)];
    const ok = await this.run(label, all);
    if (editing && this.editingText === el.id) this.textHooks?.reload();
    return ok;
  }

  /** Character / paragraph style: the selected characters while editing, else the whole box. */
  textStyle(el: Element, label: string, style: { character?: CharStyle; paragraph?: ParagraphStyle; clear?: string[] }): Promise<boolean> {
    const range = this.editingText === el.id ? this.textRange : null;
    return this.editText(el, label, (t) => textStyleOps(this.slideId!, t, style, range));
  }

  /** This client holds the edit lock again: sends the kept typing, in order. */
  async sendLocked(): Promise<void> {
    const kept = this.lockedEdits;
    this.lockedEdits = [];
    for (const k of kept) await this.run(k.label, k.ops, { presentation: k.presentation });
  }

  /**
   * The editor closes: typing still kept from a lost lock is reported with its text (a notice),
   * never dropped silently (D09-1, D10-1); so is typing refused by a save still on its way (the
   * in-place text saved on close: E10-1).
   */
  reportLocked(): void {
    this.closed = true;
    const msg = lockedTypingNotice(this.lockedEdits.map((k) => k.text));
    this.lockedEdits = [];
    if (msg) session.conn?.pushNotice("error", msg);
  }

  /** The kept typing as text (for the clipboard), and lets it go. */
  takeLockedText(): string {
    const text = this.lockedEdits.map((k) => k.text).join("\n\n");
    this.lockedEdits = [];
    return text;
  }

  /** A 409 from a text save: offer theirs / mine. */
  private conflictFrom(e: unknown, ops: PresentationOp[], label: string, presentation: string): boolean {
    if (!(e instanceof ApiRequestError) || e.code !== "conflict") return false;
    const cur = e.current as { text?: unknown; element?: unknown; slide?: unknown } | null | undefined;
    const guard = ops.find((o) => o.type === "setSlideText" && o.expectText !== undefined);
    if (typeof cur?.text !== "string" || !guard || guard.type !== "setSlideText") return false;
    const rest = ops.filter((o) => o !== guard);
    const mine = rest.find((o) => o.type === "replaceTextBox");
    const mineText =
      mine && mine.type === "replaceTextBox"
        ? (mine.textBox.paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join("")).join("\n")
        : guard.text;
    this.conflict = {
      slide: guard.slide,
      element: guard.element ?? "",
      mine: mineText,
      theirs: cur.text,
      keepMine: async () => {
        this.conflict = null;
        await this.run(label, rest, { presentation });
      },
    };
    return true;
  }
}
