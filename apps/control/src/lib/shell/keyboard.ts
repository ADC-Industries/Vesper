// Key matching for the shortcut table (shortcuts.ts). A pure function of the event, so it is
// unit tested; MainView feeds it every keydown and runs what it returns.
//
// - Modifier shortcuts match the physical key (`KeyboardEvent.code`) and the exact modifiers,
//   so the keyboard layout doesn't change them, and AltGr (which Windows reports as Ctrl+Alt)
//   never fires a Ctrl-only shortcut.
// - A text field keeps its own keys: plain keys, its undo / redo, and Ctrl+Alt combinations
//   (AltGr types characters there on many layouts).
// - Desktop-only keys (Ctrl+T, Ctrl+= / −, Ctrl+N) are left to the browser outside the desktop app.

import type { Command } from "$protocol";
import { APP_SHORTCUTS, type AppActionId } from "./shortcuts";

export type { AppActionId } from "./shortcuts";

export type KeyAction =
  | { kind: "command"; command: Command }
  | { kind: "app"; id: AppActionId }
  | { kind: "help" }
  | { kind: "closeDialog" };

export interface KeyContext {
  dialogOpen: boolean;
  /** The Edit page is open: arrows, Space and Page keys edit the slide instead of navigating. */
  editing?: boolean;
  /** The operator keys (slides, clears) are off on this page (the Screens page). */
  noShowKeys?: boolean;
  /** Running in the desktop app (Tauri): desktop-only shortcuts apply. */
  desktop?: boolean;
}

/** True when the key should go to a text field instead of the show. */
export function isTyping(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string") return false;
  const el = target as HTMLElement;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = ((el as HTMLInputElement).type || "text").toLowerCase();
    // Sliders and checkboxes don't take text; arrows on a slider still move the slider.
    return !["checkbox", "radio", "button", "submit", "reset", "color", "range"].includes(type);
  }
  return false;
}

/** The app shortcut an event matches (modifier combinations only), or null. */
export function appShortcut(e: KeyboardEvent, desktop = false): AppActionId | null {
  const ctrl = e.ctrlKey || e.metaKey;
  if (!ctrl && !e.altKey) return null;
  const code = e.code || "";
  const hit = APP_SHORTCUTS.find(
    (r) => (!r.desktopOnly || desktop) && r.bindings.some((b) => b.code === code && b.ctrl === ctrl && !!b.shift === !!e.shiftKey && !!b.alt === !!e.altKey),
  );
  return hit?.id ?? null;
}

export function keyAction(e: KeyboardEvent, ctx: KeyContext): KeyAction | null {
  if (e.key === "Escape") return ctx.dialogOpen ? { kind: "closeDialog" } : null;
  // A control took the key already (arrows on a tab strip or segmented control): not the show's.
  if (e.defaultPrevented) return null;
  const typing = isTyping(e.target);
  const app = appShortcut(e, !!ctx.desktop);
  if (app) {
    if (typing) {
      if (e.altKey && (e.ctrlKey || e.metaKey)) return null;
      if (APP_SHORTCUTS.find((r) => r.id === app)?.textFieldKeeps) return null;
    }
    return ctx.dialogOpen ? null : { kind: "app", id: app };
  }
  // Other modifier combinations (copy, reload, zoom, tabs) belong to the browser.
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  if (typing) return null;
  // Keys on a focused button (Space/Enter activate it) or slider belong to that control.
  const tag = (e.target as HTMLElement | null)?.tagName;
  const onSlider = tag === "INPUT" && (e.target as HTMLInputElement).type === "range";
  const onControl = tag === "BUTTON" || onSlider;

  if (e.key === "?") return { kind: "help" };
  if (ctx.dialogOpen || ctx.noShowKeys) return null;
  if (ctx.editing && [" ", "Spacebar", "ArrowRight", "ArrowDown", "PageDown", "ArrowLeft", "ArrowUp", "PageUp"].includes(e.key)) return null;

  switch (e.key) {
    case " ":
    case "Spacebar":
      return onControl ? null : { kind: "command", command: { type: "next" } };
    case "ArrowRight":
    case "ArrowDown":
    case "PageDown":
      return onSlider ? null : { kind: "command", command: { type: "next" } };
    case "ArrowLeft":
    case "ArrowUp":
    case "PageUp":
      return onSlider ? null : { kind: "command", command: { type: "prev" } };
    case "F1":
      return { kind: "command", command: { type: "clearAll" } };
    case "F2":
      return { kind: "command", command: { type: "clearLayer", layer: "slide" } };
    case "F3":
      return { kind: "command", command: { type: "clearLayer", layer: "media" } };
    case "F4":
      return { kind: "command", command: { type: "clearLayer", layer: "audio" } };
    case "F5":
      return { kind: "command", command: { type: "clearLayer", layer: "props" } };
    case "F6":
      return { kind: "command", command: { type: "clearLayer", layer: "messages" } };
  }
  return null;
}
