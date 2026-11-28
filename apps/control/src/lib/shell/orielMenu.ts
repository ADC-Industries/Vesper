// The Oriel menu's content (the canvas's two-pane menu: File, Edit, View, Import, Export,
// Settings, Help), built from what Oriel and the server can do. Plain TypeScript, unit tested;
// OrielMenu.svelte renders it. No Open, Save or Save As: the server saves every edit. Items for
// later features are shown disabled with "coming later", never as a mock.
//
// Every command of the old menu bar and toolbar is here, on the header or on a page: Search is
// the Library page (Ctrl+F), Look is the header's Look picker, Status is the connection pill,
// the stage screens' layouts and Edit Layouts are on the Stage page.

import type { MenuDef, MenuItem } from "./menu";
import { PAGES, type Mode, type Route } from "./pages";
import { shortcutLabel, type AppActionId } from "./shortcuts";

export const LATER = "coming later";

/** What the menu shows (state) and runs (actions). */
export interface OrielMenuContext {
  appName: string;
  desktop: boolean;
  route: Route;
  /** True when the server can show this route (capabilities). */
  available: (r: Route) => boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  undoBusy: boolean;
  audienceOn: boolean;
  stageOn: boolean;
  continuous: boolean;
  slideView: "grid" | "text" | "list";
  mediaVisible: boolean;
  act: {
    app: (id: AppActionId) => void;
    newPlaylistFolder: () => void;
    newLibrary: () => void;
    setSlideView: (v: "grid" | "text" | "list") => void;
    toggleContinuous: () => void;
    liveOutput: () => void;
    fullScreen: () => void;
    resetLayout: () => void;
    settings: (section: string) => void;
    shortcuts: () => void;
    diagnostics: () => void;
    about: () => void;
    disconnect: () => void;
    /** Import ▸ PowerPoint / Keynote / Google Slides…. */
    importSlides: () => void;
  };
}

type Item = Extract<MenuItem, { type: "item" }>;
const sep: MenuItem = { type: "separator" };
const header = (label: string): MenuItem => ({ type: "header", label });

