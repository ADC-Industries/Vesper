// The Show page's own view state (Svelte 5 runes): slide view and size, the left column's list
// and selections, the bottom bin's tab and view, the right column's tab and transport target,
// and the panel sizes. Remembered
// per browser. The open presentation and playlist are loaded by `session` (state/session).

import { creds } from "./prefs";
import { clampThumb, THUMB_DEFAULT, THUMB_NUDGE } from "../pages/show/sections";

/** The right column's compact tabs (as in the canvas, left to right). */
export type RightTab = "timers" | "messages" | "stage";
export const RIGHT_TABS: RightTab[] = ["timers", "messages", "stage"];

/** The bottom bin's tabs (Video Inputs is drawn too, disabled until the server has inputs). */
export type BinTab = "media" | "audio" | "props" | "macros" | "inputs";
export const BIN_TABS: BinTab[] = ["media", "audio", "props", "macros", "inputs"];

/** What the right column's transport acts on. */
export type TransportTarget = "video" | "audio";

/** How the slide area shows the open presentation. */
export type SlideView = "grid" | "text" | "list";

/** Which list the "N ITEMS" section shows. */
export type ListSource = "library" | "playlist";

/** Panel sizes (px, and the centre split as a share of the height), saved per browser. */
export interface Layout {
  left: number;
  right: number;
  split: number;
  libraryHeight: number;
}

export const DEFAULT_LAYOUT: Layout = { left: 250, right: 400, split: 0.7, libraryHeight: 300 };

class ShowView {
  selectedPresentationId = $state<string | null>(null);
  selectedItemId = $state<string | null>(null);
  /** Slide tile width in CSS pixels (the size slider). */
  thumbPx = $state(THUMB_DEFAULT);
  /** The selected slide (`<section key>|<slide id>`): disabled slides can be selected too. */
  selectedSlide = $state<string | null>(null);
  slideView = $state<SlideView>("grid");
  listSource = $state<ListSource>("library");
  expandedFolders = $state<Record<string, boolean>>({});

  mediaVisible = $state(true);
  mediaView = $state<"grid" | "list">("grid");
  mediaColumns = $state(6);
  mediaFilter = $state("");
  expandedBinFolders = $state<Record<string, boolean>>({});

  rightTab = $state<RightTab>("timers");
  binTab = $state<BinTab>("media");
  transportTarget = $state<TransportTarget>("video");
  layout = $state<Layout>({ ...DEFAULT_LAYOUT });
  /** Bumped to move the focus to the right column's current tab. */
  focusRequest = $state(0);

  constructor() {
    const px = Number(creds.pref("thumbPx"));
    if (px > 0) this.thumbPx = clampThumb(px);
    const mcols = Number(creds.pref("mediaColumns"));
    if (mcols >= 2 && mcols <= 12) this.mediaColumns = mcols;
    const tab = creds.pref("rightTab");
    if (tab && (RIGHT_TABS as string[]).includes(tab)) this.rightTab = tab as RightTab;
    const bin = creds.pref("binTab");
    if (bin && (BIN_TABS as string[]).includes(bin)) this.binTab = bin as BinTab;
    const view = creds.pref("slideView");
    if (view === "grid" || view === "text" || view === "list") this.slideView = view;
    const mview = creds.pref("mediaView");
    if (mview === "grid" || mview === "list") this.mediaView = mview;
    if (creds.pref("mediaVisible") === "0") this.mediaVisible = false;
    this.layout = defaultLayout();
    try {
      const saved = JSON.parse(creds.pref("layout") ?? "null") as Partial<Layout> | null;
      if (saved) this.layout = clampLayout({ ...this.layout, ...saved });
    } catch {
      /* ignore a broken saved layout */
    }
  }

  setThumbPx(px: number) {
    this.thumbPx = clampThumb(px);
    creds.setPref("thumbPx", String(this.thumbPx));
  }

  /** Ctrl+= and Ctrl+wheel up. */
  thumbLarger() {
    this.setThumbPx(this.thumbPx + THUMB_NUDGE);
  }

  /** Ctrl+− and Ctrl+wheel down. */
  thumbSmaller() {
    this.setThumbPx(this.thumbPx - THUMB_NUDGE);
  }

  setMediaColumns(n: number) {
    this.mediaColumns = Math.max(2, Math.min(12, Math.round(n)));
    creds.setPref("mediaColumns", String(this.mediaColumns));
  }

  setRightTab(t: RightTab) {
    this.rightTab = t;
    creds.setPref("rightTab", t);
  }

  /** Picks a bin tab (and shows the bin if it was hidden). */
  setBinTab(t: BinTab) {
    this.binTab = t;
    creds.setPref("binTab", t);
    if (!this.mediaVisible) this.setMediaVisible(true);
  }

  setSlideView(v: SlideView) {
    this.slideView = v;
    creds.setPref("slideView", v);
  }

  setMediaView(v: "grid" | "list") {
    this.mediaView = v;
    creds.setPref("mediaView", v);
  }

  setMediaVisible(v: boolean) {
    this.mediaVisible = v;
    creds.setPref("mediaVisible", v ? "1" : "0");
  }

  /** Changes panel sizes (clamped) and remembers them. */
  setLayout(p: Partial<Layout>) {
    this.layout = clampLayout({ ...this.layout, ...p });
    creds.setPref("layout", JSON.stringify(this.layout));
  }

  resetLayout() {
    this.setLayout(defaultLayout());
  }

  /** Opens (or focuses) a right-column panel. */
  showPanel(t: RightTab) {
    this.setRightTab(t);
    this.focusRequest++;
  }

  toggleFolder(id: string) {
    this.expandedFolders[id] = !this.expandedFolders[id];
  }

  toggleBinFolder(key: string) {
    this.expandedBinFolders[key] = !this.expandedBinFolders[key];
  }
}

export const showView = new ShowView();

/** Column widths in the reference proportions (about 12.5 % and 20.5 % of the window); the
 * left column is full height, so its sources part gets about a third of it. */
function defaultLayout(): Layout {
  const w = typeof window === "undefined" ? 1920 : window.innerWidth;
  const h = typeof window === "undefined" ? 1080 : window.innerHeight;
  return clampLayout({
    left: Math.round(Math.max(210, w * 0.125)),
    right: Math.round(Math.max(300, w * 0.205)),
    split: DEFAULT_LAYOUT.split,
    libraryHeight: Math.round(Math.max(150, h * 0.32)),
  });
}

function clampLayout(l: Layout): Layout {
  const n = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);
  return {
    left: Math.max(180, Math.min(480, n(l.left, DEFAULT_LAYOUT.left))),
    right: Math.max(300, Math.min(640, n(l.right, DEFAULT_LAYOUT.right))),
    split: Math.max(0.25, Math.min(0.85, n(l.split, DEFAULT_LAYOUT.split))),
    libraryHeight: Math.max(90, Math.min(520, n(l.libraryHeight, DEFAULT_LAYOUT.libraryHeight))),
  };
}
