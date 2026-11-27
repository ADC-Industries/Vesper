// The Reflow edit queue: typing becomes debounced `setSlideText` ops with `expectText`, and
// structural edits (slide breaks, merges, group changes) run in order behind them. Edits are
// never dropped silently:
//
// - one request at a time, in the order the operator made them;
// - typing in a slide is debounced (`DEBOUNCE_MS`) and coalesced while it waits;
// - a network failure keeps the job at the head of the queue (status `offline`) and retries
//   with backoff, or at once when `kick()` says the connection is back;
// - a 409 on a text edit (`expectText` mismatch: someone else changed the slide) pauses that
//   slide and reports a conflict; `resolve(key, "mine" | "theirs")` continues. With no editor
//   left to ask (retired, or nobody listens for conflicts) it is reported through `failed`,
//   with the text that wasn't saved;
// - after a structural edit the queue holds until the caller has refetched the presentation
//   and updated the slides' base texts (`release()`, or after `HOLD_MS`). Typing in a slide the
//   edit split or merged, made before the editor caught up (still on the old text), is moved
//   onto the slide's new text (`rebaseText`); typing in a slide that is gone is reported;
// - a `423 locked` (someone else took over editing, G8) stops the queue with the typing kept
//   (status `locked`): `unlock()` sends it after this client took the lock back,
//   `lockedTexts()` / `discardLocked()` let the operator copy it and let it go. A retired
//   queue reports it with the text.
// - second-language typing (`setSecond`, batch 16) is a job of its own: debounced and
//   coalesced like typing, kept and retried offline, kept while locked, waited for by `flush`
//   (undo) and still sent by a retired queue (R3-3, R3-4, R3-5). Its op is built when it is
//   sent (`build`), from the slide as the editor knows it then.

import type { PresentationEditResult, PresentationOp } from "$protocol";

export const DEBOUNCE_MS = 400;
export const HOLD_MS = 3000;
const RETRY_MS = [1000, 2000, 5000, 10000];

export type SaveStatus = "saved" | "saving" | "offline" | "conflict" | "locked";

export interface TextJob {
  kind: "text";
  /** The slide. */
  key: string;
  text: string;
  readyAt: number;
}

export interface SecondJob {
  kind: "second";
  /** The slide. */
  key: string;
  text: string;
  readyAt: number;
  /** The op that saves `text` (`null`: the slide is gone). */
  build: (text: string) => PresentationOp | null;
}

export interface OpsJob {
  kind: "ops";
  label: string;
  ops: PresentationOp[];
  done: (r: PresentationEditResult | null, err?: unknown) => void;
  /** The text the editor showed for each slide the ops reshape, when they were queued. */
  before: Map<string, string | undefined>;
}

/** Slides whose text a structural op changes (split, merge, set text). */
function reshapedSlides(ops: PresentationOp[]): string[] {
  const out: string[] = [];
  for (const op of ops) {
    if (op.type === "splitSlide" || op.type === "setSlideText") out.push(op.slide);
    else if (op.type === "mergeSlides") out.push(op.first, op.second);
  }
  return out;
}

/**
 * Moves an edit from one text onto another: `typed` is `before` with an edit (one changed
 * stretch), `after` what the server holds now (a split kept the start or the end of `before`,
 * a merge put something before or after it). The edit lands at the same place in `after`,
 * or `null` when that place isn't in `after` (it went to another slide).
 */
export function rebaseText(before: string, typed: string, after: string): string | null {
  if (typed === before) return after;
  // The edit: before[a, end) became `ins`.
  let a = 0;
  while (a < before.length && a < typed.length && before[a] === typed[a]) a++;
  let b = 0;
  while (b < before.length - a && b < typed.length - a && before[before.length - 1 - b] === typed[typed.length - 1 - b]) b++;
  const end = before.length - b;
  const ins = typed.slice(a, typed.length - b);
  // Within the start that `after` kept (a split keeps the start; a merge adds after it).
  let p = 0;
  while (p < before.length && p < after.length && before[p] === after[p]) p++;
  if (end <= p) return after.slice(0, a) + ins + after.slice(end);
  // Within an end that `after` kept (strictly inside it: an edit at its edge is ambiguous).
  let q = 0;
  while (q < before.length && q < after.length && before[before.length - 1 - q] === after[after.length - 1 - q]) q++;
  const d = after.length - before.length;
  if (q > 0 && (a > before.length - q || q === before.length)) return after.slice(0, a + d) + ins + after.slice(end + d);
  return null;
}

