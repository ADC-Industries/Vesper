// The shell's state (Svelte 5 runes): which page and mode are shown (mirrored in the URL hash,
// so browser Back works and a page can be bookmarked), the dialogs, menus, popovers and
// question prompts, and the app actions that shortcuts, menus and buttons share.

import type { Color, MessageEntry } from "$protocol";
import type { AppActionId } from "../shell/shortcuts";
import type { MenuItem } from "../shell/menu";
import { isMode, parseRoute, routeAvailable, routeHash, type Mode, type PageId, type Route } from "../shell/pages";
import { HEADER_COLORS, session } from "./session.svelte";
import { libraryState } from "./library.svelte";
import { screensState } from "./screens.svelte";
import { showView } from "./show.svelte";
import { siteState } from "./site.svelte";

export type { Mode, PageId, Route } from "../shell/pages";

/** A question dialog (see PromptDialog). */
export interface PromptRequest {
  kind: "text" | "confirm" | "header";
  title: string;
  message?: string;
  label?: string;
  value?: string;
  color?: Color;
  confirm: string;
  danger?: boolean;
  resolve: (r: { value: string; color?: Color } | null) => void;
}

/** Header popovers (one at a time). */
export type Popover = "look" | "connection" | null;

class Shell {
  /** The page shown: "show" (with its mode) or a full-screen page. */
  page = $state<"show" | PageId>("show");
  /** The Show page's mode (Show / Edit / Reflow). */
  mode = $state<Mode>("show");
  /** Where Done / Close go back to, last first (pages and modes the operator came through). */
  private trail = $state<Route[]>([]);
  showHelp = $state(false);
  showDiagnostics = $state(false);
  showAbout = $state(false);
  /** The PowerPoint / Keynote / Google Slides import dialog (`state/slidesImport`). */
  showSlidesImport = $state(false);
  /** The live output dialog (the old toolbar's "Live" popover). */
  showLive = $state(false);
  /** The Settings page's section. */
  settingsSection = $state("General");
  popover = $state<Popover>(null);
  /** A menu is open: it gets the keys, not the show. */
  menuOpen = $state(false);
  /** A page edits slides in place (the Library's song Edit): arrows and Space don't drive the show. */
  pageEditing = $state(false);
  contextMenu = $state.raw<{ x: number; y: number; items: MenuItem[] } | null>(null);
  prompt = $state.raw<PromptRequest | null>(null);
  /** The message editor dialog (null entry: a new message). */
  messageEditor = $state.raw<{ entry: MessageEntry | null } | null>(null);
  /** The ChordPro dialog (batch 16): import a song, or add a ChordPro file's chords to one. */
  chordPro = $state.raw<{ mode: "import" } | { mode: "apply"; presentation: string } | null>(null);
  /** The SongSelect import dialog (files, or pasted lyrics: `text` from Reflow's smart paste). */
  songSelect = $state.raw<{ text?: string; files?: { name: string; data: string }[] } | null>(null);
  /** Where to go once the Screens page's unsaved changes are settled. */
  private pending: Route | null = null;

  /** The current route (a Show mode, or a page). */
  get route(): Route {
    return this.page === "show" ? this.mode : this.page;
  }

  /** True when the server can show `r` (its capabilities). */
  available(r: Route): boolean {
    return routeAvailable(r, (c) => siteState.has(c));
  }

  /**
   * Goes to a page or mode. Leaving the Screens page asks its unsaved-changes guard first
   * (`force` skips it). `push: false` doesn't add a history entry (Back, startup).
   */
  /** Where Done / Close go back to. */
  get from(): Route {
    return this.trail[this.trail.length - 1] ?? "show";
  }

  go(r: Route, opts: { force?: boolean; push?: boolean; back?: boolean } = {}): boolean {
    const cur = this.route;
    if (r === cur) return true;
    if (!this.available(r)) return false;
    if (cur === "screens" && !opts.force && !screensState.mayLeave()) {
      this.pending = r;
      return false;
    }
    this.pending = null;
    if (r === "show") this.trail = [];
    else if (!opts.back && (!isMode(r) || !isMode(cur))) this.trail = [...this.trail.filter((x) => x !== r && x !== cur), cur].slice(-10);
    this.popover = null;
    this.contextMenu = null;
    if (isMode(r)) {
      this.page = "show";
      this.mode = r;
    } else {
      this.page = r;
      if (this.mode !== "show") this.mode = "show";
    }
    // Leaving the Edit view ends prop editing (after the mode changed, so it doesn't navigate).
    if (cur === "edit" && r !== "edit" && session.open?.prop) session.closeProp();
    if (opts.push !== false) writeHash(r);
    return true;
  }

  /** Done / Close: back to where the operator came from. */
  close(force = false) {
    const back = this.from;
    if (this.go(back === this.route ? "show" : back, { force, back: true })) this.trail = this.trail.slice(0, -1);
  }

  /** The Screens page settled its changes (applied or discarded): go where the operator was going. */
  resumeAfterGuard() {
    const to = this.pending;
    this.pending = null;
    if (to) this.go(to, { force: true });
    else this.close(true);
  }

  /** Follows the URL hash (startup, browser Back / Forward). Returns the cleanup. */
  initRouting(): () => void {
    if (typeof window === "undefined") return () => {};
    const apply = () => {
      const r = parseRoute(location.hash) ?? "show";
      if (!this.go(this.available(r) ? r : "show", { push: false })) writeHash(this.route, true);
      else if (!this.available(r)) writeHash("show", true);
    };
    apply();
    window.addEventListener("hashchange", apply);
    return () => window.removeEventListener("hashchange", apply);
  }