export function orielMenu(c: OrielMenuContext): MenuDef[] {
  const item = (id: string, label: string, opts: Partial<Item> = {}): MenuItem => ({ type: "item", id, label, ...opts });
  const later = (id: string, label: string): MenuItem => item(id, label, { disabled: true, note: LATER, title: `${label.replace(/…$/, "")}: ${LATER}` });
  const app = (id: string, label: string, action: AppActionId, opts: Partial<Item> = {}): MenuItem =>
    item(id, label, { shortcut: shortcutLabel(action, c.desktop), run: () => c.act.app(action), ...opts });
  const mode = (m: Mode, label: string, action: AppActionId) => app(`v-${m}`, label, action, { checked: c.route === m });
  const page = (id: (typeof PAGES)[number]["id"], label: string) => {
    const def = PAGES.find((p) => p.id === id)!;
    const ok = c.available(id);
    return app(`p-${id}`, label, def.action, { checked: c.route === id, disabled: !ok, title: ok ? undefined : def.missing });
  };

  return [
    {
      id: "file",
      label: "File",
      items: [
        app("new-pres", "New Presentation…", "newPresentation"),
        app("new-playlist", "New Playlist…", "newPlaylist"),
        item("new-folder", "New Playlist Folder…", { run: c.act.newPlaylistFolder }),
        item("new-library", "New Library…", { run: c.act.newLibrary }),
        sep,
        later("package", "Package Playlist…"),
        later("print", "Print…"),
        sep,
        item("disconnect", "Disconnect / Switch Server", { run: c.act.disconnect }),
      ],
    },
    {
      id: "edit",
      label: "Edit",
      items: [
        app("undo", c.undoLabel ? `Undo ${c.undoLabel}` : "Undo", "undo", {
          disabled: !c.undoLabel || c.undoBusy,
          title: c.undoLabel ? undefined : "Nothing to undo",
        }),
        app("redo", c.redoLabel ? `Redo ${c.redoLabel}` : "Redo", "redo", {
          disabled: !c.redoLabel || c.undoBusy,
          title: c.redoLabel ? undefined : "Nothing to redo",
        }),
        sep,
        later("cut", "Cut"),
        later("copy", "Copy"),
        later("paste", "Paste"),
        later("duplicate", "Duplicate"),
        later("delete", "Delete"),
        later("select-all", "Select All"),
        sep,
        app("find", "Find in Library…", "findInLibrary", { disabled: !c.available("library"), title: c.available("library") ? undefined : PAGES[1]!.missing }),
      ],
    },
    {
      id: "view",
      label: "View",
      items: [
        mode("show", "Show", "viewShow"),
        mode("edit", "Edit", "viewEdit"),
        mode("reflow", "Reflow", "viewReflow"),
        sep,
        page("bible", "Bible"),
        page("library", "Library"),
        page("stage", "Stage Display"),
        page("timers", "Timers"),
        page("screens", "Screens & Looks"),
        page("site", "Site Management"),
        sep,
        app("media-bin", "Media Bin", "toggleMediaBin", { checked: c.mediaVisible }),
        app("larger", "Larger Thumbnails", "thumbLarger"),
        app("smaller", "Smaller Thumbnails", "thumbSmaller"),
        header("Slides as"),
        item("sv-grid", "Grid", { checked: c.slideView === "grid", run: () => c.act.setSlideView("grid") }),
        item("sv-text", "Text", { checked: c.slideView === "text", run: () => c.act.setSlideView("text") }),
        item("sv-list", "List", { checked: c.slideView === "list", run: () => c.act.setSlideView("list") }),
        sep,
        item("continuous", "Continuous Playlist", {
          checked: c.continuous,
          title: "At the end of a presentation, Next moves on to the next playlist item",
          run: c.act.toggleContinuous,
        }),
        app("audience", "Audience Screens", "toggleAudience", { checked: c.audienceOn }),
        app("stage", "Stage Screens", "toggleStage", { checked: c.stageOn }),
        item("live", "Live Output…", { run: c.act.liveOutput }),
        sep,
        item("fullscreen", "Full Screen", { shortcut: "F11", run: c.act.fullScreen }),
        item("reset-layout", "Reset Panel Sizes", { run: c.act.resetLayout }),
      ],
    },
    {
      id: "import",
      label: "Import",
      items: [
        app("lyrics", "Lyrics from Text…", "newPresentation", { shortcut: undefined, title: "A new presentation opens in Reflow: type or paste the lyrics" }),
        app("chordpro", "ChordPro Song…", "importChordPro", { title: "A song with chords from a .cho / .chordpro / .pro file or pasted text" }),
        later("media-files", "Media Files…"),
        later("codex-import", "Codex Export File…"),
        sep,
        app("songselect", "Song from SongSelect…", "importSongSelect", { title: "Songs downloaded from CCLI SongSelect (lyrics .txt, ChordPro, .usr) or pasted SongSelect lyrics" }),
        later("word", "Lyrics from Word…"),
        item("pptx", "PowerPoint / Keynote / Google Slides…", {
          title: "A .pptx into the Codex, editable where possible (Keynote and Google Slides: export as .pptx first)",
          run: c.act.importSlides,
        }),
        later("pdf", "PDF…"),
        later("other-app", "Library from Another App…"),
        later("bible-import", "Bible Translation…"),
      ],
    },
    {
      id: "export",
      label: "Export",
      items: [
        later("codex-backup", "Back Up Codex…"),
        sep,
        later("pdf-out", "Presentation as PDF…"),
        later("images-out", "Presentation as Images…"),
        later("video-out", "Presentation as Video…"),
        sep,
        later("order", "Playlist as Order of Service…"),
        later("package-out", "Package Playlist…"),
      ],
    },
    {
      id: "settings",
      label: "Settings",
      items: [
        item("s-general", "General…", { shortcut: shortcutLabel("settings", c.desktop), run: () => c.act.settings("General") }),
        page("screens", "Screens & Looks…"),
        item("s-audio", "Audio…", { run: () => c.act.settings("Audio") }),
        item("s-captions", "Captions…", { run: () => c.act.settings("Captions") }),
        item("s-keyboard", "Keyboard Shortcuts…", { run: c.act.shortcuts }),
        item("s-network", "Network & IDN…", { run: () => c.act.settings("Network") }),
        item("s-streaming", "Recording & Streaming…", { run: () => c.act.settings("Streaming") }),
        item("s-media", "Media Preparation…", { run: () => c.act.settings("Media") }),
        later("s-labels", "Slide Labels…"),
        later("s-ccli", "Licenses (CCLI)…"),
        sep,
        later("updates", "Check for Updates…"),
      ],
    },
    {
      id: "reports",
      label: "Reports",
      items: [item("r-ccli", "CCLI Usage…", { run: () => c.act.settings("Reports") })],
    },
    {
      id: "help",
      label: "Help",
      items: [
        item("h-shortcuts", "Keyboard Shortcuts", { shortcut: "?", run: c.act.shortcuts }),
        item("h-diagnostics", "Diagnostics…", { run: c.act.diagnostics }),
        later("h-help", `${c.appName} Help`),
        sep,
        item("h-about", `About ${c.appName}`, { run: c.act.about }),
      ],
    },
  ];
}
