// Resumable media uploads to the Codex (crates/protocol/README.md, Codex ▸ Media: `POST
// /codex/uploads`, chunks with `Content-Range`, `finish` with the SHA-256). Plain TypeScript over
// a transport and a hasher (so the logic is unit tested without a server or a worker):
//
// - every file is sent in chunks from the server's `offset`; a chunk refused with `409` (it
//   didn't start where the server is) continues from the offset the server reports;
// - while the server can't be reached the upload waits and tries again (backing off, or at once
//   on `resumeAll`, e.g. after a reconnect), asking the server how far it got;
// - an upload the server dropped (24 hours without a chunk) starts again;
// - the SHA-256 is computed while the bytes go out and sent with `finish`;
// - cancel stops the upload and tells the server to drop it.

import type { CreateUpload, UploadFinished, UploadInfo } from "$protocol";
import { ApiRequestError } from "../../server/rest";

/** What an upload needs from the server (the Codex box's REST connection). */
export interface UploadTransport {
  create(req: CreateUpload): Promise<UploadInfo>;
  status(id: string): Promise<UploadInfo>;
  /** Sends `chunk` as the bytes from `start` on (`Content-Range: bytes start-end/size`). */
  put(id: string, chunk: Blob, start: number, size: number, signal: AbortSignal): Promise<void>;
  finish(id: string, sha256: string): Promise<UploadFinished>;
  cancel(id: string): Promise<void>;
}

/** Computes a file's SHA-256 (a Web Worker in the app), reporting the bytes done. */
export type Hasher = (file: Blob, onProgress: (done: number) => void) => Promise<string>;

export type UploadPhase = "queued" | "sending" | "waiting" | "finishing" | "done" | "failed" | "cancelled";

/** One file to upload. */
export interface UploadRequest {
  file: Blob;
  name: string;
  collection: string;
  bin?: string;
  relativePath?: string;
  /** Relink this media asset to the uploaded file instead of making a new one. */
  replaces?: string;
}

/** What the page shows of an upload. */
export interface UploadView {
  key: number;
  name: string;
  size: number;
  sent: number;
  hashed: number;
  phase: UploadPhase;
  /** Why it failed, or why it waits. */
  message: string | null;
  replaces: string | null;
  result: UploadFinished | null;
}

/**
 * What a finished upload says: relinked, the collection's own asset for the same file
 * (`existing`: no second tile appears), bytes the media library had already, or done.
 */
export function uploadDoneText(r: UploadFinished | null): string {
  if (r?.relinked) return "Linked again";
  if (r?.existing) return "Already in this collection";
  if (r?.deduplicated) return "Already in the library";
  return "Done";
}

/** `bytes 0-1023/4096` for the bytes [start, end). */
export function contentRange(start: number, end: number, size: number): string {
  return `bytes ${start}-${end - 1}/${size}`;
}

/** The next chunk from `offset`: [start, end), at most `chunk` bytes, or null when all are sent. */
export function nextChunk(offset: number, size: number, chunk: number): { start: number; end: number } | null {
  if (offset >= size) return null;
  return { start: offset, end: Math.min(size, offset + chunk) };
}

/** How long to wait before the n-th retry (1 s, 2 s, 4 s … at most 15 s). */
export function retryDelay(attempt: number): number {
  return Math.min(15_000, 1000 * 2 ** Math.max(0, attempt));
}

/** What a failed request means for an upload. */
export type UploadErrorKind = "offline" | "conflict" | "gone" | "busy" | "fatal";

export function uploadErrorKind(e: unknown): UploadErrorKind {
  if (!(e instanceof ApiRequestError)) return "fatal";
  if (e.code === "network" || e.code === "timeout" || e.status === 503 || e.status === 502 || e.status === 504) return "offline";
  if (e.status === 409) return "conflict";
  if (e.status === 404) return "gone";
  if (e.status === 429) return "busy";
  return "fatal";
}