type Job = TextJob | SecondJob | OpsJob;

/** Typing jobs (first or second language): debounced, kept offline and while locked. */
const typing = (j: Job | null): j is TextJob | SecondJob => !!j && (j.kind === "text" || j.kind === "second");

export interface Conflict {
  key: string;
  mine: string;
  theirs: string;
}

/** How the queue reaches the server (injected so it can be tested without one). */
export interface QueueTransport {
  send(ops: PresentationOp[]): Promise<PresentationEditResult>;
  /** Called after every successful send (for the undo stack). */
  applied(label: string, result: PresentationEditResult, mergeKey?: string): void;
  /** Network-type failures are retried; everything else is reported. */
  isOffline(err: unknown): boolean;
  /** The server text of a 409 `expectText` conflict, or null when it is another error. */
  conflictText(err: unknown): string | null;
  /** Other failures (the job is dropped). */
  failed(label: string, err: unknown): void;
  /** Someone else holds the document's edit lock (`423 locked`): the typing is kept. */
  isLocked?(err: unknown): boolean;
}

export interface QueueOptions {
  now?: () => number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (t: unknown) => void;
}

export class EditQueue {
  private jobs: Job[] = [];
  /** Server text per slide: what `expectText` says. */
  private base = new Map<string, string>();
  /** The server text last given for a slide while it had a local edit (see `setBase`). */
  private seen = new Map<string, string>();
  private inflight: Job | null = null;
  private paused = new Map<string, Conflict>();
  /** Slides a structural edit reshaped, with the text the editor showed before it: typing
   *  waiting for them is moved onto their new text at the next `setBase`. */
  private reshaped = new Map<string, string | undefined>();
  private holdUntil = 0;
  private timer: unknown = null;
  private attempt = 0;
  private offline = false;
  /** Refused with `423 locked`: nothing is sent until `unlock()`. */
  private locked = false;
  private listeners = new Set<(s: SaveStatus) => void>();
  private conflictListeners = new Set<(c: Conflict | null) => void>();
  private idleWaiters: (() => void)[] = [];
  private now: () => number;
  private setTimer: (fn: () => void, ms: number) => unknown;
  private clearTimer: (t: unknown) => void;

  constructor(
    private readonly t: QueueTransport,
    opts: QueueOptions = {},
  ) {
    this.now = opts.now ?? (() => Date.now());
    this.setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearTimer = opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  }

  // ---- inputs ------------------------------------------------------------------------------

  /** The slide's text on the server (after a fetch). Ignored while the slide has local edits. */
  setBase(key: string, text: string): void {
    if (this.reshaped.has(key)) return this.rebase(key, text);
    if (this.hasPending(key)) {
      // Remembered for when the local edit is let go (`discardLocked`, D10-5).
      this.seen.set(key, text);
      return;
    }
    this.seen.delete(key);
    this.base.set(key, text);
  }

  /** The slide's new server text after a structural edit: waiting typing moves onto it. */
  private rebase(key: string, text: string) {
    const before = this.reshaped.get(key);
    this.reshaped.delete(key);
    this.seen.delete(key);
    this.base.set(key, text);
    for (const j of [...this.jobs]) {
      if (j.kind !== "text" || j.key !== key) continue;
      const moved = before === undefined ? null : rebaseText(before, j.text, text);
      if (moved !== null) j.text = moved;
      else {
        this.jobs.splice(this.jobs.indexOf(j), 1);
        this.t.failed("Typing", new Error("it was typed while the slide was split or merged; type it again"));
      }
    }
  }

  /** The server text we know for a slide. */
  baseOf(key: string): string | undefined {
    return this.base.get(key);
  }

  /** The operator typed: (re)schedules a text edit of the slide. */
  setText(key: string, text: string): void {
    const last = this.jobs[this.jobs.length - 1];
    const waiting = this.jobs.find((j): j is TextJob => j.kind === "text" && j.key === key);
    const readyAt = this.now() + DEBOUNCE_MS;
    // Coalesce with a waiting edit of the same slide, unless a structural edit sits behind it.
    if (waiting && (last === waiting || !this.jobs.slice(this.jobs.indexOf(waiting)).some((j) => j.kind === "ops"))) {
      waiting.text = text;
      waiting.readyAt = readyAt;
    } else {
      this.jobs.push({ kind: "text", key, text, readyAt });
    }
    this.pump();
  }

