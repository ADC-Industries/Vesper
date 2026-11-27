// The PowerPoint / Keynote / Google Slides import (Oriel menu ▸ Import): the chosen file, the
// job on the Codex box and its report. The job runs on the server; this follows it while Oriel
// is open (the dialog may close meanwhile: a notice says when it's done).

import type { PptxImportJob } from "$protocol";
import { CodexBoxError } from "../server/codexBox";
import type { RestClient } from "../server/rest";
import { fileProblem, finished, importJob, nameFromFile, pollFailure, startImport } from "../server/slidesImport";
import { errorText } from "./errors";
import { libraryPage, libraryState } from "./library.svelte";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";
import { siteState } from "./site.svelte";

/** The value of the library choice that means "the collection's Imported Slides library". */
export const DEFAULT_LIBRARY = "";

class SlidesImport {
  file = $state.raw<File | null>(null);
  name = $state("");
  library = $state<string>(DEFAULT_LIBRARY);
  problem = $state<string | null>(null);
  job = $state.raw<PptxImportJob | null>(null);
  busy = $state(false);
  private rest: RestClient | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Failed polls in a row. */
  private failures = 0;
  /** The connection the import was started on (a disconnect or server switch stops following it). */
  private conn: unknown = null;

  /** Opens the dialog (as it was: a running import keeps showing). */
  open() {
    if (!shell.closeDialogs()) return;
    if (finished(this.job)) this.reset();
    shell.showSlidesImport = true;
  }

  close() {
    shell.showSlidesImport = false;
  }

  /** Back to choosing a file. */
  reset() {
    this.stop();
    this.file = null;
    this.name = "";
    this.problem = null;
    this.job = null;
    this.busy = false;
  }

  choose(file: File | null) {
    this.file = file;
    this.problem = file ? fileProblem(file.name, file.size) : null;
    if (file) this.name = nameFromFile(file.name);
  }

  async start() {
    const file = this.file;
    if (!file || this.problem || this.busy) return;
    this.busy = true;
    try {
      const collection = siteState.codex?.collection ?? null;
      const job = await libraryPage.onCodexBox((rest, auth) => {
        this.rest = rest;
        return startImport(rest, auth, file, file.name, {
          collection,
          library: this.library || null,
          name: this.name,
        });
      });
      this.job = job;
      this.failures = 0;
      this.conn = session.conn;
      this.poll();
    } catch (e) {
      if (!(e instanceof CodexBoxError && e.reason !== "unknown")) this.problem = errorText(e);
    } finally {
      this.busy = false;
    }
  }

  private stop() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private poll(delay = 400) {
    this.stop();
    this.timer = setTimeout(() => void this.tick(), delay);
  }

  private async tick() {
    const job = this.job;
    const rest = this.rest;
    if (!job || !rest) return;
    // Disconnected or switched to another server: back to choosing a file.
    if (session.conn !== this.conn) {
      this.reset();
      return;
    }
    try {
      const j = await importJob(rest, job.id);
      if (this.job?.id !== j.id) return;
      this.job = j;
      this.failures = 0;
      if (finished(j)) {
        this.done(j);
        return;
      }
    } catch (e) {
      if (this.job?.id !== job.id) return;
      this.failures += 1;
      const f = pollFailure(e, this.failures);
      if ("lost" in f) {
        const failed: PptxImportJob = { ...job, state: "failed", error: f.lost };
        this.job = failed;
        this.done(failed);
        return;
      }
      this.poll(f.delay);
      return;
    }
    this.poll();
  }

  private done(j: PptxImportJob) {
    this.stop();
    const conn = session.conn;
    if (j.state === "done" && j.report) {
      if (!shell.showSlidesImport) conn?.pushNotice("info", `Imported “${j.report.name}”: ${j.report.slides.length} slides.`, `pptx:${j.id}`);
    } else if (!shell.showSlidesImport) {
      conn?.pushNotice("error", `The PowerPoint import failed: ${j.error ?? "unknown error"}`, `pptx:${j.id}`);
    }
  }

  /** Opens the imported presentation on the Show page. */
  async openResult() {
    const r = this.job?.report;
    if (!r) return;
    this.close();
    // (On a show box its copy may still be catching up with the new library.)
    if (!libraryState.libraries.some((l) => l.id === r.library) && session.conn) libraryState.libraries = await session.conn.rest.libraries();
    if (libraryState.libraries.some((l) => l.id === r.library)) await libraryState.selectLibrary(r.library);
    await session.openPresentation(r.presentation);
  }
}

export const slidesImport = new SlidesImport();
