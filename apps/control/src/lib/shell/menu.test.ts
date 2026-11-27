import { describe, expect, it, vi } from "vitest";
import { CLOSED, clickItem, hoverItem, inCategories, menuKey, openMenu, paneMenuKey, pickCategory, type MenuDef, type MenuItem, type MenuNav } from "./menu";

const ran: string[] = [];
const run = (id: string) => () => ran.push(id);

const MENUS: MenuDef[] = [
  {
    id: "app",
    label: "Oriel",
    items: [
      { type: "item", id: "about", label: "About", run: run("about") },
      {
        type: "item",
        id: "settings",
        label: "Settings",
        submenu: [
          { type: "item", id: "account", label: "Account", disabled: true },
          { type: "item", id: "general", label: "General", run: run("general") },
          { type: "item", id: "screens", label: "Screens", run: run("screens") },
        ],
      },
      { type: "item", id: "updates", label: "Check for Updates", disabled: true },
      { type: "separator" },
      { type: "item", id: "disconnect", label: "Disconnect", run: run("disconnect") },
    ],
  },
  { id: "file", label: "File", items: [{ type: "item", id: "new", label: "New", disabled: true }] },
  {
    id: "screens",
    label: "Screens",
    items: [
      { type: "header", label: "Audience" },
      { type: "item", id: "enable", label: "Enable", shortcut: "Ctrl+1", checked: true, run: run("enable") },
    ],
  },
];

describe("menu keyboard handling", () => {
  it("moves through enabled items, skipping separators, headers and disabled ones", () => {
    let nav = openMenu(MENUS, 0, true);
    expect(nav.path).toEqual([0]);
    nav = menuKey(MENUS, nav, "ArrowDown").nav;
    expect(nav.path).toEqual([1]);
    nav = menuKey(MENUS, nav, "ArrowDown").nav; // skips disabled + separator
    expect(nav.path).toEqual([4]);
    nav = menuKey(MENUS, nav, "ArrowDown").nav; // wraps
    expect(nav.path).toEqual([0]);
    nav = menuKey(MENUS, nav, "ArrowUp").nav;
    expect(nav.path).toEqual([4]);
    nav = menuKey(MENUS, openMenu(MENUS, 2, true), "ArrowDown").nav;
    expect(nav.path).toEqual([1]); // the header is never highlighted
  });

  it("opens submenus with → and Enter, closes them with ← and Esc", () => {
    let nav: MenuNav = { open: 0, path: [1] };
    nav = menuKey(MENUS, nav, "ArrowRight").nav;
    expect(nav.path).toEqual([1, 1]); // first enabled item of the submenu
    nav = menuKey(MENUS, nav, "ArrowLeft").nav;
    expect(nav.path).toEqual([1]);
    nav = menuKey(MENUS, nav, "Enter").nav;
    expect(nav.path).toEqual([1, 1]);
    nav = menuKey(MENUS, nav, "Escape").nav;
    expect(nav).toEqual({ open: 0, path: [1] });
    expect(menuKey(MENUS, nav, "Escape").nav).toEqual(CLOSED);
  });

  it("moves between menus with ← and → at the top level", () => {
    let nav = openMenu(MENUS, 0, true);
    nav = menuKey(MENUS, nav, "ArrowRight").nav; // About has no submenu: next menu
    expect(nav.open).toBe(1);
    expect(nav.path).toEqual([-1]); // File has only disabled items
    nav = menuKey(MENUS, nav, "ArrowRight").nav;
    expect(nav.open).toBe(2);
    nav = menuKey(MENUS, nav, "ArrowRight").nav;
    expect(nav.open).toBe(0); // wraps
    nav = menuKey(MENUS, nav, "ArrowLeft").nav;
    expect(nav.open).toBe(2);
  });

  it("activates with Enter and closes; disabled items do nothing", () => {
    ran.length = 0;
    const r = menuKey(MENUS, { open: 0, path: [1, 2] }, "Enter");
    expect(r.nav).toEqual(CLOSED);
    r.run?.();
    expect(ran).toEqual(["screens"]);
    const d = menuKey(MENUS, { open: 0, path: [2] }, "Enter");
    expect(d.run).toBeUndefined();
    expect(d.nav).toEqual({ open: 0, path: [2] });
  });

  it("swallows other keys while open, so Space does not change slides", () => {
    const r = menuKey(MENUS, openMenu(MENUS, 0), "x");
    expect(r.handled).toBe(true);
    expect(menuKey(MENUS, CLOSED, "ArrowDown").handled).toBe(false);
  });

  it("follows the pointer: hover highlights and opens submenus, click runs", () => {
    let nav = openMenu(MENUS, 0);
    nav = hoverItem(MENUS, nav, 0, 1);
    expect(nav.path).toEqual([1, -1]);
    nav = hoverItem(MENUS, nav, 1, 0); // disabled: not highlighted
    expect(nav.path).toEqual([1, -1]);
    const spy = vi.fn();
    const menus: MenuDef[] = [{ id: "m", label: "M", items: [{ type: "item", id: "a", label: "A", run: spy }] }];
    const r = clickItem(menus, openMenu(menus, 0), 0, 0);
    expect(r.nav).toEqual(CLOSED);
    r.run?.();
    expect(spy).toHaveBeenCalledOnce();
  });
});

