// The slide editor's editing keys, shared by the Edit page and the Library's song Edit:
// Delete / Backspace, Esc, Enter (edit text in place), arrows (nudge, Shift ×10), Ctrl+A / C /
// X / V / D, Ctrl+= / − / 0 (zoom). `editorKey` maps a key press to a command (pure);
// `handleEditorKey` carries it out on an editor.

import type { PresentationOp } from "$protocol";
import type { EditState } from "../../edit/editState.svelte";
import { getClipboard, setClipboard } from "../../edit/editState.svelte";
import { boundsOf, copyElements, pasteElements, type ElementClipboard } from "../../edit/elements";
import { nudge } from "../../edit/geometry";
import { uuid } from "../../edit/ids";
import { boundsOps } from "../../edit/ops";
import { isTyping } from "../../shell/keyboard";
import { shell } from "../../state/shell.svelte";

/** What the editor's canvas offers the keys. */
export interface CanvasHandle {
  zoomStep: (d: 1 | -1) => void;
  editText: (id: string) => void;
  cancelGesture: () => boolean;
}

export type EditorKey =
  | { cmd: "delete" | "escape" | "enter" | "selectAll" | "copy" | "cut" | "paste" | "duplicate" | "zoomIn" | "zoomOut" | "zoomFit" }
  | { cmd: "nudge"; dx: number; dy: number };

export interface KeyLike {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** The command a key press means in the editor, or null. Ctrl+Alt (AltGr) is never a command. */
export function editorKey(e: KeyLike): EditorKey | null {
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) {
    if (e.altKey) return null;
    if (e.key === "Delete" || e.key === "Backspace") return { cmd: "delete" };
    if (e.key === "Escape") return { cmd: "escape" };
    if (e.key === "Enter") return { cmd: "enter" };
    const d = nudge(e.key, e.shiftKey);
    return d ? { cmd: "nudge", ...d } : null;
  }
  if (e.altKey) return null;
  switch (e.code) {
    case "KeyA":
      return { cmd: "selectAll" };
    case "KeyC":
      return { cmd: "copy" };
    case "KeyX":
      return { cmd: "cut" };
    case "KeyV":
      return { cmd: "paste" };
    case "KeyD":
      return { cmd: "duplicate" };
    case "Equal":
    case "NumpadAdd":
      return { cmd: "zoomIn" };
    case "Minus":
    case "NumpadSubtract":
      return { cmd: "zoomOut" };
    case "Digit0":
    case "Numpad0":
      return { cmd: "zoomFit" };
  }
  return null;
}

/** Deletes the selected objects that aren't locked (one undo step). */
export async function deleteSelection(st: EditState, label?: string): Promise<void> {
  const slide = st.slide;
  const movable = st.selected.filter((x) => !x.locked);
  if (!slide || !movable.length) return;
  st.markDeleted(movable.map((x) => x.id));
  await st.run(label ?? (movable.length > 1 ? "Delete Objects" : "Delete"), movable.map((x) => ({ type: "deleteElement" as const, slide: slide.id, element: x.id })));
  st.select(st.selection.filter((id) => !movable.some((m) => m.id === id)));
}

/** Adds copies of clipboard elements to the current slide and selects them. */
export async function pasteInto(st: EditState, clip: ElementClipboard, label: string): Promise<void> {
  const slide = st.slide;
  if (!slide) return;
  const els = pasteElements(clip, slide, uuid);
  const ops: PresentationOp[] = els.map((el) => ({ type: "addElement", slide: slide.id, element: el }));
  if (await st.run(label, ops)) st.select(els.map((x) => x.id));
}

/** Duplicates the selected objects (offset copies, selected afterwards). */
export function duplicateSelection(st: EditState): Promise<void> {
  const slide = st.slide;
  if (!slide || !st.selection.length) return Promise.resolve();
  return pasteInto(st, copyElements(slide, st.selection), "Duplicate");
}

/** Carries out an editing key on the editor (unless a menu, dialog or text field has it). */
export async function handleEditorKey(e: KeyboardEvent, st: EditState, canvas: CanvasHandle | null): Promise<void> {
  if (shell.menuOpen || shell.dialogOpen || st.conflict || isTyping(e.target)) return;
  const slide = st.slide;
  const k = editorKey(e);
  if (!slide || !k) return;
  const sel = st.selected;
  const movable = sel.filter((x) => !x.locked);
  switch (k.cmd) {
    case "delete":
      if (!movable.length) return;
      e.preventDefault();
      await deleteSelection(st);
      return;
    case "escape":
      if (canvas?.cancelGesture()) e.preventDefault();
      else if (st.selection.length) {
        e.preventDefault();
        st.select([]);
      }
      return;
    case "enter": {
      const t = st.single;
      if (t?.kind?.type === "text" && !t.locked) {
        e.preventDefault();
        canvas?.editText(t.id);
      }
      return;
    }
    case "nudge":
      if (!movable.length) return;
      e.preventDefault();
      await st.run(
        "Nudge",
        boundsOps(
          slide.id,
          movable.map((x) => {
            const b = boundsOf(x);
            return { id: x.id, rect: { ...b, x: b.x + k.dx, y: b.y + k.dy }, was: b };
          }),
        ),
      );
      return;
    case "selectAll":
      e.preventDefault();
      st.select(st.elements.filter((x) => !x.locked && !x.hidden).map((x) => x.id));
      return;
    case "copy":
    case "cut":
      if (!sel.length) return;
      e.preventDefault();
      setClipboard(copyElements(slide, sel.map((x) => x.id)));
      if (k.cmd === "cut" && movable.length) await deleteSelection(st, "Cut");
      return;
    case "paste": {
      const clip = getClipboard();
      if (!clip) return;
      e.preventDefault();
      await pasteInto(st, clip, "Paste");
      return;
    }
    case "duplicate":
      if (!sel.length) return;
      e.preventDefault();
      await duplicateSelection(st);
      return;
    case "zoomIn":
    case "zoomOut":
      e.preventDefault();
      canvas?.zoomStep(k.cmd === "zoomIn" ? 1 : -1);
      return;
    case "zoomFit":
      e.preventDefault();
      st.viewMode = "fit";
      return;
  }
}
