// PowerPoint / Keynote / Google Slides import (crates/protocol/README.md, "PowerPoint import"):
// the .pptx goes to the Codex box as the request body, the import runs there as a job, and its
// report says slide by slide what is editable and what was pictured. Plain TypeScript over a
// RestClient (the main connection's, or the side connection to the Codex box), unit tested.

import type { PptxImportJob, PptxImportReport, PptxImportState, PptxSlideReport } from "$protocol";
import { ApiRequestError, type RestClient } from "./rest";

const enc = encodeURIComponent;

/** File endings the server takes. */
export const PPTX_ENDINGS = ["pptx", "ppsx", "potx", "pptm", "ppsm"];

/** The hint shown with the file picker. */
export const EXPORT_HINT =
  "Keynote: File ▸ Export To ▸ PowerPoint. Google Slides: File ▸ Download ▸ Microsoft PowerPoint (.pptx). Then choose the .pptx here.";

/** What's wrong with a chosen file, before it is sent (null: fine). */
export function fileProblem(name: string, size: number): string | null {
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
  if (ext === "key") return "This is a Keynote file. In Keynote choose File ▸ Export To ▸ PowerPoint, then import the .pptx.";
  if (ext === "ppt" || ext === "pps" || ext === "pot") return "This is an old PowerPoint file (.ppt). Open it in PowerPoint and save it as .pptx first.";
  if (ext === "odp") return "This is an OpenDocument presentation. Save it as .pptx (PowerPoint) first.";
  if (!PPTX_ENDINGS.includes(ext)) return "Choose a .pptx file (Keynote and Google Slides: export as .pptx first).";
  if (size <= 0) return "The file is empty.";
  if (size > 1024 ** 3) return "The file is larger than 1 GB.";
  return null;
}

/** The presentation's name from a file name ("Easter 2026.pptx" → "Easter 2026"). */
export function nameFromFile(name: string): string {
  const base = name.split(/[\\/]/).pop() ?? name;
  const dot = base.lastIndexOf(".");
  return (dot > 0 ? base.slice(0, dot) : base).trim();
}

export interface ImportOptions {
  collection?: string | null;
  library?: string | null;
  name?: string | null;
}

/** `POST /codex/imports/pptx`: starts an import of `file` (the body). */
export async function startImport(
  rest: RestClient,
  auth: { token: () => string | null; clientId?: string | null },
  file: Blob,
  fileName: string,
  opts: ImportOptions = {},
): Promise<PptxImportJob> {
  const params: Record<string, string> = { fileName };
  if (opts.collection) params.collection = opts.collection;
  if (opts.library) params.library = opts.library;
  if (opts.name?.trim()) params.name = opts.name.trim();
  const headers: Record<string, string> = { Accept: "application/json", "Content-Type": "application/octet-stream" };
  const t = auth.token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (auth.clientId) headers["X-Client-Id"] = auth.clientId;
  let res: Response;
  try {
    res = await fetch(rest.url("/codex/imports/pptx", params), { method: "POST", headers, body: file });
  } catch {
    throw new ApiRequestError("network", "Could not reach the server", 0);
  }
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    body = undefined;
  }
  if (res.ok) return body as PptxImportJob;
  const b = body as { error?: { code?: string; message?: string; field?: string }; codex?: ApiRequestError["codex"] } | undefined;
  throw new ApiRequestError(
    (b?.error?.code ?? "internal") as ApiRequestError["code"],
    b?.error?.message ?? `HTTP ${res.status}`,
    res.status,
    b?.error?.field ?? null,
    undefined,
    b?.codex ?? null,
  );
}

/** `GET /codex/imports/pptx/{id}`. */
export const importJob = (rest: RestClient, id: string) => rest.get<PptxImportJob>(`/codex/imports/pptx/${enc(id)}`);

/** Most failed polls in a row before Oriel gives up following an import (about 5 minutes). */
export const MAX_POLL_FAILURES = 60;

/**
 * What a failed poll of a job means: try again after `delay` ms (a dropped connection, backing
 * off), or the job is `lost` (the server no longer knows it: jobs live in its memory and a
 * restart forgets them; or the server stayed out of reach), with the reason.
 */
