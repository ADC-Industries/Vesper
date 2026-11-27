// Typed REST helpers over /api/v1 (crates/protocol/README.md, "REST").

import type {
  ChordProParse,
  ChordProParsed,
  RemoteCompatStatus,
  UpdateRemoteCompat,
  ServerDetails,
  CreateGlobalGroup,
  CreateMediaBin,
  DocumentLock,
  MediaBinEdit,
  MediaBinEditResult,
  PresenceList,
  LiveSourceList,
  VideoInputEntry,
  UpdateGlobalGroup,
  UpdateMediaBin,
  UpdateTheme,
  DisplayInfo,
  CodexBox,
  CodexStatus,
  ApiError,
  AudioDeviceConfig,
  AudioDeviceInfo,
  CaptionModel,
  CaptionsConfig,
  PushCaption,
  ClearGroup,
  Command,
  CreateLibrary,
  CreatePlaylistNode,
  CreatePresentation,
  DeletedDocuments,
  DocumentKind,
  DocumentRevision,
  DocumentUse,
  DuplicatePlaylist,
  DuplicatePresentation,
  GlobalGroup,
  PlaylistEdit,
  PlaylistEditResult,
  PresentationEdit,
  PresentationEditResult,
  UpdatePlaylistNode,
  CommandResponse,
  DiagnosticsReport,
  ErrorCode,
  FontList,
  MediaAssetInfo,
  LibrarySummary,
  Look,
  LookEntry,
  Mask,
  MaskDeleted,
  MaskEntry,
  MediaBinDetail,
  MediaBinNode,
  OutputBinding,
  OutputEntry,
  OutputTransportInfo,
  PairPending,
  PairResponse,
  PlaylistDetail,
  PlaylistTreeNode,
  Presentation,
  PresentationSummary,
  Screen,
  ScreenDeleted,
  ColorCorrection,
  CornerPin,
  ScreenEntry,
  SearchResults,
  ServerInfo,
  StateSnapshot,
  ThumbnailPrefetch,
  MessageEntry,
  PropEntry,
  SlideRef,
  StageLayoutEntry,
  StageLayoutTemplate,
  ThemeEntry,
  ThemeSummary,
  TimerEntry,
  TranslationModels,
  TranslationModel,
  TranslatePreview,
  TranslateJob,
} from "$protocol";

/** The stored documents behind the entries (an entry is the document plus `revision`). */
export type TimerDoc = Omit<TimerEntry, "revision">;
export type MessageDoc = Omit<MessageEntry, "revision">;
export type PropDoc = Omit<PropEntry, "revision">;
export type StageLayoutDoc = Omit<StageLayoutEntry, "revision">;

/** A document without the entry's `revision` field (PUT bodies). */
export function withoutRevision<T extends object>(doc: T): Omit<T, "revision"> {
  const { revision: _r, ...rest } = doc as T & { revision?: string };
  return rest;
}

/** An error answer from the server (or a network failure, as code `unavailable`). */
export class ApiRequestError extends Error {
  constructor(
    public readonly code: ErrorCode | "network" | "timeout",
    message: string,
    public readonly status: number,
    /** The request field a validation error is about (`name`, `transport.sourceName`, …). */
    public readonly field: string | null = null,
    /** On a 409 edit conflict: the document as it is now (`null` when it was deleted). */
    public readonly current: unknown = undefined,
    /** On a show box's `409 codexElsewhere` (or `503`): the box that holds the site's Codex. */
    public readonly codex: CodexBox | null = null,
    /** On `409 liveShow`: what is live on the box, each named for people. */
    public readonly live: string[] | null = null,
  ) {
    super(message);
    this.name = "ApiRequestError";
  }
}

/** Extra request options: `ifMatch` sends `If-Match: "<revision>"` (optimistic edits). */
export interface RequestOptions {
  ifMatch?: string | null;
}

