// Typed REST clients for the Codex (crates/protocol/README.md, "Codex"): the box's Codex status,
// collections and the assignment, the item trash, a collection's media, search, blobs and
// storage. Plain TypeScript over a RestClient (the main connection's, or the side connection to
// the Codex box, see codexBox.ts), unit tested. Errors stay `ApiRequestError`s; `codexFailure`
// sorts out the ones the Library answers with a prompt (409 liveShow, 409 codexElsewhere, 403).

import type {
  AssetLocation,
  CodexBox,
  CodexStatus,
  CodexStorage,
  CollectionMedia,
  CollectionSummary,
  CreateCollection,
  CreateUpload,
  DeletedCollection,
  DocumentKind,
  DocumentUse,
  DormantUpload,
  LibraryItems,
  MediaAsset,
  MediaMatchReport,
  MediaMatchRequest,
  RelinkReport,
  RelinkSearch,
  RelinkTarget,
  RelinkedAsset,
  SearchResults,
  SetAssignment,
  StartDormantUpload,
  TrashItem,
  TrashPurged,
  TrashRestored,
  UpdateCollection,
  UploadFinished,
  UploadInfo,
} from "$protocol";
import { ApiRequestError, type RestClient } from "./rest";

/** `GET /codex/collections/{id}/media?status=` filters. */
export type MediaStatusFilter = "online" | "missing" | "unsupported" | "all";

/** Search scope: this box's active collection (default), one collection, or every one. */
export interface CodexSearchOptions {
  collection?: string | "all";
  kinds?: DocumentKind[];
  limit?: number;
}

const enc = encodeURIComponent;

export class CodexClient {
  constructor(readonly rest: RestClient) {}

  /** This box's view of the site's Codex: role, active collection, counts, sync. */
  status = () => this.rest.get<CodexStatus>("/codex");
  /** The Codex's collections, by sort key then name. */
  collections = () => this.rest.get<CollectionSummary[]>("/codex/collections");
  collection = (id: string) => this.rest.get<CollectionSummary>(`/codex/collections/${enc(id)}`);
  /** Deleted collections, newest first. */
  collectionTrash = () => this.rest.get<DeletedCollection[]>("/codex/collection-trash");

  // Collections (admins; on a show box: `409 codexElsewhere`).
  createCollection = (body: CreateCollection) => this.rest.request<CollectionSummary>("POST", "/codex/collections", body);
  updateCollection = (id: string, body: UpdateCollection, revision: string) =>
    this.rest.request<CollectionSummary>("PUT", `/codex/collections/${enc(id)}`, body, undefined, { ifMatch: revision });
  /** Into the collection trash (not the one this box shows: `409`). */
  deleteCollection = (id: string, revision: string) => this.rest.request<void>("DELETE", `/codex/collections/${enc(id)}`, undefined, undefined, { ifMatch: revision });
  restoreCollection = (id: string) => this.rest.request<CollectionSummary>("POST", `/codex/collection-trash/${enc(id)}/restore`);
  /** Deletes a collection in the collection trash for good. */
  purgeCollection = (id: string, revision?: string) =>
    this.rest.request<void>("DELETE", `/codex/collection-trash/${enc(id)}`, undefined, undefined, { ifMatch: revision ?? null });

  // Uploads (resumable; the chunks themselves go through `putUploadChunk`).
  createUpload = (body: CreateUpload) => this.rest.request<UploadInfo>("POST", "/codex/uploads", body);
  uploadInfo = (id: string) => this.rest.get<UploadInfo>(`/codex/uploads/${enc(id)}`);
  finishUpload = (id: string, sha256: string) => this.rest.request<UploadFinished>("POST", `/codex/uploads/${enc(id)}/finish`, { sha256 });
  cancelUpload = (id: string) => this.rest.request<void>("DELETE", `/codex/uploads/${enc(id)}`);