  /** The operator typed a slide's second-language text: (re)schedules saving it. */
  setSecond(key: string, text: string, build: SecondJob["build"]): void {
    const last = this.jobs[this.jobs.length - 1];
    const waiting = this.jobs.find((j): j is SecondJob => j.kind === "second" && j.key === key);
    const readyAt = this.now() + DEBOUNCE_MS;
    if (waiting && (last === waiting || !this.jobs.slice(this.jobs.indexOf(waiting)).some((j) => j.kind === "ops"))) {
      waiting.text = text;
      waiting.readyAt = readyAt;
      waiting.build = build;
    } else {
      this.jobs.push({ kind: "second", key, text, readyAt, build });
    }
    this.pump();
  }

  /** Queues a structural edit; earlier typing is sent first. Resolves with the server's answer. */
  enqueueOps(label: string, ops: PresentationOp[]): Promise<PresentationEditResult | null> {
    return new Promise((resolve, reject) => {
      for (const j of this.jobs) if (typing(j)) j.readyAt = 0;
      // The text the slide has before the split or merge: typing waiting, else typing being
      // saved right now (D10-3: it is in the server's text already), else the server's.
      const inflight = (k: string) => (this.inflight?.kind === "text" && this.inflight.key === k ? this.inflight.text : undefined);
      const before = new Map(reshapedSlides(ops).map((k) => [k, this.pendingText(k) ?? inflight(k) ?? this.base.get(k)] as const));
      this.jobs.push({ kind: "ops", label, ops, done: (r, e) => (e ? reject(e) : resolve(r)), before });
      this.pump();
    });
  }

  /** The connection is back: retry now. */
  kick(): void {
    this.attempt = 0;
    this.pump(true);
  }

  /** The caller refetched after a structural edit (and gave the slides' texts to `setBase`):
   *  continue. A reshaped slide it didn't give is gone (merged away): typing there can't be
   *  saved, and says so. */
  release(): void {
    if (this.holdUntil) {
      for (const key of this.reshaped.keys()) {
        const n = this.jobs.length;
        this.jobs = this.jobs.filter((j) => !(j.kind === "text" && j.key === key));
        if (this.jobs.length < n) this.t.failed("Typing", new Error("its slide was merged into another; type it again"));
      }
      this.reshaped.clear();
    }
    this.holdUntil = 0;
    this.pump();
  }

  /** Sends waiting typing at once and resolves when the queue is empty (before undo). */
  flush(): Promise<void> {
    for (const j of this.jobs) if (typing(j)) j.readyAt = 0;
    this.pump(true);
    if (this.idle()) return Promise.resolve();
    return new Promise((r) => this.idleWaiters.push(r));
  }

  /** Answers a conflict: keep the local text (overwrite) or take the server's. */
  resolve(key: string, choice: "mine" | "theirs"): void {
    const c = this.paused.get(key);
    if (!c) return;
    this.paused.delete(key);
    this.base.set(key, c.theirs);
    if (choice === "theirs") {
      this.jobs = this.jobs.filter((j) => !(j.kind === "text" && j.key === key));
    } else {
      const j = this.jobs.find((j): j is TextJob => j.kind === "text" && j.key === key);
      if (j) j.readyAt = 0;
      else this.jobs.unshift({ kind: "text", key, text: c.mine, readyAt: 0 });
    }
    this.emitConflict();
    this.pump();
  }

  /** This client holds the edit lock again: send the kept typing. */
  unlock(): void {
    if (!this.locked) return;
    this.locked = false;
    this.attempt = 0;
    this.pump(true);
  }

  /** The typing kept while someone else holds the lock, per slide (second-language typing
   *  too, `second: true`). */
  lockedTexts(): { key: string; text: string; second?: true }[] {
    if (!this.locked) return [];
    return this.jobs.filter(typing).map((j) => (j.kind === "second" ? { key: j.key, text: j.text, second: true } : { key: j.key, text: j.text }));
  }

