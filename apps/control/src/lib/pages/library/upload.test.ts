import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import type { CreateUpload, UploadFinished, UploadInfo } from "$protocol";
import { ApiRequestError } from "../../server/rest";
import { Sha256, hashBlob } from "./sha256";
import { UploadQueue, contentRange, uploadDoneText, mediaFiles, nextChunk, overallProgress, retryDelay, uploadErrorKind, type UploadTransport, type UploadView } from "./upload";

const hex = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");

describe("Sha256", () => {
  it("matches known digests", () => {
    expect(new Sha256().hex()).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(new Sha256().update(new TextEncoder().encode("abc")).hex()).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  });
  it("gives the same digest however the bytes are fed", () => {
    for (const n of [55, 56, 63, 64, 65, 119, 120, 1000, 4097]) {
      const data = new Uint8Array(n).map((_, i) => (i * 31 + 7) & 255);
      const h = new Sha256();
      for (let i = 0; i < n; i += 13) h.update(data.subarray(i, i + 13));
      expect(h.hex()).toBe(hex(data));
    }
  });
  it("hashes a blob in slices", async () => {
    const data = new Uint8Array(10_000).map((_, i) => i & 255);
    const seen: number[] = [];
    expect(await hashBlob(new Blob([data]), (d) => seen.push(d), 3000)).toBe(hex(data));
    expect(seen).toEqual([3000, 6000, 9000, 10_000]);
  });
});

describe("upload helpers", () => {
  it("chunks and ranges", () => {
    expect(nextChunk(0, 10, 4)).toEqual({ start: 0, end: 4 });
    expect(nextChunk(8, 10, 4)).toEqual({ start: 8, end: 10 });
    expect(nextChunk(10, 10, 4)).toBeNull();
    expect(contentRange(0, 4, 10)).toBe("bytes 0-3/10");
    expect(retryDelay(0)).toBe(1000);
    expect(retryDelay(10)).toBe(15_000);
  });
  it("sorts errors", () => {
    expect(uploadErrorKind(new ApiRequestError("network", "x", 0))).toBe("offline");
    expect(uploadErrorKind(new ApiRequestError("conflict", "x", 409))).toBe("conflict");
    expect(uploadErrorKind(new ApiRequestError("notFound", "x", 404))).toBe("gone");
    expect(uploadErrorKind(new ApiRequestError("rateLimited", "x", 429))).toBe("busy");
    expect(uploadErrorKind(new ApiRequestError("badRequest", "x", 400))).toBe("fatal");
  });
  it("keeps media files with their folder paths", () => {
    const f = (name: string, rel?: string) => Object.assign(new File(["x"], name), rel ? { webkitRelativePath: rel } : {});
    const out = mediaFiles([f("a.JPG", "Pics/a.JPG"), f(".DS_Store"), f("notes.txt"), f("b.mp4")]);
    expect(out.map((x) => [x.file.name, x.relativePath])).toEqual([
      ["a.JPG", "Pics/a.JPG"],
      ["b.mp4", undefined],
    ]);
  });
  it("adds up progress", () => {
    const v = (size: number, sent: number, phase: UploadView["phase"]) => ({ key: 0, name: "", size, sent, hashed: 0, phase, message: null, replaces: null, result: null });
    expect(overallProgress([v(100, 50, "sending"), v(100, 0, "done"), v(100, 0, "cancelled")])).toBe(0.75);
  });
});

