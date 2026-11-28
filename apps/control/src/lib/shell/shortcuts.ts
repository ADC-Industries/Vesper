// The one shortcut table (docs/ORIEL-REDESIGN-PLAN.md 3.2). It feeds the key dispatcher
// (keyboard.ts), the menus' accelerator labels and the "?" dialog (Settings ▸ Keyboard reads
// the same list). Shared file: change it only in a short commit announced to the other lanes.

/** App actions a shortcut, a menu item or a button can run (dispatched by `shell.appAction`). */
export type AppActionId =
  | "toggleAudience"
  | "toggleStage"
  | "viewShow"
  | "viewEdit"
  | "viewReflow"
  | "pageLibrary"
  | "pageBible"
  | "pageStage"
  | "pageTimers"
  | "pageScreens"
  | "pageSite"
  | "toggleMediaBin"
  | "thumbLarger"
  | "thumbSmaller"
  | "findInLibrary"
  | "newPresentation"
  | "newPlaylist"
  | "undo"
  | "redo"
  | "settings"
  | "importChordPro"
  | "importSongSelect";

/** A key combination matched by physical key (`KeyboardEvent.code`); `Ctrl` also means ⌘. */
export interface Binding {
  code: string;
  ctrl: boolean;
  shift?: boolean;
  alt?: boolean;
}

export type ShortcutGroup = "Show" | "Pages" | "Editing" | "Oriel";

export interface ShortcutRow {
  /** The keys as the "?" dialog shows them, one string per alternative. */
  keys: string[];
  action: string;
  note?: string;
  group: ShortcutGroup;
  /** Set for app actions the global dispatcher runs; rows without it are handled by the
   *  operator keys in keyboard.ts or by the editor or list that has the focus. */
  id?: AppActionId;
  bindings?: Binding[];
  /** Only in the desktop app: a browser keeps these keys for itself (tabs, zoom, windows). */
  desktopOnly?: boolean;
  /** A text field keeps this key for itself (its own undo and redo). */
  textFieldKeeps?: boolean;
}

const ctrl = (code: string, more: Partial<Binding> = {}): Binding => ({ code, ctrl: true, ...more });

