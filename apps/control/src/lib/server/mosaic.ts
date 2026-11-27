// Typed REST client for Vesper Mosaic on the site's server (crates/protocol/README.md "Mosaic"):
// the site's display boxes, their screens' feeds and settings, identify and test pattern, and
// the site-wide channels, and the video walls.

import type {
  MosaicChannel,
  MosaicFeed,
  MosaicServiceMode,
  MosaicIdentify,
  MosaicScreenSettings,
  MosaicState,
  MosaicTestPattern,
  MosaicWall,
  PutMosaicBox,
  PutMosaicChannel,
  PutMosaicWall,
} from "$protocol";
import type { RestClient } from "./rest";
import { raw, type RawAuth } from "./codexBackups";

const enc = encodeURIComponent;

export class MosaicClient {
  constructor(readonly rest: RestClient) {}

  private screen(machine: string, n: number, what: string): string {
    return `/mosaic/boxes/${enc(machine)}/screens/${n}/${what}`;
  }

  state = () => this.rest.get<MosaicState>("/mosaic");
  putBox = (machine: string, body: PutMosaicBox) => this.rest.request<MosaicState>("PUT", `/mosaic/boxes/${enc(machine)}`, body);
  forgetBox = (machine: string) => this.rest.request<void>("DELETE", `/mosaic/boxes/${enc(machine)}`);
  setFeed = (machine: string, n: number, feed: MosaicFeed) => this.rest.request<MosaicState>("PUT", this.screen(machine, n, "feed"), feed);
  setSettings = (machine: string, n: number, s: MosaicScreenSettings) => this.rest.request<MosaicState>("PUT", this.screen(machine, n, "settings"), s);
  identify = (machine: string, n: number, body: MosaicIdentify = {}) => this.rest.request<void>("POST", this.screen(machine, n, "identify"), body);
  testPattern = (machine: string, n: number, body: MosaicTestPattern) => this.rest.request<void>("POST", this.screen(machine, n, "test-pattern"), body);
  identifyAll = (body: MosaicIdentify = {}) => this.rest.request<void>("POST", "/mosaic/identify", body);
  testPatternAll = (body: MosaicTestPattern) => this.rest.request<void>("POST", "/mosaic/test-pattern", body);
  createChannel = (body: PutMosaicChannel) => this.rest.request<MosaicChannel>("POST", "/mosaic/channels", body);
  putChannel = (id: string, body: PutMosaicChannel) => this.rest.request<MosaicState>("PUT", `/mosaic/channels/${enc(id)}`, body);
  deleteChannel = (id: string) => this.rest.request<void>("DELETE", `/mosaic/channels/${enc(id)}`);
  createWall = (body: PutMosaicWall) => this.rest.request<MosaicWall>("POST", "/mosaic/walls", body);
  putWall = (id: string, body: PutMosaicWall) => this.rest.request<MosaicState>("PUT", `/mosaic/walls/${enc(id)}`, body);
  setWallFeed = (id: string, feed: MosaicFeed) => this.rest.request<MosaicState>("PUT", `/mosaic/walls/${enc(id)}/feed`, feed);
  identifyWall = (id: string, body: MosaicIdentify = {}) => this.rest.request<void>("POST", `/mosaic/walls/${enc(id)}/identify`, body);
  deleteWall = (id: string) => this.rest.request<void>("DELETE", `/mosaic/walls/${enc(id)}`);
  /** "Use as a display box": the machine's IDN display service (admins). */
  setService = (machine: string, mode: MosaicServiceMode) => this.rest.request<MosaicState>("PUT", `/mosaic/boxes/${enc(machine)}/service`, { mode });
  deleteLogo = () => this.rest.request<void>("DELETE", "/mosaic/logo");

  /** The site's logo for the boxes' fallback (a PNG or JPEG; admins). */
  async putLogo(file: Blob, auth: RawAuth): Promise<MosaicState> {
    const res = await raw(this.rest, auth, "PUT", "/mosaic/logo", file, file.type || "application/octet-stream");
    return (await res.json()) as MosaicState;
  }

  /** The kept logo (`hash` changes with it). */
  logoUrl(hash: string): string {
    return this.rest.url("/mosaic/logo", { v: hash }, true);
  }

  /** The thumbnail of what a screen shows (`v` changes it so the browser fetches it again). */
  thumbnailUrl(machine: string, n: number, w: number, v: number): string {
    return this.rest.url(this.screen(machine, n, "thumbnail"), { w, v }, true);
  }
}
