// Typed REST clients for media preparation and reports (crates/protocol/README.md, "Media
// preparation" and "Reports"): the Codex box's preparation status and settings, preparing,
// trimming and the original of a media file, and the CCLI usage report. Plain TypeScript over a
// RestClient, unit tested.

import type { CcliReport, MediaPrepSettings, MediaPrepStatus, PrepareMedia, RelinkedAsset, SetMediaTrim } from "$protocol";
import type { RestClient } from "./rest";

const enc = encodeURIComponent;

export class MediaPrepClient {
  constructor(readonly rest: RestClient) {}

  /** Settings, jobs, the converter and encoder found, live (on a show box: `409 codexElsewhere`). */
  status = () => this.rest.get<MediaPrepStatus>("/codex/media-prep");
  /** Saves the settings (admins). */
  setSettings = (s: MediaPrepSettings) => this.rest.request<MediaPrepSettings>("PUT", "/codex/media-prep", s);
  /** Queues a video or sound (`again`: from its original, even if it was prepared). */
  prepare = (collection: string, asset: string, again = false) =>
    this.rest.request<void>("POST", `/codex/collections/${enc(collection)}/media/${enc(asset)}/prepare`, { again } satisfies PrepareMedia);
  /** The trim, seconds (absent: none). */
  trim = (collection: string, asset: string, t: SetMediaTrim) =>
    this.rest.request<RelinkedAsset>("PUT", `/codex/collections/${enc(collection)}/media/${enc(asset)}/trim`, t);
  /** Plays the original again instead of its prepared copy. */
  useOriginal = (collection: string, asset: string) =>
    this.rest.request<RelinkedAsset>("POST", `/codex/collections/${enc(collection)}/media/${enc(asset)}/original`);
}

export class ReportsClient {
  constructor(readonly rest: RestClient) {}

  /** The CCLI usage report for `from`..`to` (`YYYY-MM-DD`, both included). */
  ccli = (from: string, to: string) => this.rest.get<CcliReport>("/reports/ccli", { from, to });
}
