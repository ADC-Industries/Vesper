// The menu model and keyboard handling. Plain TypeScript (no DOM), so the keyboard behaviour is
// unit tested; OrielMenu.svelte renders it and feeds it keys and clicks. Shared file: change it
// only in a short commit announced to the other lanes.
//
// Menus are a list of categories (`MenuDef`), each with items. ↑/↓ move through enabled items
// (skipping separators, headers and disabled items), → opens a submenu, ← closes it,
// Enter/Space activates, Esc closes. The Oriel menu shows them in two panes (`paneMenuKey`):
// categories on the left (path [-1] = the focus is there), the open category's items on the
// right.

export type MenuItem =
  | {
      type: "item";
      id: string;
      label: string;
      /** Right-aligned shortcut text ("Ctrl+1"). */
      shortcut?: string;
      disabled?: boolean;
      /** Tooltip (e.g. why it is disabled). */
      title?: string;
      /** A short note shown dimmed where the shortcut goes ("coming later"). */
      note?: string;
      /** Checkmark state for toggles (`undefined` = not a toggle). */
      checked?: boolean;
      /** Italic emphasis for a value part of the label ("Live: *Default*"). */
      detail?: string;
      run?: () => void;
      submenu?: MenuItem[];
    }
  | { type: "separator" }
  | { type: "header"; label: string };

export interface MenuDef {
  id: string;
  label: string;
  items: MenuItem[];
}

/** Which menu is open and which item is highlighted at each level (-1 = none). */
export interface MenuNav {
  /** Index of the open top-level menu, or null when closed. */
  open: number | null;
  /** Highlighted item index per open level: [top menu, submenu, sub-submenu, ...]. */
  path: number[];
}

export const CLOSED: MenuNav = { open: null, path: [] };

export type MenuResult = { nav: MenuNav; run?: () => void; handled: boolean };

export function selectable(item: MenuItem | undefined): boolean {
  return !!item && item.type === "item" && !item.disabled;
}

/** The items shown at `level` for this navigation state. */
export function levelItems(menus: MenuDef[], nav: MenuNav, level: number): MenuItem[] {
  if (nav.open === null) return [];
  let items = menus[nav.open]?.items ?? [];
  for (let l = 0; l < level; l++) {
    const it = items[nav.path[l] ?? -1];
    if (!it || it.type !== "item" || !it.submenu) return [];
    items = it.submenu;
  }
  return items;
}

function step(items: MenuItem[], from: number, dir: 1 | -1): number {
  const n = items.length;
  if (!n) return -1;
  for (let k = 1; k <= n; k++) {
    const i = (((from + dir * k) % n) + n) % n;
    if (selectable(items[i])) return i;
  }
  return -1;
}

/** Opens top menu `index` (with nothing highlighted, or the first item for keyboard use). */
export function openMenu(menus: MenuDef[], index: number, highlightFirst = false): MenuNav {
  const items = menus[index]?.items ?? [];
  return { open: index, path: [highlightFirst ? step(items, -1, 1) : -1] };
}

/** Handles a key while a menu is open. `handled: false` = let the key through. */
export function menuKey(menus: MenuDef[], nav: MenuNav, key: string): MenuResult {
  if (nav.open === null) return { nav, handled: false };
  const depth = nav.path.length - 1;
  const items = levelItems(menus, nav, depth);
  const cur = nav.path[depth] ?? -1;
  const at = items[cur];
  const set = (i: number): MenuNav => ({ open: nav.open, path: [...nav.path.slice(0, depth), i] });
  switch (key) {
    case "Escape":
      if (depth > 0) return { nav: { open: nav.open, path: nav.path.slice(0, -1) }, handled: true };
      return { nav: CLOSED, handled: true };
    case "ArrowDown":
      return { nav: set(step(items, cur, 1)), handled: true };
    case "ArrowUp":
      return { nav: set(step(items, cur < 0 ? items.length : cur, -1)), handled: true };
    case "Home":
      return { nav: set(step(items, -1, 1)), handled: true };
    case "End":
      return { nav: set(step(items, items.length, -1)), handled: true };
    case "ArrowRight":
      if (at && at.type === "item" && at.submenu && !at.disabled) {
        return { nav: { open: nav.open, path: [...nav.path, step(at.submenu, -1, 1)] }, handled: true };
      }
      return { nav: openMenu(menus, (nav.open + 1) % menus.length, true), handled: true };
    case "ArrowLeft":
      if (depth > 0) return { nav: { open: nav.open, path: nav.path.slice(0, -1) }, handled: true };
      return { nav: openMenu(menus, (nav.open - 1 + menus.length) % menus.length, true), handled: true };
    case "Enter":
    case " ":
      if (!at || at.type !== "item" || at.disabled) return { nav, handled: true };
      if (at.submenu) return { nav: { open: nav.open, path: [...nav.path, step(at.submenu, -1, 1)] }, handled: true };
      return { nav: CLOSED, run: at.run, handled: true };
    case "Tab":
      return { nav: CLOSED, handled: false };
  }
  // Every other key is swallowed while a menu is open (no slide changes behind it).
  return { nav, handled: true };
}