/** The plain-words reason an upload stopped. */
export function uploadFailureText(e: unknown): string {
  if (e instanceof ApiRequestError) {
    if (e.code === "insufficientStorage" || e.status === 507) return "Not enough free space on the Codex box";
    if (e.field === "sha256") return "The file changed while it was sent. Send it again";
    if (e.field === "name") return "This kind of file can't go into the media library";
    if (e.field === "size") return "The file is larger than the Codex box takes";
    if (e.field === "bin") return "That bin can't take uploads";
    return e.message;
  }
  return e instanceof Error ? e.message : String(e);
}

class Upload {
  view: UploadView;
  id: string | null = null;
  private ctrl: AbortController | null = null;
  private hash: Promise<string> | null = null;
  private wake: (() => void) | null = null;

  constructor(
    key: number,
    readonly req: UploadRequest,
    private readonly t: UploadTransport,
    private readonly hasher: Hasher,
    private readonly chunk: number,
    private readonly changed: () => void,
    private readonly sleep: (ms: number) => Promise<void>,
  ) {
    this.view = { key, name: req.name, size: req.file.size, sent: 0, hashed: 0, phase: "queued", message: null, replaces: req.replaces ?? null, result: null };
  }

  private set(patch: Partial<UploadView>) {
    this.view = { ...this.view, ...patch };
    this.changed();
  }

  private get stopped() {
    return this.view.phase === "cancelled";
  }

  /** Wakes a waiting upload (the connection is back). */
  resume() {
    this.wake?.();
  }

  async cancel() {
    if (this.view.phase === "done" || this.view.phase === "cancelled") return;
    this.set({ phase: "cancelled", message: null });
    this.ctrl?.abort();
    this.wake?.();
    if (this.id) await this.t.cancel(this.id).catch(() => {});
  }

  /** Waits after a failed request; returns false when cancelled meanwhile. */
  private async pause(attempt: number, message: string): Promise<boolean> {
    this.set({ phase: "waiting", message });
    await Promise.race([this.sleep(retryDelay(attempt)), new Promise<void>((r) => (this.wake = r))]);
    this.wake = null;
    if (this.stopped) return false;
    this.set({ phase: "sending", message: null });
    return true;
  }

  async run(): Promise<void> {
    const { req } = this;
    const size = req.file.size;
    this.hash ??= this.hasher(req.file, (done) => this.set({ hashed: done }));
    this.hash.catch(() => {});
    this.set({ phase: "sending" });
    let attempt = 0;
    let recreated = false;
    let offset = 0;
    for (;;) {
      if (this.stopped) return;
      try {
        if (!this.id) {
          const info = await this.t.create({
            collection: req.collection,
            name: req.name,
            size,
            ...(req.bin && !req.replaces ? { bin: req.bin } : {}),
            ...(req.relativePath ? { relativePath: req.relativePath } : {}),
            ...(req.replaces ? { replaces: req.replaces } : {}),
          });
          this.id = info.id;
          offset = info.offset;
          this.set({ sent: offset });
        }
        const next = nextChunk(offset, size, this.chunk);
        if (next) {
          this.ctrl = new AbortController();
          await this.t.put(this.id, req.file.slice(next.start, next.end), next.start, size, this.ctrl.signal);
          offset = next.end;
          attempt = 0;
          this.set({ sent: offset });
          continue;
        }
        this.set({ phase: "finishing" });
        const sha256 = await this.hash;
        if (this.stopped) return;
        const result = await this.t.finish(this.id, sha256);
        this.set({ phase: "done", result, sent: size, hashed: size, message: null });
        return;
      } catch (e) {
        if (this.stopped) return;
        const kind = uploadErrorKind(e);
        if (kind === "offline" || kind === "busy") {
          if (!(await this.pause(attempt++, kind === "busy" ? "Waiting for the Codex box" : "Waiting for the connection"))) return;
          // Ask how far the server got before sending more.
          if (this.id) {
            try {
              offset = (await this.t.status(this.id)).offset;
              this.set({ sent: offset });
            } catch (err) {
              if (uploadErrorKind(err) === "gone") this.id = null;
            }
          }
          continue;
        }
        if (kind === "conflict" && this.id) {
          try {
            offset = (await this.t.status(this.id)).offset;
            this.set({ sent: offset, phase: "sending" });
            if (++attempt < 8) continue;
          } catch {
            /* fall through: failed */
          }
        }
        if (kind === "gone" && this.id && !recreated) {
          // Dropped by the server (a day without a chunk): start again once.
          recreated = true;
          this.id = null;
          offset = 0;
          this.set({ sent: 0 });
          continue;
        }
        this.set({ phase: "failed", message: uploadFailureText(e) });
        return;
      }
    }
  }
}