export function pollFailure(e: unknown, failures: number): { lost: string } | { delay: number } {
  if (e instanceof ApiRequestError && e.status === 404)
    return { lost: "The server no longer knows this import (it was restarted). Look for the presentation in the library, or import the file again." };
  if (failures >= MAX_POLL_FAILURES) return { lost: "Lost contact with the server during the import. Look for the presentation in the library, or import the file again." };
  return { delay: Math.min(400 * 2 ** Math.max(failures - 1, 0), 5000) };
}

/** The job is over (done or failed). */
export const finished = (j: PptxImportJob | null) => !!j && (j.state === "done" || j.state === "failed");

const STATE_TEXT: Record<PptxImportState, string> = {
  queued: "Waiting for another import to finish…",
  reading: "Reading the slides",
  pictures: "Drawing slide pictures",
  media: "Taking pictures into the Codex",
  saving: "Saving the presentation…",
  done: "Imported",
  failed: "The import failed",
};

/** "Reading the slides (3 of 12)". */
export function progressText(j: PptxImportJob): string {
  const base = STATE_TEXT[j.state] ?? j.state;
  if ((j.state === "reading" || j.state === "media") && j.total > 0) return `${base} (${Math.min(j.done + 1, j.total)} of ${j.total})`;
  if (j.state === "pictures" && j.total > 0) return `${base} for ${j.total} slide${j.total === 1 ? "" : "s"}…`;
  return base;
}

/** 0..1 for a progress bar (the stages weighted: slides, pictures, media, saving). */
export function progressFraction(j: PptxImportJob): number {
  const part = j.total > 0 ? Math.min(j.done / j.total, 1) : 0;
  switch (j.state) {
    case "queued":
      return 0;
    case "reading":
      return 0.05 + 0.35 * part;
    case "pictures":
      return 0.4 + 0.2 * part;
    case "media":
      return 0.6 + 0.3 * part;
    case "saving":
      return 0.95;
    default:
      return 1;
  }
}

/** "12 slides: 10 fully editable, 2 with parts drawn from a picture of the slide." */
export function summary(r: PptxImportReport): string {
  const n = r.slides.length;
  const parts = [`${n} slide${n === 1 ? "" : "s"}`];
  const bits: string[] = [];
  bits.push(`${r.editableSlides} fully editable`);
  if (r.picturedSlides) bits.push(`${r.picturedSlides} with parts drawn from a picture of the slide`);
  const approx = r.slides.filter((s) => (s.approximated?.length ?? 0) > 0).length;
  if (approx) bits.push(`${approx} with parts approximated or left out`);
  return `${parts[0]}: ${bits.join(", ")}.`;
}

/** What a slide's row says ("Editable", "Chart pictured", "Table left out" …). */
export function slideStatus(s: PptxSlideReport): { tone: "ok" | "picture" | "warn"; text: string } {
  const pictured = s.pictured ?? [];
  const approx = s.approximated ?? [];
  if (approx.length) {
    const left = approx.filter((a) => (a.detail ?? "").includes("left out")).length;
    return { tone: "warn", text: left === approx.length ? `${kinds(approx)} left out` : `${kinds(approx)} approximated` };
  }
  if (pictured.length) return { tone: "picture", text: `${kinds(pictured)} as picture` };
  return { tone: "ok", text: "Editable" };
}

/** "Chart", "Chart, table", "3 objects". */
function kinds(list: { kind: string }[]): string {
  const names = [...new Set(list.map((x) => kindName(x.kind)))];
  const s = names.length > 2 ? `${list.length} objects` : names.join(", ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** A report kind for people. */
export function kindName(kind: string): string {
  switch (kind) {
    case "smartArt":
      return "SmartArt";
    case "background":
      return "background";
    default:
      return kind.replace(/^an? /, "");
  }
}

/** Every note of a slide, one line each ("Chart “Sales”: drawn from the slide's picture"). */
export function slideNotes(s: PptxSlideReport): string[] {
  const line = (n: { kind: string; name?: string; detail?: string }) => {
    const what = kindName(n.kind);
    const head = (what.charAt(0).toUpperCase() + what.slice(1)) + (n.name ? ` “${n.name}”` : "");
    return n.detail ? `${head}: ${n.detail}` : head;
  };
  return [...(s.pictured ?? []), ...(s.approximated ?? []), ...(s.notes ?? [])].map(line);
}
