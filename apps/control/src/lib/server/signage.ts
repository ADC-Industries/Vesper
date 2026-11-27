// Typed REST client for the site's signage (crates/protocol/README.md "Signage"): channels,
// playlists, feeds, settings, the place search and the override.

import type { PutSignageChannel, SignageChannel, SignageFeed, SignageFeedPreview, SignageOverride, SignagePlaces, SignagePlaylist, SignageSettings, SignageState } from "$protocol";
import type { RestClient } from "./rest";

const enc = encodeURIComponent;

export class SignageClient {
  constructor(readonly rest: RestClient) {}

  state = () => this.rest.get<SignageState>("/signage");
  createChannel = (body: PutSignageChannel) => this.rest.request<SignageChannel>("POST", "/signage/channels", body);
  putChannel = (id: string, body: PutSignageChannel) => this.rest.request<SignageChannel>("PUT", `/signage/channels/${enc(id)}`, body);
  deleteChannel = (id: string) => this.rest.request<void>("DELETE", `/signage/channels/${enc(id)}`);
  createPlaylist = (body: SignagePlaylist) => this.rest.request<SignagePlaylist>("POST", "/signage/playlists", body);
  putPlaylist = (id: string, body: SignagePlaylist) => this.rest.request<SignagePlaylist>("PUT", `/signage/playlists/${enc(id)}`, body);
  deletePlaylist = (id: string) => this.rest.request<void>("DELETE", `/signage/playlists/${enc(id)}`);
  createFeed = (body: SignageFeed) => this.rest.request<SignageFeed>("POST", "/signage/feeds", body);
  putFeed = (id: string, body: SignageFeed) => this.rest.request<SignageFeed>("PUT", `/signage/feeds/${enc(id)}`, body);
  deleteFeed = (id: string) => this.rest.request<void>("DELETE", `/signage/feeds/${enc(id)}`);
  previewFeed = (body: SignageFeed) => this.rest.request<SignageFeedPreview>("POST", "/signage/feeds/preview", body);
  putSettings = (body: SignageSettings) => this.rest.request<SignageSettings>("PUT", "/signage/settings", body);
  places = (q: string) => this.rest.get<SignagePlaces>("/signage/places", { q });
  pushOverride = (body: SignageOverride) => this.rest.request<SignageOverride>("POST", "/signage/override", body);
  clearOverride = () => this.rest.request<void>("DELETE", "/signage/override");
}
