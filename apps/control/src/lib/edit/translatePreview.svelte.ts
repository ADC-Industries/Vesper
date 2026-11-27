// A machine-translation preview (docs/DUAL-LANGUAGE-PLAN.md §5), shared by Reflow ("Translate
// to…", a slide's "Translate this slide…") and the slide editor (Inspector ▸ Slide ▸ Second
// language): `POST /translate/preview` starts a job, polled until it is done; its proposals are
// shown labelled until Keep (one `setSlideTranslation` per slide, marked as a machine
// translation; the proposals are dropped only once the edit is accepted) or Discard (nothing
// written). A job nobody follows any more (the preview closed, replaced or disposed) is
// cancelled on the server, so it never holds the server's one translation slot.

import type { PresentationOp, TranslateJob, TranslatePreview as TranslateRequest, TranslatedSlide } from "$protocol";
import type { Connection } from "../server";
import { keepTranslationOps } from "./language";

export class TranslatePreview {
  /** The running or finished job (null: none). */
  job = $state<TranslateJob | null>(null);
  /** The finished job's proposals by slide. */
  proposals = $state<Record<string, TranslatedSlide>>({});
  /** Keep is sending the proposals. */
  keeping = $state(false);
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped by every cancel: a start whose answer arrives after one is no longer wanted. */
  #seq = 0;
  #disposed = false;

  constructor(
    private readonly conn: Connection,
    private readonly onDone: (job: TranslateJob) => void = () => {},
  ) {}

  /** Proposals are waiting for Keep or Discard. */
  get proposing(): boolean {
    return Object.keys(this.proposals).length > 0;
  }

  get count(): number {
    return Object.keys(this.proposals).length;
  }

  /** The job is still working (queued, downloading the model or translating). */
  get running(): boolean {
    const s = this.job?.status;
    return s === "queued" || s === "downloading" || s === "translating";
  }

  #stop() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
  }

  async #poll(id: string) {
    try {
      const j = await this.conn.rest.translateJob(id);
      if (this.job?.id !== id) return;
      this.job = j;
      if (j.status === "done") {
        const next: Record<string, TranslatedSlide> = {};
        for (const s of j.slides) next[s.slide] = s;
        this.proposals = next;
        if (!j.slides.length) this.conn.pushNotice("info", "Nothing to translate: the slides have no text.");
        this.onDone(j);
        return;
      }
      if (j.status === "failed" || j.status === "cancelled") return;
    } catch (e) {
      if (this.job?.id === id) this.job = { ...this.job, status: "failed", detail: e instanceof Error ? e.message : String(e) };
      return;
    }
    this.#timer = setTimeout(() => void this.#poll(id), 400);
  }

  /** Starts a preview (any earlier one is dropped). */
  async start(req: TranslateRequest): Promise<void> {
    this.cancel();
    if (this.#disposed) return;
    const n = this.#seq;
    try {
      const j = await this.conn.rest.translatePreview(req);
      if (this.#disposed || n !== this.#seq) {
        // Cancelled, replaced or disposed while it started: nobody follows it.
        void this.conn.rest.cancelTranslateJob(j.id).catch(() => {});
        return;
      }
      this.job = j;
      this.#timer = setTimeout(() => void this.#poll(j.id), 300);
    } catch (e) {
      this.conn.pushNotice("error", `Could not translate: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  /** Stops a running job (the server stops it too) or closes a finished one. */
  cancel(): void {
    const j = this.job;
    const running = this.running;
    this.#seq++;
    this.#stop();
    this.job = null;
    this.proposals = {};
    if (j && running) void this.conn.rest.cancelTranslateJob(j.id).catch(() => {});
  }

  /**
   * Keep: `run` sends the ops that make the proposals the slides' second texts (only for the
   * slides `exists` still finds), with their language, and says whether they were accepted.
   * The proposals are dropped only then: a refused edit leaves them to keep again.
   */
  async keep(run: (ops: PresentationOp[], language: string) => Promise<boolean>, exists: (slide: string) => boolean = () => true): Promise<boolean> {
    const job = this.job;
    if (!job || this.keeping) return false;
    const ops = keepTranslationOps(Object.values(this.proposals).filter((s) => exists(s.slide)));
    this.keeping = true;
    let ok = false;
    try {
      ok = ops.length ? await run(ops, job.to) : true;
    } catch {
      ok = false;
    } finally {
      this.keeping = false;
    }
    if (ok && this.job === job) {
      this.proposals = {};
      this.job = null;
    }
    return ok;
  }

  /** Discard: nothing is written. */
  discard(): void {
    this.cancel();
  }

  /** The preview's owner goes away: a running job is cancelled on the server too. */
  dispose(): void {
    this.cancel();
    this.#disposed = true;
  }
}