  /** Back to the Show page without asking (disconnect). */
  reset() {
    this.pending = null;
    this.page = "show";
    this.mode = "show";
    this.trail = [];
    // Nothing of the old connection stays open: an open popover (the connection pill's, whose
    // "Switch server" disconnects), menu or page edit would block the show keys after the next
    // connect.
    this.showHelp = false;
    this.showDiagnostics = false;
    this.showAbout = false;
    this.showSlidesImport = false;
    this.showLive = false;
    this.messageEditor = null;
    this.chordPro = null;
    this.songSelect = null;
    this.popover = null;
    this.menuOpen = false;
    this.contextMenu = null;
    this.pageEditing = false;
    if (this.prompt) {
      const p = this.prompt;
      this.prompt = null;
      p.resolve(null);
    }
  }

  setMode(m: Mode) {
    this.go(m);
  }

  togglePopover(p: Exclude<Popover, null>) {
    this.popover = this.popover === p ? null : p;
  }

  /** Closes every dialog (Esc). Pages stay: Esc doesn't leave a page. */
  closeDialogs(): boolean {
    if (this.page === "screens" && screensState.escape?.()) return false;
    this.showHelp = false;
    this.showDiagnostics = false;
    this.showAbout = false;
    this.showSlidesImport = false;
    this.showLive = false;
    this.messageEditor = null;
    this.chordPro = null;
    this.songSelect = null;
    this.popover = null;
    this.contextMenu = null;
    if (this.prompt) {
      const p = this.prompt;
      this.prompt = null;
      p.resolve(null);
    }
    return true;
  }

  /** Goes to the Settings page at a section (Ctrl+,, the Oriel menu's Settings pane). */
  openSettings(section: string) {
    if (!this.closeDialogs()) return;
    if (this.go("settings")) this.settingsSection = section;
  }

  get dialogOpen(): boolean {
    return (
      this.showHelp ||
      this.showDiagnostics ||
      this.showAbout ||
      this.showSlidesImport ||
      this.showLive ||
      this.messageEditor !== null ||
      this.chordPro !== null ||
      this.songSelect !== null ||
      this.popover !== null ||
      this.prompt !== null ||
      this.contextMenu !== null
    );
  }

  /** Runs an app action (shared by shortcuts, menus and the header). */
  appAction(id: AppActionId) {
    switch (id) {
      case "toggleAudience":
        return void session.toggleScreens("audience");
      case "toggleStage":
        return void session.toggleScreens("stage");
      case "viewShow":
        this.closeDialogs();
        this.go("show");
        return;
      case "viewReflow":
        if (!this.closeDialogs()) return;
        this.go(this.route === "reflow" ? "show" : "reflow");
        return;
      case "viewEdit":
        if (!this.closeDialogs()) return;
        this.go(this.route === "edit" ? "show" : "edit");
        return;
      case "pageLibrary":
      case "pageBible":
      case "pageStage":
      case "pageTimers":
      case "pageScreens":
      case "pageSite": {
        const page = id.slice(4).toLowerCase() as PageId;
        if (!this.closeDialogs()) return;
        this.go(page);
        return;
      }
      case "findInLibrary":
        if (!this.closeDialogs()) return;
        if (this.go("library")) libraryState.searchFocus++;
        return;
      case "toggleMediaBin":
        return showView.setMediaVisible(!showView.mediaVisible);
      case "thumbLarger":
        return showView.thumbLarger();
      case "thumbSmaller":
        return showView.thumbSmaller();
      case "newPresentation":
        return void session.newPresentation();
      case "importChordPro":
        if (!this.closeDialogs()) return;
        this.chordPro = { mode: "import" };
        return;
      case "importSongSelect":
        if (!this.closeDialogs()) return;
        this.songSelect = {};
        return;
      case "newPlaylist":
        return void session.newPlaylist(false);
      case "undo":
        return void session.undo();
      case "redo":
        return void session.redo();
      case "settings":
        return this.openSettings("General");
    }
  }

  private ask(req: Omit<PromptRequest, "resolve">): Promise<{ value: string; color?: Color } | null> {
    this.contextMenu = null;
    this.popover = null;
    if (this.prompt) this.prompt.resolve(null);
    return new Promise((resolve) => (this.prompt = { ...req, resolve }));
  }

  /** Asks for a name; null when cancelled or empty. */
  async askText(title: string, value = "", confirm = "OK", label = "Name"): Promise<string | null> {
    return (await this.ask({ kind: "text", title, value, confirm, label }))?.value ?? null;
  }

  async confirm(title: string, message: string, confirm = "Delete", danger = true): Promise<boolean> {
    return !!(await this.ask({ kind: "confirm", title, message, confirm, danger }));
  }

  async askHeader(title: string, name = "", color?: Color): Promise<{ name: string; color: Color } | null> {
    const r = await this.ask({ kind: "header", title, value: name, color, confirm: "OK" });
    return r ? { name: r.value, color: r.color ?? HEADER_COLORS[0]! } : null;
  }
}

export const shell = new Shell();

function writeHash(r: Route, replace = false) {
  if (typeof location === "undefined" || typeof history === "undefined") return;
  const h = routeHash(r);
  if (location.hash === h) return;
  if (replace) history.replaceState(history.state, "", h);
  else history.pushState(history.state, "", h);
}