/** A fake server: keeps the bytes, can drop the link, and can lose an upload. */
function fakeServer() {
  const uploads = new Map<string, { req: CreateUpload; bytes: Uint8Array; offset: number }>();
  const s = {
    down: false,
    /** Chunks to break off half-way (the server keeps what came). */
    breakNext: 0,
    created: 0,
    puts: [] as string[],
    finished: [] as { id: string; sha256: string }[],
    /** The assets made so far, by collection and SHA-256 (a re-upload answers `existing`). */
    assets: new Map<string, string>(),
    cancelled: [] as string[],
    transport: null as unknown as UploadTransport,
  };
  const net = () => {
    if (s.down) throw new ApiRequestError("network", "Could not reach the server", 0);
  };
  const info = (id: string): UploadInfo => {
    const u = uploads.get(id);
    if (!u) throw new ApiRequestError("notFound", "no such upload", 404);
    return { id, collection: u.req.collection, name: u.req.name, size: u.req.size, offset: u.offset, createdAt: "", updatedAt: "", expiresAt: "" };
  };
  s.transport = {
    async create(req) {
      net();
      const id = `u${++s.created}`;
      uploads.set(id, { req, bytes: new Uint8Array(req.size), offset: 0 });
      return info(id);
    },
    async status(id) {
      net();
      return info(id);
    },
    async put(id, chunk, start, size) {
      net();
      const u = uploads.get(id);
      if (!u) throw new ApiRequestError("notFound", "no such upload", 404);
      s.puts.push(contentRange(start, start + chunk.size, size));
      if (start !== u.offset) throw new ApiRequestError("conflict", "wrong offset", 409);
      let data = new Uint8Array(await chunk.arrayBuffer());
      if (s.breakNext > 0) {
        s.breakNext--;
        data = data.subarray(0, Math.floor(data.length / 2));
        u.bytes.set(data, start);
        u.offset += data.length;
        throw new ApiRequestError("network", "Could not reach the server", 0);
      }
      u.bytes.set(data, start);
      u.offset += data.length;
    },
    async finish(id, sha256) {
      net();
      const u = uploads.get(id)!;
      if (u.offset < u.req.size) throw new ApiRequestError("conflict", "bytes missing", 409);
      if (hex(u.bytes) !== sha256) throw new ApiRequestError("badRequest", "sha256 mismatch", 400, "sha256");
      s.finished.push({ id, sha256 });
      const key = `${u.req.collection}:${sha256}`;
      const had = s.assets.get(key);
      if (!had) s.assets.set(key, `asset-${id}`);
      const asset = { schemaVersion: 2, id: had ?? `asset-${id}`, kind: "image", name: u.req.name };
      return { collection: u.req.collection, asset, revision: "r1", deduplicated: !!had, ...(had ? { existing: true } : {}) } as UploadFinished;
    },
    async cancel(id) {
      s.cancelled.push(id);
      uploads.delete(id);
    },
  };
  return { s, uploads };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
async function until(fn: () => boolean) {
  for (let i = 0; i < 500 && !fn(); i++) await flush();
  expect(fn()).toBe(true);
}

describe("UploadQueue", () => {
  const data = new Uint8Array(10_000).map((_, i) => (i * 7) & 255);
  const hasher = (b: Blob, p: (n: number) => void) => hashBlob(b, p, 4096);

  it("sends a file in chunks and finishes with its SHA-256", async () => {
    const { s } = fakeServer();
    const done: UploadView[] = [];
    const q = new UploadQueue({ transport: () => s.transport, hasher, chunkBytes: 4000, onDone: (v) => done.push(v) });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1", bin: "b1", relativePath: "Pics/Dawn.jpg" }]);
    await until(() => last[0]?.phase === "done");
    expect(s.puts).toEqual(["bytes 0-3999/10000", "bytes 4000-7999/10000", "bytes 8000-9999/10000"]);
    expect(s.finished).toEqual([{ id: "u1", sha256: hex(data) }]);
    expect(done).toHaveLength(1);
    expect(last[0]!.sent).toBe(10_000);
  });

  it("says when the collection had the file already (the same SHA-256)", async () => {
    const { s } = fakeServer();
    const q = new UploadQueue({ transport: () => s.transport, hasher, chunkBytes: 4000 });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1" }]);
    await until(() => last[0]?.phase === "done");
    q.add([{ file: new Blob([data]), name: "Dawn copy.jpg", collection: "c1" }]);
    await until(() => last[1]?.phase === "done");
    expect(last[1]!.result?.existing).toBe(true);
    expect(last[1]!.result?.asset.id).toBe(last[0]!.result?.asset.id);
    expect(uploadDoneText(last[0]!.result)).toBe("Done");
    expect(uploadDoneText(last[1]!.result)).toBe("Already in this collection");
  });

  it("words a finished upload: relinked, existing, deduplicated, done", () => {
    const r = (extra: Partial<UploadFinished>) => ({ collection: "c1", asset: { schemaVersion: 2, id: "a", name: "x.jpg" }, revision: "r1", deduplicated: false, ...extra }) as UploadFinished;
    expect(uploadDoneText(r({ relinked: true }))).toBe("Linked again");
    expect(uploadDoneText(r({ existing: true, deduplicated: true }))).toBe("Already in this collection");
    expect(uploadDoneText(r({ deduplicated: true }))).toBe("Already in the library");
    expect(uploadDoneText(r({}))).toBe("Done");
    expect(uploadDoneText(null)).toBe("Done");
  });

  it("waits while offline and resumes from the server's offset", async () => {
    const { s } = fakeServer();
    const q = new UploadQueue({ transport: () => s.transport, hasher, chunkBytes: 4000, sleep: () => new Promise(() => {}) });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    s.breakNext = 1;
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1" }]);
    await until(() => last[0]?.phase === "waiting");
    expect(last[0]!.message).toBe("Waiting for the connection");
    s.down = true;
    q.resumeAll();
    await until(() => last[0]?.phase === "waiting");
    s.down = false;
    q.resumeAll();
    await until(() => last[0]?.phase === "done");
    // The broken chunk kept 2000 bytes: the next one starts there.
    expect(s.puts).toContain("bytes 2000-5999/10000");
    expect(s.finished[0]!.sha256).toBe(hex(data));
  });

  it("starts again when the server dropped the upload", async () => {
    const { s, uploads } = fakeServer();
    let firstPut = true;
    const t: UploadTransport = {
      ...s.transport,
      put: async (id, chunk, start, size, signal) => {
        if (firstPut) {
          firstPut = false;
          uploads.delete(id);
        }
        return s.transport.put(id, chunk, start, size, signal);
      },
    };
    const q = new UploadQueue({ transport: () => t, hasher, chunkBytes: 4000 });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1" }]);
    await until(() => last[0]?.phase === "done");
    expect(s.created).toBe(2);
  });

  it("cancels and tells the server", async () => {
    const { s } = fakeServer();
    const q = new UploadQueue({ transport: () => s.transport, hasher, chunkBytes: 4000, sleep: () => new Promise(() => {}) });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    s.breakNext = 1;
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1", replaces: "a9" }]);
    await until(() => last[0]?.phase === "waiting");
    await q.cancel(last[0]!.key);
    expect(last[0]!.phase).toBe("cancelled");
    expect(s.cancelled).toEqual(["u1"]);
    expect(last[0]!.replaces).toBe("a9");
    q.clearFinished();
    expect(last).toEqual([]);
  });

  it("fails with plain words on a refused file, and can retry", async () => {
    const { s } = fakeServer();
    const t: UploadTransport = { ...s.transport, create: async () => Promise.reject(new ApiRequestError("insufficientStorage", "full", 507)) };
    let useBad = true;
    const q = new UploadQueue({ transport: () => ({ ...s.transport, create: (r) => (useBad ? t.create(r) : s.transport.create(r)) }), hasher, chunkBytes: 4000 });
    let last: UploadView[] = [];
    q.subscribe((l) => (last = l));
    q.add([{ file: new Blob([data]), name: "Dawn.jpg", collection: "c1" }]);
    await until(() => last[0]?.phase === "failed");
    expect(last[0]!.message).toBe("Not enough free space on the Codex box");
    useBad = false;
    q.retry(last[0]!.key);
    await until(() => last[0]?.phase === "done");
  });
});
