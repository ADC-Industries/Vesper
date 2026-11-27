// Typed REST client for the Codex's protection (crates/protocol/README.md, "Settings backups and
// restore" and "Export and import"): settings backup sets, snapshots, restore previews and
// restores (queued ones for show boxes), Oriel's own preferences, the manual export download,
// nightly exports and their stored passphrase, and importing an export. On a show box these
// answer `409 codexElsewhere`: run them through the side connection (codexBox.ts). Plain
// TypeScript over a RestClient; errors stay `ApiRequestError`s.

import type {
  BackupSetSummary,
  CodexExportStatus,
  CreateExport,
  ImportPreview,
  ImportRequest,
  ImportResult,
  ImportUpload,
  OrielPreferences,
  OrielPreferencesBackup,
  QueuedRestore,
  RestorePreview,
  RestoreRequest,
  RestoreResult,
  ScheduledExport,
  SnapshotContents,
  SnapshotSummary,
  SnapshotTaken,
  UpdateExportSettings,
} from "$protocol";
import { ApiRequestError, type RestClient } from "./rest";

const enc = encodeURIComponent;

/** The key and client ID a raw (non-JSON) request sends, like the RestClient's own. */
export interface RawAuth {
  token: () => string | null;
  clientId?: string | null;
}

/** A downloaded export: its bytes and the file name the server gave it. */
export interface ExportFile {
  blob: Blob;
  fileName: string;
  /** The server's note (media files aren't in it). */
  note: string | null;
}

export class CodexBackupsClient {
  constructor(readonly rest: RestClient) {}

  // Settings backups.
  sets = () => this.rest.get<BackupSetSummary[]>("/codex/backups");
  snapshots = (set: string) => this.rest.get<SnapshotSummary[]>(`/codex/backups/${enc(set)}/snapshots`);
  /** This box's settings now (on a show box: offered to the site's Codex). */
  backUpNow = () => this.rest.request<SnapshotTaken>("POST", "/codex/backups/snapshots");
  /** A snapshot with its items' contents (admins). */
  contents = (set: string, snapshot: string) => this.rest.get<SnapshotContents>(`/codex/backups/${enc(set)}/snapshots/${enc(snapshot)}`);
  /** What restoring would change on `target` (a server ID; absent: the set's own box). */
  preview = (set: string, snapshot: string, target?: string) =>
    this.rest.get<RestorePreview>(`/codex/backups/${enc(set)}/snapshots/${enc(snapshot)}/preview`, target ? { target } : undefined);
  /** `RestoreResult` onto the Codex box itself, `QueuedRestore` (202) onto a show box. */
  restore = (set: string, snapshot: string, body: RestoreRequest) =>
    this.rest.request<RestoreResult | QueuedRestore>("POST", `/codex/backups/${enc(set)}/snapshots/${enc(snapshot)}/restore`, body);
  restores = () => this.rest.get<QueuedRestore[]>("/codex/backups/restores");
  cancelRestore = (id: string) => this.rest.request<void>("DELETE", `/codex/backups/restores/${enc(id)}`);

  // Oriel's own preferences (each client its own); `set`: another computer's (any paired client).
  orielPreferences = (snapshot?: string, set?: string) =>
    this.rest.get<OrielPreferencesBackup>(
      "/codex/backups/oriel",
      snapshot || set ? { ...(snapshot ? { snapshot } : {}), ...(set ? { set } : {}) } : undefined,
    );
  saveOrielPreferences = (body: OrielPreferences) => this.rest.request<SnapshotTaken>("PUT", "/codex/backups/oriel", body);

  // Nightly exports.
  exportStatus = () => this.rest.get<CodexExportStatus>("/codex/export/settings");
  saveExportSettings = (body: UpdateExportSettings) => this.rest.request<CodexExportStatus>("PUT", "/codex/export/settings", body);
  setPassphrase = (passphrase: string) => this.rest.request<CodexExportStatus>("PUT", "/codex/export/passphrase", { passphrase });
  clearPassphrase = () => this.rest.request<CodexExportStatus>("DELETE", "/codex/export/passphrase");
  scheduled = () => this.rest.get<ScheduledExport[]>("/codex/export/scheduled");

  // Import.
  previewImport = (id: string, passphrase: string) => this.rest.request<ImportPreview>("POST", `/codex/export/imports/${enc(id)}/preview`, { passphrase });
  applyImport = (id: string, body: ImportRequest) => this.rest.request<ImportResult>("POST", `/codex/export/imports/${enc(id)}/apply`, body);
  dropImport = (id: string) => this.rest.request<void>("DELETE", `/codex/export/imports/${enc(id)}`);

  /** The manual export: the encrypted file (no timeout: a large Codex takes a while). */
  async exportNow(body: CreateExport, auth: RawAuth): Promise<ExportFile> {
    const res = await raw(this.rest, auth, "POST", "/codex/export", JSON.stringify(body), "application/json");
    const cd = res.headers.get("Content-Disposition") ?? "";
    const fileName = /filename="?([^";]+)"?/.exec(cd)?.[1] ?? `vesper-codex-${new Date().toISOString().slice(0, 10)}.vcodex`;
    return { blob: await res.blob(), fileName, note: res.headers.get("X-Codex-Export-Note") };
  }

  /** Uploads an export file to import (kept an hour on the server). */
  async uploadImport(file: Blob, auth: RawAuth): Promise<ImportUpload> {
    const res = await raw(this.rest, auth, "POST", "/codex/export/imports", file, "application/octet-stream");
    return (await res.json()) as ImportUpload;
  }
}

/** A request whose body or answer isn't JSON; errors as `ApiRequestError`s. */
export async function raw(
  rest: RestClient,
  auth: RawAuth,
  method: string,
  path: string,
  body: BodyInit,
  type: string,
  extra: Record<string, string> = {},
): Promise<Response> {
  const headers: Record<string, string> = { Accept: "application/json, application/octet-stream", "Content-Type": type, ...extra };
  const t = auth.token();
  if (t) headers.Authorization = `Bearer ${t}`;
  if (auth.clientId) headers["X-Client-Id"] = auth.clientId;
  let res: Response;
  try {
    res = await fetch(rest.url(path), { method, headers, body });
  } catch {
    throw new ApiRequestError("network", "Could not reach the server", 0);
  }
  if (res.ok) return res;
  let parsed: { error?: { code?: string; message?: string; field?: string }; codex?: ApiRequestError["codex"] } | undefined;
  try {
    parsed = (await res.json()) as typeof parsed;
  } catch {
    /* not JSON */
  }
  const err = parsed?.error;
  throw new ApiRequestError(
    (err?.code ?? "internal") as ApiRequestError["code"],
    err?.message ?? `HTTP ${res.status}`,
    res.status,
    err?.field ?? null,
    undefined,
    parsed?.codex ?? null,
  );
}

/** True for a restore that was queued for a show box (rather than applied here). */
export function isQueued(r: RestoreResult | QueuedRestore): r is QueuedRestore {
  return "state" in r && "target" in r;
}