export const SHORTCUT_TABLE: ShortcutRow[] = [
  { keys: ["Space", "→", "↓", "Page Down"], action: "Next slide", group: "Show" },
  { keys: ["←", "↑", "Page Up"], action: "Previous slide", group: "Show" },
  { keys: ["F1"], action: "Clear all", note: "F1 is no longer Help", group: "Show" },
  { keys: ["F2"], action: "Clear slide", group: "Show" },
  { keys: ["F3"], action: "Clear media", group: "Show" },
  { keys: ["F4"], action: "Clear audio", group: "Show" },
  { keys: ["F5"], action: "Clear props", group: "Show" },
  { keys: ["F6"], action: "Clear messages", group: "Show" },
  { keys: ["Ctrl+1"], action: "Audience screens on or off", group: "Show", id: "toggleAudience", bindings: [ctrl("Digit1")] },
  { keys: ["Ctrl+2"], action: "Stage screens on or off", group: "Show", id: "toggleStage", bindings: [ctrl("Digit2")] },
  { keys: ["Ctrl+M"], action: "Show or hide the media bin", group: "Show", id: "toggleMediaBin", bindings: [ctrl("KeyM")] },
  {
    keys: ["Ctrl+="],
    action: "Larger slide thumbnails",
    note: "desktop app only; a browser zooms the page",
    group: "Show",
    id: "thumbLarger",
    bindings: [ctrl("Equal"), ctrl("Equal", { shift: true }), ctrl("NumpadAdd")],
    desktopOnly: true,
  },
  {
    keys: ["Ctrl+−"],
    action: "Smaller slide thumbnails",
    note: "desktop app only; a browser zooms the page",
    group: "Show",
    id: "thumbSmaller",
    bindings: [ctrl("Minus"), ctrl("NumpadSubtract")],
    desktopOnly: true,
  },
  { keys: ["Ctrl+E"], action: "Edit (press again: back to Show)", group: "Pages", id: "viewEdit", bindings: [ctrl("KeyE")] },
  { keys: ["Ctrl+R"], action: "Reflow (press again: back to Show)", group: "Pages", id: "viewReflow", bindings: [ctrl("KeyR")] },
  { keys: ["Ctrl+S"], action: "Back to Show", note: "there is no Save: every edit is saved", group: "Pages", id: "viewShow", bindings: [ctrl("KeyS")] },
  { keys: ["Ctrl+L"], action: "Library", group: "Pages", id: "pageLibrary", bindings: [ctrl("KeyL")] },
  { keys: ["Ctrl+B"], action: "Bible", group: "Pages", id: "pageBible", bindings: [ctrl("KeyB")] },
  {
    keys: ["Ctrl+T"],
    action: "Stage display",
    note: "desktop app only (browsers keep Ctrl+T); use the menu in a browser",
    group: "Pages",
    id: "pageStage",
    bindings: [ctrl("KeyT")],
    desktopOnly: true,
  },
  { keys: ["Ctrl+Alt+T"], action: "Timers", group: "Pages", id: "pageTimers", bindings: [ctrl("KeyT", { alt: true })] },
  { keys: ["Ctrl+Alt+S"], action: "Screens & Looks", group: "Pages", id: "pageScreens", bindings: [ctrl("KeyS", { alt: true })] },
  { keys: ["Ctrl+Alt+M"], action: "Site management", group: "Pages", id: "pageSite", bindings: [ctrl("KeyM", { alt: true })] },
  { keys: ["Ctrl+F"], action: "Find in the library", group: "Pages", id: "findInLibrary", bindings: [ctrl("KeyF")] },
  {
    keys: ["Ctrl+N"],
    action: "New presentation",
    note: "desktop app only",
    group: "Editing",
    id: "newPresentation",
    bindings: [ctrl("KeyN")],
    desktopOnly: true,
  },
  {
    keys: ["Ctrl+Shift+N"],
    action: "New playlist",
    note: "desktop app only",
    group: "Editing",
    id: "newPlaylist",
    bindings: [ctrl("KeyN", { shift: true })],
    desktopOnly: true,
  },
  { keys: ["Ctrl+Z"], action: "Undo", note: "a text field keeps its own undo", group: "Editing", id: "undo", bindings: [ctrl("KeyZ")], textFieldKeeps: true },
  {
    keys: ["Ctrl+Shift+Z", "Ctrl+Y"],
    action: "Redo",
    group: "Editing",
    id: "redo",
    bindings: [ctrl("KeyZ", { shift: true }), ctrl("KeyY")],
    textFieldKeeps: true,
  },
  { keys: ["Ctrl+X", "Ctrl+C", "Ctrl+V"], action: "Cut, copy, paste", note: "editors and lists", group: "Editing" },
  { keys: ["Ctrl+D"], action: "Duplicate", note: "editors and lists", group: "Editing" },
  { keys: ["Delete"], action: "Delete", note: "editors and lists", group: "Editing" },
  { keys: ["Ctrl+A"], action: "Select all", note: "editors and lists", group: "Editing" },
  { keys: ["Ctrl+Enter"], action: "Reflow: slide break at the caret", note: "Backspace at a slide's start merges it up", group: "Editing" },
  { keys: ["Arrows", "Shift+Arrows"], action: "Editor: nudge 1 px (10 px)", group: "Editing" },
  { keys: ["Ctrl+,"], action: "Settings", group: "Oriel", id: "settings", bindings: [ctrl("Comma")] },
  { keys: ["?"], action: "Shortcut list", group: "Oriel" },
  { keys: ["Esc"], action: "Close menus and dialogs; cancel a drag", note: "doesn't leave a page", group: "Oriel" },
];

export const SHORTCUT_GROUPS: ShortcutGroup[] = ["Show", "Pages", "Editing", "Oriel"];

/** The rows the dispatcher matches: app actions with key bindings. */
export const APP_SHORTCUTS = SHORTCUT_TABLE.filter((r): r is ShortcutRow & { id: AppActionId; bindings: Binding[] } => !!r.id && !!r.bindings);

/**
 * The accelerator label a menu shows for an action ("Ctrl+E"), or undefined when it has
 * none (or, in a browser, when only the desktop app gets that key).
 */
export function shortcutLabel(id: AppActionId, desktop = true): string | undefined {
  const row = APP_SHORTCUTS.find((r) => r.id === id);
  if (!row || (row.desktopOnly && !desktop)) return undefined;
  return row.keys[0];
}

/** The rows the "?" dialog lists in a group (desktop-only keys marked in a browser). */
export function rowsOf(group: ShortcutGroup): ShortcutRow[] {
  return SHORTCUT_TABLE.filter((r) => r.group === group);
}
