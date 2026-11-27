// The timers' documents (Svelte 5 runes): the timer list (`GET /timers`; their running state is
// in the show state). Loaded with the core session's first lists (`load`) and refetched here on
// `documentChanged` (`attach`). The Timers page owns it; the Show page's messages and the slide
// editor read it.

import type { TimerEntry, TimerKind } from "$protocol";
import { nextTimerName } from "../pages/timers/timers";
import { errorText } from "./errors";
import { changes } from "./events";
import { creds } from "./prefs";
import { session } from "./session.svelte";

/** The fields of a timer the Timers page changes. */
export type TimerPatch = { name?: string; kind?: TimerKind; allowOverrun?: boolean };

class TimersState {
  timerDocs = $state.raw<TimerEntry[]>([]);
  /** The Timers page's selected timer (null = the first). */
  selected = $state<string | null>(null);
  /** The system clock's format on the Timers page, remembered per browser (G12: the server has no
   * box-wide clock format). */
  clock24 = $state(creds.pref("clock24") === "1");

  /** Called once per connection (session.begin): follows timer changes. */
  attach(): () => void {
    return changes.on("documentChanged", (e) => {
      if (e.kind === "timer") void session.refetch("timer", () => this.load());
    });
  }

  /** (Re)loads the timers. A missing route (an older server) leaves the list empty. */
  async load() {
    const conn = session.conn;
    if (!conn) return;
    const [t] = await Promise.allSettled([conn.rest.timers()]);
    if (session.conn !== conn) return;
    if (t.status === "fulfilled") this.timerDocs = t.value;
  }

  setClock24(on: boolean) {
    this.clock24 = on;
    creds.setPref("clock24", on ? "1" : "0");
  }

  private async write<T>(what: string, fn: () => Promise<T>): Promise<T | null> {
    const conn = session.conn;
    if (!conn) return null;
    try {
      return await fn();
    } catch (e) {
      conn.pushNotice("error", `${what} failed: ${errorText(e)}`);
      return null;
    } finally {
      await this.load();
    }
  }

  /** Creates a timer ("Timer N", or a copy of `from`) and selects it. */
  async create(from?: TimerEntry): Promise<TimerEntry | null> {
    const conn = session.conn;
    if (!conn) return null;
    const doc = from
      ? { name: `${from.name ?? "Timer"} copy`, ...(from.kind ? { kind: from.kind } : {}), ...(from.allowOverrun ? { allowOverrun: true } : {}) }
      : { name: nextTimerName(this.timerDocs.map((t) => t.name ?? "")), kind: { type: "countdown", seconds: 300 } as TimerKind };
    const e = await this.write(from ? "Duplicate" : "New timer", () => conn.rest.createTimer(doc));
    if (e) this.selected = e.id;
    return e;
  }

  /** Saves changed fields of a timer (whole-document PUT with its revision). */
  update(t: TimerEntry, patch: TimerPatch, what = "Change timer") {
    const conn = session.conn;
    if (!conn) return Promise.resolve(null);
    return this.write(what, () => conn.rest.updateTimer({ ...t, ...patch }, t.revision));
  }

  remove(t: TimerEntry) {
    const conn = session.conn;
    if (!conn) return Promise.resolve(null);
    if (this.selected === t.id) this.selected = null;
    return this.write("Delete", () => conn.rest.deleteTimer(t.id, t.revision));
  }
}

export const timersState = new TimersState();
