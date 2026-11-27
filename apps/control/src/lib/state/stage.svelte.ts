// The stage display's state (Svelte 5 runes): stage layouts and the built-in templates new ones
// start from, the stage message presets, and the stage actions (layouts per screen, stage
// messages, editing a layout). Loaded with the core session's first lists (`load`) and
// refetched here on `documentChanged` (`attach`).

import type { PresentationOp, StageLayoutEntry, StageLayoutTemplate } from "$protocol";
import type { Connection, DocumentChangedMessage } from "../server";
import { parsePresets } from "../pages/stage/stage";
import { errorText } from "./errors";
import { changes } from "./events";
import { creds } from "./prefs";
import { session } from "./session.svelte";

class StageState {
  /** Stage layouts and the built-in templates new ones start from. */
  stageLayoutDocs = $state.raw<StageLayoutEntry[]>([]);
  stageTemplates = $state.raw<StageLayoutTemplate[]>([]);
  /** Stage message presets (Stage tab), remembered per browser. */
  stagePresets = $state<string[]>(parsePresets(creds.pref("stagePresets")));
  /** The Stage page's selected stage screen (null = the first) and the layout it edits when no
   * stage screen shows one (null = the selected screen's). */
  pickedScreen = $state<string | null>(null);
  pickedLayout = $state<string | null>(null);

  /** Called once per connection (session.begin): follows stage layout changes. */
  attach(): () => void {
    return changes.on("documentChanged", (e) => {
      if (e.kind === "stageLayout") void session.refetch("stageLayout", (conn) => this.refetchChanged(conn, e.id, e.msg));
    });
  }

  private async refetchChanged(conn: Connection, id: string, msg?: DocumentChangedMessage) {
    await this.load();
    if (session.conn !== conn) return;
    session.oneSlideChanged(id, msg);
  }

  /** (Re)loads the stage layouts, and the templates when asked and none are loaded yet.
   * Missing routes (an older server) leave the lists empty. */
  async load(withTemplates = false) {
    const conn = session.conn;
    if (!conn) return;
    const r = conn.rest;
    const [sl, tpl] = await Promise.allSettled([
      r.stageLayouts(),
      withTemplates && !this.stageTemplates.length ? r.stageLayoutTemplates() : Promise.reject(),
    ]);
    if (session.conn !== conn) return;
    if (sl.status === "fulfilled") this.stageLayoutDocs = sl.value;
    if (tpl.status === "fulfilled") this.stageTemplates = tpl.value;
  }

  /** Opens a stage layout in the Edit view (one-slide document, as a prop). */
  async openStageLayout(id: string) {
    const conn = session.conn;
    if (!conn) return;
    conn.rest.stageLayoutDocuments.add(id);
    try {
      const p = await conn.rest.presentation(id);
      session.showOneSlide(p, true);
    } catch (e) {
      conn.pushNotice("error", `Could not open the stage layout: ${errorText(e)}`);
    }
  }

  /** Creates a stage layout (from a built-in template, or blank) and returns it. */
  async createStageLayout(template: string | null, name?: string): Promise<StageLayoutEntry | null> {
    const conn = session.conn;
    if (!conn) return null;
    try {
      const e = await conn.rest.createStageLayout({ ...(template ? { template } : {}), ...(name ? { name } : {}) });
      await this.load();
      return e;
    } catch (err) {
      conn.pushNotice("error", `Could not create the stage layout: ${errorText(err)}`);
      return null;
    }
  }

  /**
   * "Edit Layouts…": opens the layout the first stage screen shows (else the first layout) in
   * the Edit view; with no layouts yet, makes one from the built-in "Current / Next" first.
   */
  async editStageLayouts() {
    const shown = session.showState?.stageScreens?.find((s) => s.layout)?.layout;
    let id = shown ?? this.stageLayoutDocs[0]?.id ?? null;
    if (!id) id = (await this.createStageLayout("currentNext"))?.id ?? null;
    if (id) await this.openStageLayout(id);
  }

  /** Chooses the layout a stage screen shows (`null` = its default). */
  setStageLayout(screen: string, layout: string | null) {
    return session.run({ type: "setStageLayout", screen, layout });
  }

  /** Shows a stage message (performers only). */
  stageMessage(text: string, flash = false) {
    const t = text.trim();
    if (!t) return Promise.resolve(false);
    return session.run({ type: "stageMessage", text: t, ...(flash ? { flash: true } : {}) });
  }

  stageMessageClear() {
    return session.run({ type: "stageMessageClear" });
  }

  /** Edits a stage layout's slide with edit ops (undoable, as in the Edit view) and reloads. */
  async editStageLayout(id: string, label: string, ops: PresentationOp[]): Promise<boolean> {
    const conn = session.conn;
    if (!conn || !ops.length) return false;
    conn.rest.stageLayoutDocuments.add(id);
    const r = await session.edit(label, (e) => e.editPresentation(id, label, ops));
    await this.load();
    return r !== null;
  }

  /** Replaces the stage message presets (remembered in this browser). */
  setStagePresets(list: string[]) {
    this.stagePresets = list;
    creds.setPref("stagePresets", JSON.stringify(list));
  }
}

export const stageState = new StageState();
