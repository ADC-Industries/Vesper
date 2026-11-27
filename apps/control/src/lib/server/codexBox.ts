// The side connection to the box that holds the site's Codex (docs/ORIEL-REDESIGN-PLAN.md 2.5).
//
// On a show box, uploads and Codex management answer `409 codexElsewhere` naming the Codex box
// (`CodexBox {serverId, name, addresses}`). The site key Oriel holds works on every server of
// the site, so Oriel asks once ("Uploads and collections are managed on *name*. Use it for
// this?") and then sends those requests there over REST only. If none of the addresses
// answers, it says so (and the page offers to switch Oriel's main connection). Show-time edits
// keep going through the show box. Plain TypeScript, unit tested.

import type { CodexBox } from "$protocol";
import { ApiRequestError, RestClient } from "./rest";

export interface CodexBoxPrompts {
  /** Asks once per Codex box: "Uploads and collections are managed on *name*. Use it for this?" */
  ask(box: CodexBox): Promise<boolean>;
  /** None of the box's addresses answered (says so; may offer to switch the main connection). */
  unreachable(box: CodexBox): Promise<void>;
  /** This client holds no site key for the box (an Oriel on a show box itself, which that box
   *  trusts without one): says so, and may offer to pair with the Codex box. Without it, the
   *  request goes there without a key (and is refused). */
  unpaired?(box: CodexBox): Promise<void>;
}

export interface CodexBoxOptions {
  /** The site key (the same one the main connection sends; given the Codex box, the one this
   *  client got there when the main connection has none). */
  token: (box?: CodexBox) => string | null;
  /** Sent as `X-Client-Id`, like the main connection's. */
  clientId?: () => string | null;
  prompts: CodexBoxPrompts;
  fetch?: typeof fetch;
  /** Per address, while looking for one that answers (ms). */
  probeTimeoutMs?: number;
}

/** The operator said no, or the Codex box couldn't be reached: nothing was sent there. */
export class CodexBoxError extends Error {
  constructor(
    public readonly reason: "declined" | "unreachable" | "unpaired" | "unknown",
    public readonly box: CodexBox | null,
  ) {
    super(
      reason === "declined"
        ? "Not sent to the Codex box"
        : reason === "unreachable"
          ? `Could not reach ${boxName(box)}`
          : reason === "unpaired"
            ? `This computer isn't paired with ${boxName(box)}`
            : "This box doesn't know where the site's Codex is yet",
    );
    this.name = "CodexBoxError";
  }
}

/** The Codex box's name for messages ("the site's Codex box" before the show box heard it). */
export function boxName(box: CodexBox | null, known?: string | null): string {
  return box?.name?.trim() || known?.trim() || "the site's Codex box";
}

export class CodexBoxLink {
  /** The Codex box in use, once the operator agreed and it answered. */
  box: CodexBox | null = null;
  private client: RestClient | null = null;
  private accepted = new Set<string>();
  private listeners = new Set<(box: CodexBox | null) => void>();

  constructor(private readonly opts: CodexBoxOptions) {}

  /** Called with the box in use (null after `reset`): the page's "Managed on *name*" note. */
  subscribe(fn: (box: CodexBox | null) => void): () => void {
    this.listeners.add(fn);
    fn(this.box);
    return () => this.listeners.delete(fn);
  }

  /**
   * Runs `fn` on the main server; on `409 codexElsewhere` asks (once per box), finds an
   * address of the Codex box that answers and runs `fn` there instead. Throws
   * `CodexBoxError` when the operator declines or no address answers; other errors as they come.
   */
  async run<T>(main: RestClient, fn: (rest: RestClient) => Promise<T>): Promise<T> {
    try {
      return await fn(main);
    } catch (e) {
      if (!(e instanceof ApiRequestError) || e.code !== "codexElsewhere") throw e;
      const box = e.codex;
      if (!box) throw new CodexBoxError("unknown", null);
      // The box would refuse a request without a key: say so (and offer to pair) instead.
      if (this.opts.prompts.unpaired && !this.opts.token(box)) {
        await this.opts.prompts.unpaired(box);
        throw new CodexBoxError("unpaired", box);
      }
      if (!this.accepted.has(box.serverId)) {
        if (!(await this.opts.prompts.ask(box))) throw new CodexBoxError("declined", box);
        this.accepted.add(box.serverId);
      }
      let side = this.client && this.box?.serverId === box.serverId ? this.client : null;
      if (side) {
        try {
          return await fn(side);
        } catch (err) {
          if (!(err instanceof ApiRequestError) || (err.code !== "network" && err.code !== "timeout")) throw err;
          // The address stopped answering: look again below.
        }
      }
      side = await this.connect(box);
      if (!side) {
        this.set(null, null);
        await this.opts.prompts.unreachable(box);
        throw new CodexBoxError("unreachable", box);
      }
      return fn(side);
    }
  }

  /** The first of the box's addresses that answers as that server (its ID), or null. */
  async connect(box: CodexBox): Promise<RestClient | null> {
    const token = () => this.opts.token(box);
    for (const address of box.addresses) {
      const rest = new RestClient({
        baseUrl: address,
        token,
        fetch: this.opts.fetch,
        timeoutMs: this.opts.probeTimeoutMs ?? 4000,
      });
      try {
        const info = await rest.serverInfo();
        if (info.id !== box.serverId) continue;
      } catch {
        continue;
      }
      const client = new RestClient({ baseUrl: address, token, fetch: this.opts.fetch });
      client.clientId = this.opts.clientId?.() ?? null;
      this.set(box, client);
      return client;
    }
    return null;
  }

  /** Forgets the box and the answer (another main server). */
  reset(): void {
    this.accepted.clear();
    this.set(null, null);
  }

  private set(box: CodexBox | null, client: RestClient | null) {
    this.box = box;
    this.client = client;
    for (const fn of this.listeners) fn(box);
  }
}