/** Normalises a user-typed address ("booth", "10.0.0.5:5555", "http://…/") to an origin. */
export function normaliseBaseUrl(input: string, defaultPort: number): string {
  let s = input.trim();
  if (!s) throw new Error("Enter a server address");
  const hadScheme = /^https?:\/\//i.test(s);
  if (!hadScheme) s = `http://${s}`;
  const url = new URL(s);
  // "booth" or "10.0.0.5" means the default API port. A full URL is taken as typed.
  const hostPart = s.replace(/^https?:\/\//i, "").split("/")[0] ?? "";
  if (!hadScheme && !/:\d+$/.test(hostPart)) url.port = String(defaultPort);
  return url.origin;
}

export interface RestOptions {
  /** Origin of the server, e.g. `http://booth.local:5555` (no trailing slash). */
  baseUrl: string;
  /** Returns the pairing token, if any. */
  token: () => string | null;
  /** Request timeout in ms. */
  timeoutMs?: number;
  fetch?: typeof fetch;
}

export class RestClient {
  readonly baseUrl: string;
  /** Sent as `X-Client-Id` (echoed in `documentChanged`, so a client can skip its own edits). */
  clientId: string | null = null;
  /**
   * IDs of props opened as documents in the Edit view (prop-document mode): `presentation`,
   * `editPresentation`, `slideRenderUrl` and `thumbnailUrl` route these to `/props/{id}/…`.
   */
  readonly propDocuments = new Set<string>();
  /** IDs of stage layouts opened as documents in the Edit view (routed to `/stage-layouts/{id}/…`). */
  readonly stageLayoutDocuments = new Set<string>();
  /** IDs of themes opened as documents in the Edit view (routed to `/themes/{id}/…`). */
  readonly themeDocuments = new Set<string>();

  /** The route base of a document that isn't a presentation (prop, stage layout, theme), or null. */
  private oneSlideBase(id: string): string | null {
    if (this.propDocuments.has(id)) return "/props";
    if (this.stageLayoutDocuments.has(id)) return "/stage-layouts";
    if (this.themeDocuments.has(id)) return "/themes";
    return null;
  }
  private token: () => string | null;
  private timeoutMs: number;
  private fetchImpl: typeof fetch;

  constructor(opts: RestOptions) {
    this.baseUrl = opts.baseUrl.replace(/\/+$/, "");
    this.token = opts.token;
    this.timeoutMs = opts.timeoutMs ?? 8000;
    this.fetchImpl = opts.fetch ?? ((...a) => fetch(...a));
  }

  /** Absolute URL of an API path, optionally with the token as a query parameter. */
  url(path: string, params: Record<string, string | number> = {}, withToken = false): string {
    const u = new URL(`${this.baseUrl}/api/v1${path}`);
    for (const [k, v] of Object.entries(params)) u.searchParams.set(k, String(v));
    const t = withToken ? this.token() : null;
    if (t) u.searchParams.set("token", t);
    return u.toString();
  }

  /** WebSocket URL (`ws://…/api/v1/ws?protocolVersion=1&token=…`). */
  wsUrl(protocolVersion: number): string {
    const u = new URL(this.url("/ws", { protocolVersion }, true));
    u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
    return u.toString();
  }

  async request<T>(
    method: string,
    path: string,
    body?: unknown,
    params?: Record<string, string | number>,
    opts: RequestOptions = {},
  ): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (opts.ifMatch) headers["If-Match"] = `"${opts.ifMatch}"`;
    if (this.clientId) headers["X-Client-Id"] = this.clientId;
    const t = this.token();
    if (t) headers.Authorization = `Bearer ${t}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), this.timeoutMs);
    let res: Response;
    let text: string;
    try {
      res = await this.fetchImpl(this.url(path, params), {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: ctrl.signal,
      });
      // The body too: the link can drop (or stall) after the headers came.
      text = await res.text();
    } catch {
      const aborted = ctrl.signal.aborted;
      throw new ApiRequestError(
        aborted ? "timeout" : "network",
        aborted ? "The server did not answer in time" : "Could not reach the server",
        0,
      );
    } finally {
      clearTimeout(timer);
    }
    let json: unknown = undefined;
    if (text) {
      try {
        json = JSON.parse(text);
      } catch {
        if (!res.ok) throw new ApiRequestError("internal", text.slice(0, 200), res.status);
        throw new ApiRequestError("internal", "The server sent something that is not JSON", res.status);
      }
    }
    if (!res.ok) {
      const body = json as { error?: ApiError; current?: unknown; codex?: CodexBox } | undefined;
      const err = body?.error;
      throw new ApiRequestError(
        err?.code ?? "internal",
        err?.message ?? `HTTP ${res.status}`,
        res.status,
        err?.field ?? null,
        body && "current" in body ? body.current : undefined,
        body?.codex ?? null,
        err?.live ?? null,
      );
    }
    return json as T;
  }

  get<T>(path: string, params?: Record<string, string | number>): Promise<T> {
    return this.request<T>("GET", path, undefined, params);
  }

  serverInfo = () => this.get<ServerInfo>("/server");
  /** This box's view of the site's Codex (`GET /codex`): role, active collection, sync. */
  codex = () => this.get<CodexStatus>("/codex");
  /** This Vesper's version, OS, CPU, GPU, uptime and products (G9). */
  serverDetails = () => this.get<ServerDetails>("/server/details");
  /**
   * Pairs with the code shown on the server (`PairResponse`), or, without one (or at a server
   * of the site that isn't the site's server), asks a site admin's approval (`PairPending`:
   * wait with `pairingStatus`). `browser`: this is a web browser (its key lives a year).
   */
  pair = (code: string | null, clientName: string, browser = false) =>
    this.request<PairResponse | PairPending>("POST", "/pair", {
      ...(code ? { code } : {}),
      clientName,
      ...(browser ? { browser: true } : {}),
    });
  /** A waiting pairing's answer: the key once approved, `PairPending` while it waits. */
  pairingStatus = (pairingId: string) =>
    this.get<PairResponse | PairPending>(`/pair/${encodeURIComponent(pairingId)}`);
  /** Takes a waiting pairing back (the client cancels, or types its code again). */
  withdrawPairing = (pairingId: string) =>
    this.request<void>("DELETE", `/pair/${encodeURIComponent(pairingId)}`);
  /**
   * Swaps a credential from before site keys (the one this client sends) for its site key, once.
   * `browser`: this is a web browser (its key lives a year).
   */
  upgradeCredential = (browser = false) =>
    this.request<PairResponse>("POST", "/pair/upgrade", browser ? { browser: true } : undefined);
  state = () => this.get<StateSnapshot>("/state");
  libraries = () => this.get<LibrarySummary[]>("/libraries");
  libraryPresentations = (id: string) => this.get<PresentationSummary[]>(`/libraries/${id}/presentations`);
  presentations = () => this.get<PresentationSummary[]>("/presentations");
  /**
   * Codex search (`GET /codex/search`): this box's active collection unless `collection` says
   * otherwise (an ID or `"all"`); `kinds` comma-separated document kinds.
   */
  codexSearch = (q: string, opts: { collection?: string; kinds?: DocumentKind[]; limit?: number } = {}) => {
    const params: Record<string, string | number> = { q };
    if (opts.collection) params.collection = opts.collection;
    if (opts.kinds?.length) params.kinds = opts.kinds.join(",");
    if (opts.limit) params.limit = opts.limit;
    return this.get<SearchResults>("/codex/search", params);
  };
  presentation = (id: string) => {
    const base = this.oneSlideBase(id);
    return this.get<Presentation>(base ? `${base}/${id}/document` : `/presentations/${id}`);
  };
  playlists = () => this.get<PlaylistTreeNode[]>("/playlists");
  playlist = (id: string) => this.get<PlaylistDetail>(`/playlists/${id}`);
  looks = () => this.get<Look[]>("/looks");
  // Looks (crates/protocol/README.md "Looks"): documents with revisions; edits send If-Match.
  look = (id: string) => this.get<LookEntry>(`/looks/${id}`);
  createLook = (doc: Partial<Look> = {}) => this.request<LookEntry>("POST", "/looks", doc);
  updateLook = (doc: Look, revision: string | null) =>
    this.request<LookEntry>("PUT", `/looks/${doc.id}`, doc, undefined, { ifMatch: revision });
  duplicateLook = (id: string, name?: string) => this.request<LookEntry>("POST", `/looks/${id}/duplicate`, name ? { name } : {});
  deleteLook = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/looks/${id}`, undefined, undefined, { ifMatch: revision });
  // Masks (same rules); a mask that looks use is deleted only with `detach` (they forget it).
  masks = () => this.get<MaskEntry[]>("/masks");
  createMask = (doc: Partial<Mask> = {}) => this.request<MaskEntry>("POST", "/masks", doc);
  updateMask = (doc: Mask, revision: string | null) =>
    this.request<MaskEntry>("PUT", `/masks/${doc.id}`, doc, undefined, { ifMatch: revision });
  duplicateMask = (id: string, name?: string) => this.request<MaskEntry>("POST", `/masks/${id}/duplicate`, name ? { name } : {});
  deleteMask = (id: string, revision: string | null, detach = false) =>
    this.request<MaskDeleted>("DELETE", `/masks/${id}`, undefined, detach ? { detach: "true" } : undefined, { ifMatch: revision });
  screens = () => this.get<Screen[]>("/screens");
  clearGroups = () => this.get<ClearGroup[]>("/clear-groups");

  // Screen configuration (documents with revisions; edits send If-Match).
  screenEntries = () => this.get<ScreenEntry[]>("/screens");
  createScreen = (doc: Partial<Screen>) => this.request<ScreenEntry>("POST", "/screens", doc);
  updateScreen = (doc: Screen, revision: string | null) =>
    this.request<ScreenEntry>("PUT", `/screens/${doc.id}`, doc, undefined, { ifMatch: revision });
  /** A screen's colour correction / corner pin, applied live (no If-Match; `{}` resets). */
  setScreenColor = (id: string, color: Partial<ColorCorrection>) => this.request<ScreenEntry>("PUT", `/screens/${id}/color`, color);
  setScreenCornerPin = (id: string, pin: Partial<CornerPin>) => this.request<ScreenEntry>("PUT", `/screens/${id}/corner-pin`, pin);
  deleteScreen = (id: string, revision: string | null) =>
    this.request<ScreenDeleted>("DELETE", `/screens/${id}`, undefined, undefined, { ifMatch: revision });
  outputs = () => this.get<OutputEntry[]>("/outputs");
  createOutput = (doc: Partial<OutputBinding> & Pick<OutputBinding, "screen">) =>
    this.request<OutputEntry>("POST", "/outputs", doc);
  updateOutput = (doc: OutputBinding, revision: string | null) =>
    this.request<OutputEntry>("PUT", `/outputs/${doc.id}`, doc, undefined, { ifMatch: revision });
  deleteOutput = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/outputs/${id}`, undefined, undefined, { ifMatch: revision });
  outputTransports = () => this.get<OutputTransportInfo[]>("/output-transports");
  /** The monitors and projectors attached to the box (capability `displays`). */
  displays = () => this.get<DisplayInfo[]>("/displays");
  /** A display no output uses shows its name for a few seconds (capability `screenLayouts`). */
  identifyDisplay = (id: string) => this.request<void>("POST", `/displays/${encodeURIComponent(id)}/identify`);
  diagnostics = () => this.get<DiagnosticsReport>("/diagnostics");
  // The machine's audio output (applied live when saved).
  audioDevices = () => this.get<AudioDeviceInfo[]>("/audio-devices");
  audioDevice = () => this.get<AudioDeviceConfig>("/audio-device");
  saveAudioDevice = (cfg: AudioDeviceConfig) => this.request<AudioDeviceConfig>("PUT", "/audio-device", cfg);
  // Live captions (docs/CAPTIONS-PLAN.md): the box's configuration (applied live), the models.
  captionsConfig = () => this.get<CaptionsConfig>("/captions/config");
  saveCaptionsConfig = (cfg: CaptionsConfig) => this.request<CaptionsConfig>("PUT", "/captions/config", cfg);
  captionModels = () => this.get<CaptionModel[]>("/captions/models");
  pushCaption = (line: PushCaption) => this.request<void>("POST", "/captions/lines", line);
  // Machine translation (docs/DUAL-LANGUAGE-PLAN.md §5): the box's models (changing them: the
  // site's admin) and previews (never saved: Keep sends `setSlideTranslation`).
  translationModels = () => this.get<TranslationModels>("/translate/models");
  downloadTranslationModel = (id: string) => this.request<TranslationModel>("POST", `/translate/models/${encodeURIComponent(id)}/download`);
  stopTranslationModel = (id: string) => this.request<TranslationModel>("POST", `/translate/models/${encodeURIComponent(id)}/stop`);
  deleteTranslationModel = (id: string) => this.request<TranslationModel>("DELETE", `/translate/models/${encodeURIComponent(id)}`);
  translatePreview = (req: TranslatePreview) => this.request<TranslateJob>("POST", "/translate/preview", req);
  translateJob = (id: string) => this.get<TranslateJob>(`/translate/jobs/${encodeURIComponent(id)}`);
  cancelTranslateJob = (id: string) => this.request<void>("DELETE", `/translate/jobs/${encodeURIComponent(id)}`);
  // Control surfaces: the Remote API compatibility layer (capability `remoteCompat`).
  remoteCompat = () => this.get<RemoteCompatStatus>("/remote-compat");
  saveRemoteCompat = (change: UpdateRemoteCompat) => this.request<RemoteCompatStatus>("PUT", "/remote-compat", change);
  mediaBins = () => this.get<MediaBinNode[]>("/media-bins");
  mediaBin = (id: string) => this.get<MediaBinDetail>(`/media-bins/${id}`);
  prefetchThumbnails = (id: string, w: number) =>
    this.request<ThumbnailPrefetch>("POST", `/presentations/${id}/thumbnails`, undefined, { w });
  // Content editing (crates/protocol/README.md, "Editing content").
  editPresentation = (id: string, body: PresentationEdit) =>
    this.request<PresentationEditResult>("POST", `${this.oneSlideBase(id) ?? "/presentations"}/${id}/edits`, body);
  editPlaylist = (id: string, body: PlaylistEdit) => this.request<PlaylistEditResult>("POST", `/playlists/${id}/edits`, body);
  createLibrary = (body: CreateLibrary) => this.request<DocumentRevision>("POST", "/libraries", body);
  renameLibrary = (id: string, name: string, revision: string | null = null) =>
    this.request<DocumentRevision>("PUT", `/libraries/${id}`, { name }, undefined, { ifMatch: revision });
  deleteLibrary = (id: string) => this.request<DeletedDocuments>("DELETE", `/libraries/${id}`);
  createPresentation = (body: CreatePresentation) => this.request<DocumentRevision>("POST", "/presentations", body);
  duplicatePresentation = (id: string, body: DuplicatePresentation = {}) =>
    this.request<DocumentRevision>("POST", `/presentations/${id}/duplicate`, body);
  movePresentation = (id: string, library: string) =>
    this.request<DocumentRevision>("POST", `/presentations/${id}/move`, { library });
  deletePresentation = (id: string) => this.request<DeletedDocuments>("DELETE", `/presentations/${id}`);
  createPlaylist = (body: CreatePlaylistNode) => this.request<DocumentRevision>("POST", "/playlists", body);
  updatePlaylist = (id: string, body: UpdatePlaylistNode) => this.request<DocumentRevision>("PUT", `/playlists/${id}`, body);
  duplicatePlaylist = (id: string, body: DuplicatePlaylist = {}) =>
    this.request<DocumentRevision>("POST", `/playlists/${id}/duplicate`, body);
  deletePlaylist = (id: string) => this.request<DeletedDocuments>("DELETE", `/playlists/${id}`);
  globalGroups = () => this.get<GlobalGroup[]>("/groups");
  // Media bins and global groups (G17): documents of the collection, like playlists.
  createMediaBin = (body: CreateMediaBin) => this.request<DocumentRevision>("POST", "/media-bins", body);
  updateMediaBin = (id: string, body: UpdateMediaBin) => this.request<DocumentRevision>("PUT", `/media-bins/${id}`, body);
  deleteMediaBin = (id: string) => this.request<DeletedDocuments>("DELETE", `/media-bins/${id}`);
  editMediaBin = (id: string, body: MediaBinEdit) => this.request<MediaBinEditResult>("POST", `/media-bins/${id}/edits`, body);
  createGlobalGroup = (body: CreateGlobalGroup) => this.request<DocumentRevision>("POST", "/groups", body);
  updateGlobalGroup = (id: string, body: UpdateGlobalGroup) => this.request<DocumentRevision>("PUT", `/groups/${id}`, body);
  deleteGlobalGroup = (id: string) => this.request<DeletedDocuments>("DELETE", `/groups/${id}`);
  fonts = () => this.get<FontList>("/fonts");
  media = (id: string) => this.get<MediaAssetInfo>(`/media/${id}`);
  themes = () => this.get<ThemeSummary[]>("/themes");
  // Themes (linked themes, crates/protocol/README.md "Themes").
  theme = (id: string) => this.get<ThemeEntry>(`/themes/${id}`);
  createTheme = (name?: string) => this.request<DocumentRevision>("POST", "/themes", name ? { name } : {});
  renameTheme = (id: string, name: string, revision: string | null = null) =>
    this.request<DocumentRevision>("PUT", `/themes/${id}`, { name }, undefined, { ifMatch: revision });
  /** Changes a theme's fields (`UpdateTheme`: name, second-language style). */
  updateTheme = (id: string, body: UpdateTheme, revision: string | null = null) =>
    this.request<DocumentRevision>("PUT", `/themes/${id}`, body, undefined, { ifMatch: revision });
  duplicateTheme = (id: string, name?: string) => this.request<DocumentRevision>("POST", `/themes/${id}/duplicate`, name ? { name } : {});
  /** Deletes a theme; songs that follow it move to `reassign`, or keep their look (`unlink`). */
  deleteTheme = (id: string, opts: { reassign?: string; unlink?: boolean } = {}) =>
    this.request<DeletedDocuments>("DELETE", `/themes/${id}`, undefined, {
      ...(opts.reassign ? { reassign: opts.reassign } : {}),
      ...(opts.unlink ? { unlink: "true" } : {}),
    });
  /** Who follows the theme (with `template`: that template slide of it). */
  themeUsedBy = (id: string, template?: string) => this.get<DocumentUse[]>(`/themes/${id}/used-by`, template ? { template } : undefined);
  /** A template slide's picture (PNG); `v` changes with the theme (the browser asks again). */
  themeThumbnailUrl(theme: string, slide: string, w: number, v = ""): string {
    return this.url(`/themes/${theme}/slides/${slide}/thumbnail`, v ? { w, v } : { w }, true);
  }

  // Presence and edit locks (crates/protocol/README.md "Presence and edit locks").
  presence = () => this.get<PresenceList>("/presence");

  // Live network sources (G7, the bin's Video Inputs).
  liveSources = () => this.get<LiveSourceList>("/sources");
  /** Logical video inputs with this box's binding (`source`), in order. */
  videoInputs = () => this.get<VideoInputEntry[]>("/video-inputs");
  createVideoInput = (doc: Partial<Omit<VideoInputEntry, "revision">>) => this.request<VideoInputEntry>("POST", "/video-inputs", doc);
  /** The whole input; `source: null` unbinds it on this box. */
  updateVideoInput = (doc: Omit<VideoInputEntry, "revision" | "source"> & { source: VideoInputEntry["source"] | null }, revision: string | null) =>
    this.request<VideoInputEntry>("PUT", `/video-inputs/${doc.id}`, doc, undefined, { ifMatch: revision });
  deleteVideoInput = (id: string, revision: string | null) => this.request<void>("DELETE", `/video-inputs/${id}`, undefined, undefined, { ifMatch: revision });
  /** The newest picture of a source as JPEG (`204` while it connects); `v` busts the cache. */
  sourceThumbnailUrl(source: string, w: number, v: number | string): string {
    return this.url("/sources/thumbnail", { source, w, v }, true);
  }
  /** Takes a document's edit lock (`takeOver`: from whoever holds it); `423 locked` when held. */
  takeLock = (kind: DocumentKind, id: string, takeOver = false) =>
    this.request<DocumentLock>("POST", "/presence/locks", { kind, id, ...(takeOver ? { takeOver } : {}) });
  releaseLock = (kind: DocumentKind, id: string) => this.request<void>("DELETE", `/presence/locks/${kind}/${id}`);

  // Timers, messages and props (documents with revisions; edits send If-Match).
  timers = () => this.get<TimerEntry[]>("/timers");
  createTimer = (doc: Partial<TimerDoc>) => this.request<TimerEntry>("POST", "/timers", doc);
  updateTimer = (doc: TimerDoc, revision: string | null) =>
    this.request<TimerEntry>("PUT", `/timers/${doc.id}`, withoutRevision(doc), undefined, { ifMatch: revision });
  deleteTimer = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/timers/${id}`, undefined, undefined, { ifMatch: revision });
  messages = () => this.get<MessageEntry[]>("/messages");
  createMessage = (doc: Partial<MessageDoc>) => this.request<MessageEntry>("POST", "/messages", doc);
  updateMessage = (doc: MessageDoc, revision: string | null) =>
    this.request<MessageEntry>("PUT", `/messages/${doc.id}`, withoutRevision(doc), undefined, { ifMatch: revision });
  deleteMessage = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/messages/${id}`, undefined, undefined, { ifMatch: revision });
  props = () => this.get<PropEntry[]>("/props");
  createProp = (doc: Partial<PropDoc> & { fromSlide?: SlideRef }) => this.request<PropEntry>("POST", "/props", doc);
  updateProp = (doc: PropDoc, revision: string | null) =>
    this.request<PropEntry>("PUT", `/props/${doc.id}`, withoutRevision(doc), undefined, { ifMatch: revision });
  deleteProp = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/props/${id}`, undefined, undefined, { ifMatch: revision });

  // Stage layouts (documents with revisions; edits send If-Match).
  stageLayouts = () => this.get<StageLayoutEntry[]>("/stage-layouts");
  stageLayoutTemplates = () => this.get<StageLayoutTemplate[]>("/stage-layout-templates");
  createStageLayout = (doc: Partial<StageLayoutDoc> & { template?: string }) =>
    this.request<StageLayoutEntry>("POST", "/stage-layouts", doc);
  updateStageLayout = (doc: StageLayoutDoc, revision: string | null) =>
    this.request<StageLayoutEntry>("PUT", `/stage-layouts/${doc.id}`, withoutRevision(doc), undefined, { ifMatch: revision });
  /** ChordPro → a new, unsaved presentation (batch 16). */
  parseChordPro = (body: ChordProParse) => this.request<ChordProParsed>("POST", "/chordpro/parse", body);
  /** A presentation as a ChordPro file (download URL, token in the query). */
  chordProUrl(presentation: string): string {
    return this.url(`/presentations/${presentation}/chordpro`, {}, true);
  }
  deleteStageLayout = (id: string, revision: string | null) =>
    this.request<void>("DELETE", `/stage-layouts/${id}`, undefined, undefined, { ifMatch: revision });

  /** A stage layout's picture as designed (transparent PNG); `v` = its revision. */
  stageLayoutThumbnailUrl(id: string, w: number, v: string): string {
    return this.url(`/stage-layouts/${id}/thumbnail`, { w, v }, true);
  }

  /** A built-in template's picture (`currentNext` is the built-in default layout). */
  stageTemplateThumbnailUrl(id: string, w: number): string {
    return this.url(`/stage-layout-templates/${id}/thumbnail`, { w }, true);
  }

  /** A prop's picture (transparent PNG); `v` changes with the prop (its revision). */
  propThumbnailUrl(prop: string, w: number, v: string): string {
    return this.url(`/props/${prop}/thumbnail`, { w, v }, true);
  }

  /** Sends a command over HTTP (the WebSocket is preferred; see Connection.command). */
  command = (c: Command) => this.request<CommandResponse>("POST", "/commands", c);

  /** Media thumbnail URL for an `<img>` (token in the query). */
  mediaThumbnailUrl(asset: string, w: number): string {
    return this.url(`/media/${asset}/thumbnail`, { w }, true);
  }

  /**
   * The Edit view's picture of a slide (transparent PNG, never cached by the server): `hide`
   * leaves elements out, `v` changes whenever the slide does (so the browser asks again).
   */
  slideRenderUrl(presentation: string, slide: string, w: number, hide: string[], v: string, chords = false): string {
    const params: Record<string, string | number> = { w, v };
    if (hide.length) params.hide = hide.join(",");
    if (chords) params.chords = 1;
    const base = this.oneSlideBase(presentation) ?? "/presentations";
    return this.url(`${base}/${presentation}/slides/${slide}/render`, params, true);
  }

  /** Thumbnail URL for an `<img>` (token in the query, since images cannot send headers). */
  thumbnailUrl(presentation: string, slide: string, w: number): string {
    const base = this.oneSlideBase(presentation);
    if (base === "/themes") return this.themeThumbnailUrl(presentation, slide, w);
    if (base) return this.url(`${base}/${presentation}/thumbnail`, { w }, true);
    return this.url(`/presentations/${presentation}/slides/${slide}/thumbnail`, { w }, true);
  }
}