  // Missing media.
  relink = (collection: string, asset: string, target: RelinkTarget) =>
    this.rest.request<RelinkedAsset>("POST", `/codex/collections/${enc(collection)}/media/${enc(asset)}/relink`, target);
  /** "Find missing files": the server looks for each missing asset's file. */
  relinkSearch = (collection: string, body: RelinkSearch = {}) => this.rest.request<RelinkReport>("POST", `/codex/collections/${enc(collection)}/media/search`, body);
  /** Matches files of a folder on this computer to missing assets (nothing changes). */
  matchFolder = (collection: string, body: MediaMatchRequest) => this.rest.request<MediaMatchReport>("POST", `/codex/collections/${enc(collection)}/media/match`, body);

  /**
   * Shows another collection on this box (`PUT /codex/assignment`, admins). Without `confirm`
   * the server refuses with `409 liveShow` while something is live.
   */
  assign = (collection: string, confirm = false) => {
    const body: SetAssignment = confirm ? { collection, confirm: true } : { collection };
    return this.rest.request<CodexStatus>("PUT", "/codex/assignment", body);
  };

  // A show box's own, dormant collection (G15): the offer and progress, upload it to the
  // site's Codex (admins), cancel.
  dormantUpload = () => this.rest.get<DormantUpload>("/codex/dormant/upload");
  startDormantUpload = (body: StartDormantUpload = {}) => this.rest.request<DormantUpload>("POST", "/codex/dormant/upload", body);
  cancelDormantUpload = () => this.rest.request<void>("DELETE", "/codex/dormant/upload");

  /** The item trash (deleted documents, 30 days), newest first. */
  trash = () => this.rest.get<TrashItem[]>("/codex/trash");
  restoreTrash = (id: string) => this.rest.request<TrashRestored>("POST", `/codex/trash/${enc(id)}/restore`);
  /** Empties the item trash for good (admins; on a show box: `409 codexElsewhere`). */
  emptyTrash = () => this.rest.request<TrashPurged>("DELETE", "/codex/trash");

  /** A collection's media assets with their availability on this box. */
  media = (collection: string, status: MediaStatusFilter[] = []) =>
    this.rest.get<CollectionMedia[]>(`/codex/collections/${enc(collection)}/media`, status.length ? { status: status.join(",") } : undefined);

  /** Codex search, best first (see `RestClient.codexSearch`, which this mirrors). */
  search = (q: string, opts: CodexSearchOptions = {}) => {
    const params: Record<string, string | number> = { q };
    if (opts.collection) params.collection = opts.collection;
    if (opts.kinds?.length) params.kinds = opts.kinds.join(",");
    if (opts.limit) params.limit = opts.limit;
    return this.rest.get<SearchResults>("/codex/search", params);
  };

  /** The media library's items by path (for a relink to any of them; on a show box: `409 codexElsewhere`). */
  libraryItems = (opts: { q?: string; kind?: "video" | "picture" | "sound"; limit?: number } = {}) => {
    const params: Record<string, string | number> = {};
    if (opts.q) params.q = opts.q;
    if (opts.kind) params.kind = opts.kind;
    if (opts.limit) params.limit = opts.limit;
    return this.rest.get<LibraryItems>("/codex/library", Object.keys(params).length ? params : undefined);
  };

  /** The documents that use a media asset or a prop ("Used by"). */
  usedBy = (what: "media" | "prop", id: string) => this.rest.get<DocumentUse[]>(`/${what === "media" ? "media" : "props"}/${enc(id)}/used-by`);

  /** Where the Codex's disk space goes. */
  storage = () => this.rest.get<CodexStorage>("/codex/storage");

  /** A media file by SHA-256 (for `<img>` / `<video>`: the token rides in the query). */
  blobUrl(sha256: string, ext: string): string {
    return this.rest.url(`/codex/blobs/${sha256}.${ext}`, {}, true);
  }

  /** The 320 px JPEG of a media file by SHA-256. */
  blobThumbnailUrl(sha256: string, ext: string): string {
    return this.rest.url(`/codex/blobs/${sha256}.${ext}/thumbnail`, {}, true);
  }
}

