// This Vesper's details (G9: `GET /server/details`), read while something shows them (the
// status bar, Site ▸ Devices) and again every minute for the uptime.

import type { ServerDetails } from "$protocol";
import { session } from "./session.svelte";

class DetailsState {
  details = $state.raw<ServerDetails | null>(null);
  private users = 0;
  private timer: ReturnType<typeof setInterval> | null = null;

  async load() {
    const conn = session.conn;
    if (!conn) return;
    try {
      const d = await conn.rest.serverDetails();
      if (session.conn === conn) this.details = d;
    } catch {
      /* an older server: nothing shown */
    }
  }

  /** Something shows the details: read them now and every minute until it stops. */
  use(): () => void {
    this.users++;
    void this.load();
    this.timer ??= setInterval(() => void this.load(), 60_000);
    return () => {
      if (--this.users > 0) return;
      if (this.timer) clearInterval(this.timer);
      this.timer = null;
    };
  }
}

export const serverDetails = new DetailsState();
