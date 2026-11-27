// Fine-grained views of the show state. Each store only notifies when its own slice changes,
// so the many state messages (fps counters refresh every few seconds) don't re-render the
// slide grid or the sidebar.

import type { AudioState, BuildProgress, CaptionsState, ClockFormatState, DiagnosticsSummary, DisplayInfo, Layer, MediaLayerState, LayerStates, MediaTransport, TimerState, ScreenState, ScreensEnabled, ShowOptions, SlidePosition, StageMessageState, StageScreenState } from "$protocol";
import { getContext } from "svelte";
import type { Readable } from "svelte/store";
import { layerActivity, type LayerActivity } from "./model";
import type { Connection } from "./server";
import { jsonEqual, select } from "./server";

export interface Live {
  /** What is on the slide layer. */
  slide: Readable<SlidePosition | null>;
  /** What is on the announcements layer (presentations whose destination is announcements). */
  announcement: Readable<SlidePosition | null>;
  /** The slide triggered last: the slide layer's, or the announcements layer's (`LatestLive`). */
  latest: Readable<SlidePosition | null>;
  /** What `next` would trigger. */
  cursor: Readable<SlidePosition | null>;
  /** Live slide ID only (cheap to compare in every thumbnail). */
  liveSlideId: Readable<string | null>;
  cursorSlideId: Readable<string | null>;
  /** Playlist item of the live slide. */
  livePlaylistItem: Readable<string | null>;
  activity: Readable<LayerActivity>;
  /** Empty layers whose last cleared content the server can bring back (`restoreLayer`). */
  restorable: Readable<Layer[]>;
  activeLook: Readable<string | null>;
  screens: Readable<ScreenState[]>;
  diagnostics: Readable<DiagnosticsSummary | null>;
  screensEnabled: Readable<ScreensEnabled>;
  /** Screen labels shown on every output (`identifyScreens`). */
  identify: Readable<boolean>;
  options: Readable<ShowOptions>;
  layers: Readable<LayerStates | null>;
  timers: Readable<TimerState[]>;
  /** The video the transport controls act on (`null` = none live). */
  transport: Readable<MediaTransport | null>;
  /** The audio bus without its meters (those come through `Connection.onAudioLevels`). */
  audio: Readable<Omit<AudioState, "levels"> | null>;
  /** What is on the audio layer (position refreshed up to 4 times a second while playing). */
  audioLayer: Readable<MediaLayerState | null>;
  /** Every stage screen with the layout it shows. */
  stageScreens: Readable<StageScreenState[]>;
  /** The message shown to the performers (`null` = none). */
  stageMessage: Readable<StageMessageState | null>;
  /** The box-wide clock format (`setClockFormat`). */
  clockFormat: Readable<ClockFormatState>;
  /** The box's attached displays (refreshed on hot-plug). */
  displays: Readable<DisplayInfo[]>;
  /** The live slide's object build progress (plan G18; `null`: none). */
  build: Readable<BuildProgress | null>;
  /** Live captions: on or off, the engine's status, the lines on screen (`null`: a server
   * without captions). */
  captions: Readable<CaptionsState | null>;
}

export const LIVE_KEY = Symbol("live");

export function createLive(conn: Connection): Live {
  const s = conn.show;
  return {
    slide: select(s, (st) => st?.layers.slide ?? null, jsonEqual),
    announcement: select(s, (st) => st?.layers.announcements ?? null, jsonEqual),
    latest: latestStore(s),
    cursor: select(s, (st) => st?.cursor ?? null, jsonEqual),
    liveSlideId: select(s, (st) => st?.layers.slide?.slide ?? null),
    cursorSlideId: select(s, (st) => st?.cursor?.slide ?? null),
    livePlaylistItem: select(s, (st) => st?.layers.slide?.playlist?.item ?? null),
    activity: select(s, layerActivity, jsonEqual),
    restorable: select(s, (st) => st?.restorableLayers ?? [], jsonEqual),
    activeLook: select(s, (st) => st?.activeLook ?? null),
    // Signage channels' screens are no operator screens (Site ▸ Signage shows them).
    screens: select(s, (st) => (st?.screens ?? []).filter((x) => !x.signage), jsonEqual),
    diagnostics: select(s, (st) => st?.diagnostics ?? null, jsonEqual),
    screensEnabled: select(s, (st) => st?.screensEnabled ?? { audience: true, stage: true }, jsonEqual),
    identify: select(s, (st) => st?.identify ?? false),
    options: select(s, (st) => st?.options ?? { continuousPlaylist: true }, jsonEqual),
    layers: select(s, (st) => st?.layers ?? null, jsonEqual),
    timers: select(s, (st) => st?.timers ?? [], jsonEqual),
    transport: select(s, (st) => st?.transport ?? null, jsonEqual),
    audio: select(s, (st) => (st?.audio ? withoutLevels(st.audio) : null), jsonEqual),
    audioLayer: select(s, (st) => st?.layers.audio ?? null, jsonEqual),
    stageScreens: select(s, (st) => st?.stageScreens ?? [], jsonEqual),
    stageMessage: select(s, (st) => st?.stageMessage ?? null, jsonEqual),
    clockFormat: select(s, (st) => st?.clockFormat ?? { use24Hour: false, seconds: false }, jsonEqual),
    displays: select(s, (st) => st?.displays ?? [], jsonEqual),
    build: select(s, (st) => st?.build ?? null, jsonEqual),
    captions: select(s, (st) => st?.captions ?? null, jsonEqual),
  };
}

/**
 * Which of the two slide layers was triggered last. A presentation whose destination is
 * announcements goes to the announcements layer and leaves the slide layer as it was, so the
 * slide the operator just triggered (and `next` continues from) is whichever layer changed
 * last. Feed it every state with `update`; `current` is that slide.
 */
export class LatestLive {
  private slide: SlidePosition | null = null;
  private ann: SlidePosition | null = null;
  private annLast = false;
  private first = true;

  update(st: { layers: { slide: SlidePosition | null; announcements: SlidePosition | null }; cursor: SlidePosition | null } | null): SlidePosition | null {
    const slide = st?.layers.slide ?? null;
    const ann = st?.layers.announcements ?? null;
    if (this.first) {
      this.first = false;
      // Nothing seen change yet: the one `next` continues from.
      this.annLast = !!ann && (!slide || st?.cursor?.presentation === ann.presentation);
    } else if (ann && !jsonEqual(ann, this.ann)) this.annLast = true;
    else if (slide && !jsonEqual(slide, this.slide)) this.annLast = false;
    if (!ann) this.annLast = false;
    else if (!slide) this.annLast = true;
    this.slide = slide;
    this.ann = ann;
    return this.current;
  }

  get current(): SlidePosition | null {
    return this.annLast ? this.ann : this.slide;
  }
}

function latestStore(s: Connection["show"]): Readable<SlidePosition | null> {
  return {
    subscribe(run) {
      const l = new LatestLive();
      let first = true;
      let last: SlidePosition | null = null;
      return s.subscribe((v) => {
        const next = l.update(v?.state ?? null);
        if (first || !jsonEqual(last, next)) {
          first = false;
          last = next;
          run(next);
        }
      });
    },
  };
}

function withoutLevels(a: AudioState): Omit<AudioState, "levels"> {
  const { levels: _levels, ...rest } = a;
  return rest;
}

export function useLive(): Live {
  return getContext<Live>(LIVE_KEY);
}
