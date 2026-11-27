// What the Show page's sections do: trigger and select slides, disable them, media cues, the
// slide menu, a song item's arrangement and moving playlists in the tree. Edits go through the
// session's editor (undo), and the changed document is refetched for the sections at once.

import type { PlaylistItemRef } from "$protocol";
import type { GridSlide } from "../../model";
import type { DragPayload, DropHandlers } from "../../dnd.svelte";
import type { MenuItem } from "../../shell/menu";
import { uuid } from "../../edit/ids";
import type { UndoEntry } from "../../edit/undo";
import { session } from "../../state/session.svelte";
import { shell } from "../../state/shell.svelte";
import { showView } from "../../state/show.svelte";
import { presCache } from "./presCache.svelte";
import type { MediaCue, ShowSection } from "./sections";

type PresSection = Extract<ShowSection, { type: "presentation" }>;

/** The playlist item a section's triggers carry (none for a library section). */
export function itemRef(sec: PresSection): PlaylistItemRef | null {
  const pl = session.playlist;
  return sec.itemId && pl ? { playlist: pl.id, item: sec.itemId } : null;
}

export function slideKey(sec: ShowSection, slideId: string): string {
  return `${sec.key}|${slideId}`;
}

/** One occurrence of a slide in its section (selection, `data-key`): the slide key and its
 *  place in the play order (`-`: a slide the arrangement leaves out). */
export function occurrenceKey(sec: ShowSection, slideId: string, step: number | null | undefined): string {
  return `${slideKey(sec, slideId)}#${step ?? "-"}`;
}

/** Makes the section the active one (Edit and Reflow open its presentation). */
export function activate(sec: PresSection) {
  const ref = itemRef(sec);
  const open = session.open;
  if (open && open.presentation.id === sec.presentationId && open.playlistItem?.item === ref?.item) {
    showView.selectedItemId = ref?.item ?? null;
    return;
  }
  void session.openPresentation(sec.presentationId, ref);
}

/** A click on a slide (at `step` in the play order: that occurrence of a repeated slide):
 *  select it, and show it unless it is disabled. */
export function clickSlide(sec: PresSection, s: GridSlide, step: number | null = null) {
  showView.selectedSlide = occurrenceKey(sec, s.id, step);
  activate(sec);
  if (!s.enabled) return;
  const ref = itemRef(sec);
  void session.run({
    type: "triggerSlide",
    presentation: sec.presentationId,
    slide: s.id,
    ...(ref ? { playlistItem: ref } : {}),
    ...(step !== null ? { step } : {}),
  });
}

async function editPresentation(id: string, label: string, ops: Parameters<NonNullable<typeof session.editor>["editPresentation"]>[2]) {
  const r = await session.edit(label, (e) => e.editPresentation(id, label, ops));
  if (r === null) return;
  await presCache.refresh(id);
  if (session.open?.presentation.id === id) void session.reopen();
}

/** Right-click: disabled ↔ enabled (`setSlide {enabled}`). */
export function toggleDisabled(sec: PresSection, s: GridSlide, step: number | null = null) {
  showView.selectedSlide = occurrenceKey(sec, s.id, step);
  return editPresentation(sec.presentationId, s.enabled ? "Disable Slide" : "Enable Slide", [{ type: "setSlide", slide: s.id, enabled: !s.enabled }]);
}

/** A media bin item dropped on a slide becomes a media action (a media cue). */
export function addMediaCue(presentationId: string, slideId: string, media: { id: string; name: string }) {
  return editPresentation(presentationId, "Add Media Cue", [
    { type: "addSlideAction", slide: slideId, action: { id: uuid(), name: media.name, kind: { type: "media", asset: media.id } } },
  ]);
}

export function removeMediaCue(presentationId: string, slideId: string, cue: MediaCue) {
  return editPresentation(presentationId, "Remove Media Cue", [{ type: "removeSlideAction", slide: slideId, action: cue.action }]);
}

/** The song's arrangement for this playlist item (`null`: the presentation's own). */
export async function setArrangement(itemId: string, arrangement: string | null) {
  const pl = session.playlist;
  if (!pl) return;
  const r = await session.edit("Arrangement", (e) => e.editPlaylist(pl.id, "Arrangement", [{ type: "setItem", item: itemId, arrangement }]));
  if (r !== null) await session.selectPlaylist(pl.id, true);
}

/** The slide menu ("…", Shift+F10, the menu key). */
export function slideMenu(sec: PresSection, s: GridSlide, cues: MediaCue[], x: number, y: number, step: number | null = null) {
  showView.selectedSlide = occurrenceKey(sec, s.id, step);
  const item = (id: string, label: string, run: () => void, more: Partial<Extract<MenuItem, { type: "item" }>> = {}): MenuItem => ({ type: "item", id, label, run, ...more });
  const open = (mode: "edit" | "reflow") => {
    void session.openPresentation(sec.presentationId, itemRef(sec)).then(() => shell.setMode(mode));
  };
  const items: MenuItem[] = [
    item("show", "Show", () => clickSlide(sec, s, step), { disabled: !s.enabled, title: s.enabled ? undefined : "Enable the slide to show it" }),
    item("toggle", s.enabled ? "Disable Slide" : "Enable Slide", () => void toggleDisabled(sec, s, step), { shortcut: "Right-click" }),
  ];
  if (cues.length) {
    items.push({ type: "separator" });
    for (const c of cues) items.push(item(`cue-${c.action}`, `Remove Media Cue “${c.name}”`, () => void removeMediaCue(sec.presentationId, s.id, c)));
  }
  items.push({ type: "separator" }, item("edit", "Edit Slides", () => open("edit")), item("reflow", "Open in Reflow", () => open("reflow")));
  shell.popover = null;
  shell.contextMenu = { x, y, items };
}

/** Moves a playlist or folder in the tree (`PUT /playlists/{id}`), with undo. */
export async function moveNode(id: string, from: { parent: string | null; after: string | null }, to: { parent: string | null; after: string | null }) {
  const conn = session.conn;
  const editor = session.editor;
  if (!conn || !editor) return;
  const put = (p: { parent: string | null; after: string | null }) => conn.rest.updatePlaylist(id, { parent: p.parent, after: p.after });
  const entry = (a: typeof from, b: typeof from): UndoEntry => ({
    label: "Move",
    run: async () => {
      await put(a);
      return entry(b, a);
    },
  });
  const ok = await session.edit("Move", async () => {
    await put(to);
    return true;
  });
  if (!ok) return;
  editor.undo.push(entry(from, to));
  if (to.parent) showView.expandedFolders[to.parent] = true;
  session.playlistTree = (await conn.rest.playlists().catch(() => null)) ?? session.playlistTree;
}

/** A media bin item dropped on a slide (`<section key>|<slide id>`): its presentation is in the
 *  section key (a library section) or the playlist item. */
export function dropMediaOnSlide(target: string, p: DragPayload) {
  if (p.kind !== "media") return;
  const bar = target.indexOf("|");
  const sec = target.slice(0, bar);
  const slide = target.slice(bar + 1);
  if (bar < 0 || !slide) return;
  const presentation = sec.startsWith("lib:")
    ? sec.slice(4)
    : session.playlist?.items.find((i) => `item:${i.id}` === sec && i.kind.type === "presentation")?.presentation?.id;
  if (presentation) void addMediaCue(presentation, slide, p);
}

/** The media bin's drag handlers: its items drop on slides only. */
export const mediaDrops: DropHandlers = {
  onPlaylist: () => {},
  onList: () => {},
  onSpring: () => {},
  onSlide: dropMediaOnSlide,
};
