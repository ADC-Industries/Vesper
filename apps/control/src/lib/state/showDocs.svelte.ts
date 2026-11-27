// The Show page's documents (Svelte 5 runes): messages (and the themes their templates use),
// props, the Messages tab's selected message, and the prop actions. Loaded with the core
// session's first lists (`load`) and refetched here on `documentChanged` (`attach`).

import type { MessageEntry, PropEntry, SlideRef, ThemeSummary } from "$protocol";
import type { Connection, DocumentChangedMessage } from "../server";
import { errorText } from "./errors";
import { changes } from "./events";
import { session } from "./session.svelte";

type Kind = "message" | "prop" | "theme";

class ShowDocsState {
  messageDocs = $state.raw<MessageEntry[]>([]);
  propDocs = $state.raw<PropEntry[]>([]);
  /** The themes message templates use. */
  themes = $state.raw<ThemeSummary[]>([]);
  /** The Messages tab's selected message. */
  selectedMessage = $state<string | null>(null);

  /** Called once per connection (session.begin): follows message, prop and theme changes. */
  attach(): () => void {
    return changes.on("documentChanged", (e) => {
      if (e.kind === "message" || e.kind === "prop" || e.kind === "theme") {
        const kind = e.kind;
        void session.refetch(kind, (conn) => this.refetchChanged(conn, kind, e.id, e.msg));
      }
    });
  }

  private async refetchChanged(conn: Connection, kind: Kind, id: string, msg?: DocumentChangedMessage) {
    await this.load(kind);
    if (session.conn !== conn) return;
    if (kind === "prop" || kind === "theme") session.oneSlideChanged(id, msg);
  }

  /** (Re)loads messages, props and themes (one kind, or all). Missing routes (an older server)
   * leave the lists empty. */
  async load(kind?: Kind) {
    const conn = session.conn;
    if (!conn) return;
    const r = conn.rest;
    const want = (k: Kind) => !kind || kind === k;
    const [m, p, th] = await Promise.allSettled([
      want("message") ? r.messages() : Promise.reject(),
      want("prop") ? r.props() : Promise.reject(),
      want("theme") ? r.themes() : Promise.reject(),
    ]);
    if (session.conn !== conn) return;
    if (m.status === "fulfilled") {
      this.messageDocs = m.value;
      if (this.selectedMessage && !m.value.some((x) => x.id === this.selectedMessage)) this.selectedMessage = null;
      if (!this.selectedMessage) this.selectedMessage = m.value[0]?.id ?? null;
    }
    if (p.status === "fulfilled") this.propDocs = p.value;
    if (th.status === "fulfilled") this.themes = th.value;
  }

  /** Creates a prop (blank, or a copy of a slide) and returns it. */
  async createProp(name: string, fromSlide: SlideRef | null): Promise<PropEntry | null> {
    const conn = session.conn;
    if (!conn) return null;
    try {
      const e = await conn.rest.createProp({ name, ...(fromSlide ? { fromSlide } : {}) });
      await this.load("prop");
      return e;
    } catch (err) {
      conn.pushNotice("error", `Could not create the prop: ${errorText(err)}`);
      return null;
    }
  }

  /** Opens a prop in the Edit view as a one-slide document (prop-document mode). */
  async openProp(id: string) {
    const conn = session.conn;
    if (!conn) return;
    conn.rest.propDocuments.add(id);
    try {
      const p = await conn.rest.presentation(id);
      session.showOneSlide(p);
    } catch (e) {
      conn.pushNotice("error", `Could not open the prop: ${errorText(e)}`);
    }
  }
}

export const showDocsState = new ShowDocsState();