  /** The operator let the kept typing go (after copying it): the slides show the server's text. */
  discardLocked(): void {
    if (!this.locked) return;
    const keys = this.jobs.filter((j): j is TextJob => j.kind === "text").map((j) => j.key);
    this.jobs = this.jobs.filter((j) => !typing(j));
    this.locked = false;
    // The slides show the server's text as it is now (the lock holder's edits included), not
    // the text from before this operator typed (D10-5).
    for (const k of keys) {
      const now = this.seen.get(k);
      this.seen.delete(k);
      if (now !== undefined && !this.hasPending(k)) this.base.set(k, now);
    }
    this.pump();
  }

  /** Reports the kept typing with its text and drops it (nobody is left to offer it to). */
  private reportLocked() {
    for (const t of this.lockedTexts())
      this.t.failed(t.second ? "Second-language typing" : "Typing", new Error(`someone else took over editing. What you typed: “${t.text}”`));
    this.jobs = this.jobs.filter((j) => !typing(j));
    this.locked = false;
  }

  // ---- state -------------------------------------------------------------------------------

  /** A local edit of this slide is waiting, in flight or in conflict. */
  hasPending(key: string): boolean {
    const isKey = (j: Job | null) => !!j && j.kind === "text" && j.key === key;
    return this.paused.has(key) || isKey(this.inflight) || this.jobs.some(isKey);
  }

  /** A second-language edit of this slide is waiting or in flight. */
  hasPendingSecond(key: string): boolean {
    const isKey = (j: Job | null) => !!j && j.kind === "second" && j.key === key;
    return isKey(this.inflight) || this.jobs.some(isKey);
  }

  /** The second-language text the operator wants for a slide (a waiting or in-flight edit). */
  pendingSecond(key: string): string | undefined {
    const js = this.jobs.filter((j): j is SecondJob => j.kind === "second" && j.key === key);
    const f = this.inflight;
    return js[js.length - 1]?.text ?? (f?.kind === "second" && f.key === key ? f.text : undefined);
  }

  /** The text the operator wants for a slide (a waiting edit), if any. */
  pendingText(key: string): string | undefined {
    const js = this.jobs.filter((j): j is TextJob => j.kind === "text" && j.key === key);
    return js[js.length - 1]?.text ?? this.paused.get(key)?.mine;
  }

  idle(): boolean {
    if (this.locked) return !this.inflight;
    return !this.inflight && this.jobs.every((j) => j.kind === "text" && this.paused.has(j.key));
  }

  get status(): SaveStatus {
    if (this.paused.size) return "conflict";
    if (this.locked) return "locked";
    if (this.offline) return "offline";
    return this.inflight || this.jobs.length ? "saving" : "saved";
  }

  get conflict(): Conflict | null {
    return this.paused.values().next().value ?? null;
  }

  /** Svelte store contract (status). */
  subscribe(fn: (s: SaveStatus) => void): () => void {
    this.listeners.add(fn);
    fn(this.status);
    return () => this.listeners.delete(fn);
  }

  onConflict(fn: (c: Conflict | null) => void): () => void {
    this.conflictListeners.add(fn);
    return () => this.conflictListeners.delete(fn);
  }

  /**
   * The editor moved on (another presentation opened): stop reporting to it, but send what is
   * still waiting, at once, and keep retrying it while offline. Typing is never dropped.
   */
  retire(): void {
    this.listeners.clear();
    this.conflictListeners.clear();
    // A conflict nobody can answer any more: said, with the text that wasn't saved.
    for (const c of [...this.paused.values()]) this.unanswerable(c.key, c.mine, c.theirs);
    // Typing kept while someone else holds the lock: said too, with its text.
    if (this.locked) this.reportLocked();
    void this.flush();
  }

  /** A text edit refused because someone else changed the slide, with no editor left to ask
   * (it closed, or another presentation opened): reported with the typing, never paused
   * silently. */
  private unanswerable(key: string, mine: string, theirs: string) {
    this.paused.delete(key);
    this.base.set(key, theirs);
    this.jobs = this.jobs.filter((j) => !(j.kind === "text" && j.key === key));
    this.t.failed("Typing", new Error(`someone else changed the slide meanwhile. What you typed: “${mine}”`));
  }

  /** Stops the queue: what is still waiting is NOT sent (see `retire`). */
  dispose(): void {
    if (this.timer) this.clearTimer(this.timer);
    this.timer = null;
    this.listeners.clear();
    this.conflictListeners.clear();
  }

  // ---- engine ------------------------------------------------------------------------------

  private emit() {
    const s = this.status;
    for (const l of this.listeners) l(s);
    if (this.idle()) for (const w of this.idleWaiters.splice(0)) w();
  }