describe("two-pane menu keys", () => {
  const it2 = (id: string, disabled = false): MenuItem => ({ type: "item", id, label: id, disabled, run: () => {} });
  const panes: MenuDef[] = [
    { id: "file", label: "File", items: [it2("new"), { type: "separator" }, it2("print", true), it2("quit")] },
    { id: "edit", label: "Edit", items: [it2("undo", true), it2("find")] },
    { id: "help", label: "Help", items: [it2("about")] },
  ];

  it("moves through categories on the left and into the items with →", () => {
    let nav = pickCategory(panes, 0);
    expect(inCategories(nav)).toBe(true);
    nav = paneMenuKey(panes, nav, "ArrowDown").nav;
    expect(nav).toEqual({ open: 1, path: [-1] });
    nav = paneMenuKey(panes, nav, "ArrowUp").nav;
    nav = paneMenuKey(panes, nav, "ArrowUp").nav;
    expect(nav.open).toBe(2);
    nav = paneMenuKey(panes, pickCategory(panes, 1), "ArrowRight").nav;
    // The first enabled item (Undo is disabled).
    expect(nav).toEqual({ open: 1, path: [1] });
    expect(inCategories(nav)).toBe(false);
  });

  it("skips separators and disabled items, goes back with ←, runs with Enter", () => {
    let nav: MenuNav = { open: 0, path: [0] };
    nav = paneMenuKey(panes, nav, "ArrowDown").nav;
    expect(nav.path).toEqual([3]);
    nav = paneMenuKey(panes, nav, "ArrowLeft").nav;
    expect(nav).toEqual({ open: 0, path: [-1] });
    const r = paneMenuKey(panes, { open: 0, path: [3] }, "Enter");
    expect(r.nav).toEqual(CLOSED);
    expect(r.run).toBeTypeOf("function");
  });

  it("closes with Esc from either pane and lets Tab through", () => {
    expect(paneMenuKey(panes, { open: 1, path: [-1] }, "Escape").nav).toEqual(CLOSED);
    expect(paneMenuKey(panes, { open: 1, path: [1] }, "Escape").nav).toEqual(CLOSED);
    expect(paneMenuKey(panes, { open: 1, path: [1] }, "Tab")).toEqual({ nav: CLOSED, handled: false });
    expect(paneMenuKey(panes, { open: 1, path: [-1] }, "Tab").handled).toBe(false);
    // Other keys are swallowed (no slide changes behind the menu).
    expect(paneMenuKey(panes, { open: 1, path: [-1] }, " ").handled).toBe(true);
    expect(paneMenuKey(panes, CLOSED, "ArrowDown").handled).toBe(false);
  });
});