/** The uploads of one page: run a few at a time, in the order they were added. */
export class UploadQueue {
  private uploads: Upload[] = [];
  private running = 0;
  private seq = 0;
  private listeners = new Set<(list: UploadView[]) => void>();

  constructor(
    private readonly opts: {
      transport: () => UploadTransport;
      hasher: Hasher;
      chunkBytes?: number;
      parallel?: number;
      sleep?: (ms: number) => Promise<void>;
      /** One upload finished (the page reloads its media). */
      onDone?: (v: UploadView) => void;
    },
  ) {}

  subscribe(fn: (list: UploadView[]) => void): () => void {
    this.listeners.add(fn);
    fn(this.list());
    return () => this.listeners.delete(fn);
  }

  list(): UploadView[] {
    return this.uploads.map((u) => u.view);
  }

  private emit() {
    const l = this.list();
    for (const fn of this.listeners) fn(l);
  }

  add(reqs: UploadRequest[]): number[] {
    const keys: number[] = [];
    for (const r of reqs) {
      const key = ++this.seq;
      const sleep = this.opts.sleep ?? ((ms: number) => new Promise<void>((res) => setTimeout(res, ms)));
      this.uploads.push(new Upload(key, r, this.opts.transport(), this.opts.hasher, this.opts.chunkBytes ?? 8 * 1024 * 1024, () => this.emit(), sleep));
      keys.push(key);
    }
    this.emit();
    this.pump();
    return keys;
  }

  private pump() {
    const max = this.opts.parallel ?? 2;
    while (this.running < max) {
      const next = this.uploads.find((u) => u.view.phase === "queued");
      if (!next) return;
      this.running++;
      void next.run().finally(() => {
        this.running--;
        if (next.view.phase === "done") this.opts.onDone?.(next.view);
        this.pump();
      });
    }
  }

  cancel(key: number) {
    return this.uploads.find((u) => u.view.key === key)?.cancel();
  }

  /** Sends a failed upload again from where the server is. */
  retry(key: number) {
    const u = this.uploads.find((x) => x.view.key === key);
    if (!u || u.view.phase !== "failed") return;
    u.view = { ...u.view, phase: "queued", message: null };
    this.emit();
    this.pump();
  }

  /** The connection is back: waiting uploads try at once. */
  resumeAll() {
    for (const u of this.uploads) if (u.view.phase === "waiting") u.resume();
  }

  /** Takes finished, failed and cancelled uploads off the list. */
  clearFinished() {
    this.uploads = this.uploads.filter((u) => u.view.phase !== "done" && u.view.phase !== "cancelled" && u.view.phase !== "failed");
    this.emit();
  }

  get active(): boolean {
    return this.uploads.some((u) => u.view.phase !== "done" && u.view.phase !== "cancelled" && u.view.phase !== "failed");
  }
}

/** Total progress over the uploads still going (0–1), for the tray's header. */
export function overallProgress(list: readonly UploadView[]): number {
  const going = list.filter((u) => u.phase !== "cancelled" && u.phase !== "failed");
  const size = going.reduce((n, u) => n + u.size, 0);
  return size ? going.reduce((n, u) => n + (u.phase === "done" ? u.size : u.sent), 0) / size : 1;
}

/** Files of a folder pick or a drop, with their relative paths, media files only. */
export function mediaFiles(files: Iterable<File>): { file: File; relativePath?: string }[] {
  const out: { file: File; relativePath?: string }[] = [];
  for (const f of files) {
    if (f.name.startsWith(".")) continue;
    if (!/\.(jpe?g|png|gif|webp|bmp|tiff?|heic|avif|mp4|m4v|mov|mkv|webm|avi|wmv|mpe?g|mxf|ts|mp3|wav|aac|m4a|flac|ogg|oga|opus|aiff?)$/i.test(f.name)) continue;
    const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath;
    out.push(rel ? { file: f, relativePath: rel } : { file: f });
  }
  return out;
}