  private emitConflict() {
    const c = this.conflict;
    for (const l of this.conflictListeners) l(c);
    this.emit();
  }

  private schedule(ms: number) {
    if (this.timer) this.clearTimer(this.timer);
    this.timer = this.setTimer(
      () => {
        this.timer = null;
        this.pump(true);
      },
      Math.max(0, ms),
    );
  }

  private pump(retryNow = false) {
    this.emit();
    if (this.inflight) return;
    if (this.locked) return;
    if (this.offline && !retryNow) return;
    const now = this.now();
    if (this.holdUntil > now) return this.schedule(this.holdUntil - now);
    // The first job that is not waiting on a conflict.
    const i = this.jobs.findIndex((j) => !(j.kind === "text" && this.paused.has(j.key)));
    if (i < 0) return;
    const job = this.jobs[i]!;
    if (typing(job) && job.readyAt > now) return this.schedule(job.readyAt - now);
    if (this.timer) {
      this.clearTimer(this.timer);
      this.timer = null;
    }
    this.jobs.splice(i, 1);
    this.inflight = job;
    this.emit();
    void this.send(job);
  }

  private async send(job: Job) {
    let ops: PresentationOp[];
    if (job.kind === "second") {
      const op = job.build(job.text);
      if (!op) {
        this.inflight = null;
        this.t.failed("Second-language typing", new Error(`its slide is gone. What you typed: “${job.text}”`));
        this.pump();
        return;
      }
      ops = [op];
    } else {
      // `setSlideText` in a structural batch expects the text as saved by the typing before it.
      ops =
        job.kind === "text"
          ? [{ type: "setSlideText", slide: job.key, text: job.text, expectText: this.base.get(job.key) }]
          : job.ops.map((op) =>
              op.type === "setSlideText" && op.expectText === undefined && this.base.has(op.slide)
                ? { ...op, expectText: this.base.get(op.slide) }
                : op,
            );
    }
    try {
      const r = await this.t.send(ops);
      this.offline = false;
      this.attempt = 0;
      this.inflight = null;
      if (job.kind === "text") {
        this.base.set(job.key, job.text);
        this.seen.delete(job.key);
        this.t.applied("Typing", r, `text:${job.key}`);
      } else if (job.kind === "second") {
        // One undo step per slide while typing.
        this.t.applied("Second Language", r, `second:${job.key}`);
      } else {
        this.holdUntil = this.now() + HOLD_MS;
        // The server text of these slides changed: typing queued since is rebased when their
        // new text comes (`setBase`).
        for (const [k, before] of job.before) this.reshaped.set(k, before);
        this.t.applied(job.label, r);
        job.done(r);
      }
    } catch (e) {
      this.inflight = null;
      if (this.t.isOffline(e)) {
        // Back to the head of the queue; retry with backoff (or on kick()).
        this.jobs.unshift(job);
        this.offline = true;
        const ms = RETRY_MS[Math.min(this.attempt, RETRY_MS.length - 1)]!;
        this.attempt++;
        this.schedule(ms);
        this.emit();
        return;
      }
      if (typing(job) && this.t.isLocked?.(e)) {
        // Someone else took over editing: keep the typing (and what waits behind it).
        this.jobs.unshift(job);
        this.locked = true;
        if (!this.listeners.size) {
          // Nobody left to offer the typing to: reported with its text and dropped; what
          // waits behind it (a split, a merge) carries on and is answered (D10-6).
          this.reportLocked();
          this.pump();
          return;
        }
        this.emit();
        return;
      }
      const theirs = job.kind === "text" ? this.t.conflictText(e) : null;
      if (job.kind === "text" && theirs !== null && !this.conflictListeners.size) {
        this.unanswerable(job.key, job.text, theirs);
      } else if (job.kind === "text" && theirs !== null) {
        this.jobs.unshift(job);
        this.paused.set(job.key, { key: job.key, mine: job.text, theirs });
        this.emitConflict();
      } else if (job.kind === "ops") {
        this.t.failed(job.label, e);
        job.done(null, e);
      } else if (job.kind === "second") {
        // Reported with the text, never dropped silently.
        const why = e instanceof Error ? e.message : String(e);
        this.t.failed("Second-language typing", new Error(`${why}. What you typed: “${job.text}”`));
      } else {
        this.t.failed("Typing", e);
      }
    }
    this.pump();
  }
}