/** Pointer moved onto item `index` at `level`: highlight it and open its submenu, if any. */
export function hoverItem(menus: MenuDef[], nav: MenuNav, level: number, index: number): MenuNav {
  if (nav.open === null) return nav;
  const items = levelItems(menus, nav, level);
  const it = items[index];
  const path = [...nav.path.slice(0, level), selectable(it) ? index : -1];
  if (it && it.type === "item" && it.submenu && !it.disabled) path.push(-1);
  return { open: nav.open, path };
}

/** Item clicked: returns what to run (menus close) or the state with its submenu open. */
export function clickItem(menus: MenuDef[], nav: MenuNav, level: number, index: number): MenuResult {
  const items = levelItems(menus, nav, level);
  const it = items[index];
  if (!it || it.type !== "item" || it.disabled) return { nav, handled: true };
  if (it.submenu) return { nav: hoverItem(menus, nav, level, index), handled: true };
  return { nav: CLOSED, run: it.run, handled: true };
}

/** Pointer on category `index` of a two-pane menu: show its items, the focus stays left. */
export function pickCategory(menus: MenuDef[], index: number): MenuNav {
  return index >= 0 && index < menus.length ? { open: index, path: [-1] } : CLOSED;
}

/** True when the focus is on the category pane of a two-pane menu. */
export function inCategories(nav: MenuNav): boolean {
  return nav.open !== null && nav.path.length <= 1 && (nav.path[0] ?? -1) < 0;
}

/**
 * Keys in a two-pane menu (the Oriel menu). In the category pane ↑/↓/Home/End pick the
 * category and →/Enter/Space move into its items; in the items pane ← goes back to the
 * categories; Esc closes; Tab closes and lets the key through.
 */
export function paneMenuKey(menus: MenuDef[], nav: MenuNav, key: string): MenuResult {
  if (nav.open === null) return { nav, handled: false };
  const n = menus.length;
  if (inCategories(nav)) {
    switch (key) {
      case "ArrowDown":
        return { nav: pickCategory(menus, (nav.open + 1) % n), handled: true };
      case "ArrowUp":
        return { nav: pickCategory(menus, (nav.open - 1 + n) % n), handled: true };
      case "Home":
        return { nav: pickCategory(menus, 0), handled: true };
      case "End":
        return { nav: pickCategory(menus, n - 1), handled: true };
      case "ArrowRight":
      case "Enter":
      case " ": {
        const first = step(menus[nav.open]?.items ?? [], -1, 1);
        return { nav: first < 0 ? nav : { open: nav.open, path: [first] }, handled: true };
      }
      case "Escape":
        return { nav: CLOSED, handled: true };
      case "Tab":
        return { nav: CLOSED, handled: false };
    }
    return { nav, handled: true };
  }
  if (nav.path.length === 1) {
    if (key === "ArrowLeft") return { nav: pickCategory(menus, nav.open), handled: true };
    if (key === "Escape") return { nav: CLOSED, handled: true };
    if (key === "ArrowRight") {
      const at = levelItems(menus, nav, 0)[nav.path[0] ?? -1];
      if (!(at && at.type === "item" && at.submenu && !at.disabled)) return { nav, handled: true };
    }
  }
  return menuKey(menus, nav, key);
}
