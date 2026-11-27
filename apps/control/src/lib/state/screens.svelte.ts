// The screens' state (Svelte 5 runes): the box's screen configuration (`GET /screens`), the
// screen the right column previews, and the Screens page's unsaved-changes guard. The pure
// screen-configuration logic is in `screens.ts` (unit tested).

import type { Screen } from "$protocol";
import { operatorScreens } from "./screens";

/** What the Screens tab offers the page toolbar. */
export interface ScreensEditor {
  readonly dirty: boolean;
  readonly busy: boolean;
  apply(): Promise<boolean>;
  revert(): void;
}

class ScreensState {
  /** The screen configuration documents (`GET /screens`). Live status is in the show state. */
  screens = $state.raw<Screen[]>([]);
  /** The screen the right column previews. */
  previewScreen = $state<string | null>(null);
  /**
   * Asked before the Screens page closes (Done, Esc, another page): returns `false` to keep
   * it open (it then asks about unsaved changes itself).
   */
  guard: (() => boolean) | null = null;
  /** Esc on the Screens page: closes its own prompt or menu; `true` when it closed one. */
  escape: (() => boolean) | null = null;
  /** The Screens tab's staged changes, for the page toolbar's Apply / Revert (set while the
   * Screens tab is mounted). */
  editor = $state.raw<ScreensEditor | null>(null);
  /** The Screens & Looks page's tab. */
  tab = $state<"screens" | "looks">("screens");

  setScreens(all: Screen[]) {
    // Signage channels' screens are never operator screens (previewed, configured), H02-7.
    const list = operatorScreens(all);
    this.screens = list;
    if (!this.previewScreen || !list.some((s) => s.id === this.previewScreen)) {
      const audience = list.find((s) => s.kind?.type === "audience") ?? list[0];
      this.previewScreen = audience?.id ?? null;
    }
  }

  /** True when the Screens page may close now. */
  mayLeave(): boolean {
    return !this.guard || this.guard();
  }
}

export const screensState = new ScreensState();