/**
 * Sends one chunk of an upload (`PUT /codex/uploads/{id}` with `Content-Range`): the bytes as
 * they are, which `RestClient.request` (JSON bodies) doesn't do. Errors as `ApiRequestError`s.
 */
export async function putUploadChunk(
  rest: RestClient,
  auth: { token: () => string | null; clientId?: string | null },
  id: string,
  chunk: Blob,
  start: number,
  size: number,
  signal?: AbortSignal,
): Promise<void> {
  const headers: Record<string, string> = {
    Accept: "application/json",
    "Content-Type": "application/octet-stream",
    "Content-Range": `bytes ${start}-${start + chunk.size - 1}/${size}`,
  };
  const t = auth.token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (auth.clientId) headers["X-Client-Id"] = auth.clientId;
  let res: Response;
  try {
    res = await fetch(rest.url(`/codex/uploads/${enc(id)}`), { method: "PUT", headers, body: chunk, signal });
  } catch {
    throw new ApiRequestError("network", "Could not reach the server", 0);
  }
  if (res.ok) return;
  let err: { code?: string; message?: string; field?: string } | undefined;
  try {
    err = ((await res.json()) as { error?: typeof err })?.error;
  } catch {
    /* not JSON */
  }
  throw new ApiRequestError((err?.code ?? "internal") as ApiRequestError["code"], err?.message ?? `HTTP ${res.status}`, res.status, err?.field ?? null);
}

/** The SHA-256 and extension of an asset's file, when its location names them. */
export function assetBlob(asset: Pick<MediaAsset, "name" | "location">): { sha256: string; ext: string } | null {
  const loc: AssetLocation | undefined = asset.location;
  if (!loc || (loc.type !== "library" && loc.type !== "managed")) return null;
  const fromName = /\.([A-Za-z0-9]{1,8})$/.exec(asset.name ?? "")?.[1];
  const ext = (loc.type === "managed" ? loc.extension : undefined) ?? fromName ?? "bin";
  return { sha256: loc.sha256, ext: ext.toLowerCase() };
}

/** What a failed Codex request means for the Library page. */
export type CodexFailure =
  /** Something is live on this box: ask, then send again with `confirm: true`. */
  | { kind: "liveShow"; message: string; live: string[] | null }
  /** A show box: this is managed on the Codex box (`box`, null while the box doesn't know it). */
  | { kind: "elsewhere"; box: CodexBox | null; message: string }
  /** An admin action at this access level: unlock or sign in first. */
  | { kind: "forbidden"; message: string }
  /** The site's Codex can't be reached (a show box), or the media library is closed. */
  | { kind: "unavailable"; box: CodexBox | null; message: string }
  /** Gone, or refused because of the state it's in (e.g. its shelf is gone too). */
  | { kind: "notFound"; message: string }
  | { kind: "conflict"; message: string }
  /** The request never got an answer. */
  | { kind: "offline"; message: string }
  | { kind: "other"; message: string };

export function codexFailure(e: unknown): CodexFailure {
  if (!(e instanceof ApiRequestError)) return { kind: "other", message: e instanceof Error ? e.message : String(e) };
  const message = e.message;
  if (e.code === "network" || e.code === "timeout") return { kind: "offline", message };
  if (e.code === "liveShow") return { kind: "liveShow", message, live: e.live };
  if (e.code === "codexElsewhere") return { kind: "elsewhere", box: e.codex, message };
  if (e.status === 403 || e.code === "forbidden") return { kind: "forbidden", message };
  if (e.status === 503 || e.code === "unavailable") return { kind: "unavailable", box: e.codex, message };
  if (e.status === 404) return { kind: "notFound", message };
  if (e.status === 409) return { kind: "conflict", message };
  return { kind: "other", message };
}

/** The plain-words message for a refused admin action (Oriel has no unlock prompt yet). */
export function forbiddenText(action: string): string {
  return `${action} is for the site's admins. Unlock the site with its PIN or sign in as an admin on the server, then try again.`;
}
