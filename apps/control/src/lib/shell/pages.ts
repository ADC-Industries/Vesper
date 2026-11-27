// The page registry: the header's page buttons, their routes in the URL hash, what each page
// needs from the server and how its toolbar closes. Plain TypeScript, unit tested. Shared file:
// change it only in a short commit announced to the other lanes.

import type { AppActionId } from "./shortcuts";

/** The centre's mode on the Show page (the header's Show / Edit / Reflow). */
export type Mode = "show" | "edit" | "reflow";

/** Full-screen pages besides Show (the header's page buttons, and Settings). */
export type PageId = "bible" | "library" | "stage" | "timers" | "screens" | "site" | "settings";

/** Where the operator is: a mode of the Show page, or another page. One URL hash each. */
export type Route = Mode | PageId;

export interface PageDef {
  id: PageId;
  /** The header button. */
  label: string;
  /** The page toolbar's title. */
  title: string;
  action: AppActionId;
  /** A server capability the page needs (`GET /server`); without it the button is disabled. */
  capability?: string;
  /** Why it is disabled without the capability. */
  missing?: string;
  /** Done (pages that change outputs or documents) or Close. */
  closeLabel: "Done" | "Close";
  /** Not a header button (Settings: the Oriel menu and Ctrl+, open it). */
  menuOnly?: true;
}

export const PAGES: PageDef[] = [
  {
    id: "bible",
    label: "Bible",
    title: "Bible",
    action: "pageBible",
    capability: "scripture",
    missing: "This server has no Bible yet (update the server)",
    closeLabel: "Close",
  },
  {
    id: "library",
    label: "Library",
    title: "Library",
    action: "pageLibrary",
    capability: "codex",
    missing: "This server has no Codex library (update the server)",
    closeLabel: "Close",
  },
  { id: "stage", label: "Stage", title: "Stage display", action: "pageStage", closeLabel: "Close" },
  { id: "timers", label: "Timers", title: "Timers", action: "pageTimers", closeLabel: "Close" },
  {
    id: "screens",
    label: "Screens",
    title: "Screens & Looks",
    action: "pageScreens",
    capability: "screenConfig",
    missing: "This server can't configure its screens from Oriel",
    closeLabel: "Done",
  },
  {
    id: "site",
    label: "Site",
    title: "Site management",
    action: "pageSite",
    capability: "site",
    missing: "This server isn't part of a site",
    closeLabel: "Close",
  },
  { id: "settings", label: "Settings", title: "Settings", action: "settings", closeLabel: "Close", menuOnly: true },
];

/** The header's page buttons, in the canvas's order. */
export const HEADER_PAGES: PageDef[] = PAGES.filter((p) => !p.menuOnly);

export const MODES: Mode[] = ["show", "edit", "reflow"];

export function pageDef(id: PageId): PageDef {
  return PAGES.find((p) => p.id === id)!;
}

export function isMode(r: Route): r is Mode {
  return (MODES as string[]).includes(r);
}

/** The route a URL hash names (`#/library`), or null for anything else. */
export function parseRoute(hash: string): Route | null {
  const m = /^#\/?([a-z]+)\/?$/.exec(hash.trim());
  if (!m) return null;
  const r = m[1] as Route;
  return isMode(r) || PAGES.some((p) => p.id === r) ? r : null;
}

export function routeHash(r: Route): string {
  return `#/${r}`;
}

/** True when the server can show `r` (`has` answers for a capability). */
export function routeAvailable(r: Route, has: (cap: string) => boolean): boolean {
  if (isMode(r)) return true;
  const cap = pageDef(r).capability;
  return !cap || has(cap);
}
