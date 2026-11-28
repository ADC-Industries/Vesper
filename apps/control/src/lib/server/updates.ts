// Typed REST client for the site's update channel and its Vesper Launchers (G10,
// crates/protocol/README.md "Updates"): signed packages (list, upload with the signature,
// delete), each machine's Launcher (reports, queued Update / Roll back / Add app actions) and
// the server's check of Vesper's releases on GitHub (settings, last check, the catalogue).

import type { Catalogue, GithubReleases, GithubSettings, LauncherAction, LauncherEntry, UpdatePackage } from "$protocol";
import type { RestClient } from "./rest";
import { raw, type RawAuth } from "./codexBackups";

const enc = encodeURIComponent;

/** Base64 of a text (the `.minisig` file goes in a header). */
export function base64Text(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

export class UpdatesClient {
  constructor(readonly rest: RestClient) {}

  packages = () => this.rest.get<UpdatePackage[]>("/updates/packages");
  launchers = () => this.rest.get<LauncherEntry[]>("/updates/launchers");
  deletePackage = (id: string) => this.rest.request<void>("DELETE", `/updates/packages/${enc(id)}`);
  queue = (machineId: string, action: LauncherAction) =>
    this.rest.request<LauncherEntry>("POST", `/updates/launchers/${enc(machineId)}/actions`, action);
  github = () => this.rest.get<GithubReleases>("/updates/github");
  setGithub = (s: GithubSettings) => this.rest.request<GithubReleases>("PUT", "/updates/github", s);
  checkGithub = () => this.rest.request<GithubReleases>("POST", "/updates/github/check", {});
  /** What can be added ("Add app"); with a machine, only what it can run and doesn't have. */
  catalogue = (machineId?: string) => this.rest.get<Catalogue>("/updates/catalogue", machineId ? { machine: machineId } : undefined);

  /** Uploads a package with its minisign signature (admins); the server checks the signature. */
  async upload(file: Blob, fileName: string, signature: string, auth: RawAuth): Promise<UpdatePackage> {
    const path = `/updates/packages?file=${enc(fileName)}`;
    const res = await raw(this.rest, auth, "POST", path, file, "application/octet-stream", { "X-Vesper-Signature": base64Text(signature) });
    return (await res.json()) as UpdatePackage;
  }
}
