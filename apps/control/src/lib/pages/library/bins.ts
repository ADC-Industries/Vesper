// Media bins, bin playlists and global groups in the Library (G17): the pure parts (the
// tree's rows, where "move up / down" puts a bin or group, the items and ops sent). Unit tested.

import type { GlobalGroup, MediaBinItem, MediaBinNode, MediaBinOp, MediaKind, PlaylistItem, PlaylistOp, PlaylistTreeNode } from "$protocol";
import { uuid } from "../../edit/ids";

/** One media bin of the tree, flattened in display order. */
export interface BinRow {
  id: string;
  name: string;
  /** Its folder path, outermost first. */
  path: string[];
  kind: "manual" | "watched";
  itemCount: number;
}

/** A bin row's drop target (`bin:<id>`): manual bins only, as a watched folder's items follow
 * the folder and the server refuses adds to it (D09-4). */
export function binDropTarget(r: BinRow): string | undefined {
  return r.kind === "manual" ? `bin:${r.id}` : undefined;
}

/** The media (not audio) bins of a bin tree, in display order (each folder's sub-folders first,
 * then its bins, as the server sends them). */
export function mediaBinRows(tree: MediaBinNode[]): BinRow[] {
  const out: BinRow[] = [];
  const walk = (nodes: MediaBinNode[]) => {
    for (const n of nodes) {
      if (n.kind === "folder") walk(n.children);
      else if (n.id && n.browser !== "audio") out.push({ id: n.id, name: n.name, path: n.path, kind: n.kind === "watched" ? "watched" : "manual", itemCount: n.itemCount });
    }
  };
  walk(tree);
  return out;
}

/** A row's label in the tree: the bin's name after its folders ("Visuals › Backgrounds"). */
export function binLabel(r: Pick<BinRow, "name" | "path">): string {
  return [...r.path, r.name].join(" › ");
}

/** Where "move up" (-1) or "down" (+1) puts a bin: after which bin of its folder (`null` =
 * first). `null` when it is already at that end. */
export function binMove(rows: BinRow[], id: string, dir: -1 | 1): { after: string | null } | null {
  const me = rows.find((r) => r.id === id);
  if (!me) return null;
  const sibs = rows.filter((r) => r.path.join("\u0000") === me.path.join("\u0000"));
  return listMove(
    sibs.map((r) => r.id),
    id,
    dir,
  );
}

/** Where "move up / down" puts a global group (the groups in their order). */
export function groupMove(groups: Pick<GlobalGroup, "id">[], id: string, dir: -1 | 1): { after: string | null } | null {
  return listMove(
    groups.map((g) => g.id),
    id,
    dir,
  );
}

/** `after` for moving `id` one place in `ids` (display order). */
export function listMove(ids: string[], id: string, dir: -1 | 1): { after: string | null } | null {
  const i = ids.indexOf(id);
  if (i < 0) return null;
  const j = i + dir;
  if (j < 0 || j >= ids.length) return null;
  const rest = ids.filter((x) => x !== id);
  // In the list without it, it goes to index j.
  return { after: j === 0 ? null : rest[j - 1]! };
}

/** The layer a file plays on when it is triggered from a bin or playlist. */
export function layerFor(kind: MediaKind | undefined): "audio" | "background" {
  return kind === "audio" ? "audio" : "background";
}

/** A new bin item for a file (default trigger settings). */
export function newBinItem(asset: string, kind: MediaKind | undefined, id: string = uuid()): MediaBinItem {
  return { id, trigger: { asset, layer: layerFor(kind) } };
}

/** The ops adding files at the end of a bin (`last`: its last item now), skipping files it
 * already holds. */
export function addToBinOps(
  existing: { id: string; asset: string }[],
  files: { asset: string; kind?: MediaKind }[],
  makeId: () => string = uuid,
): MediaBinOp[] {
  const have = new Set(existing.map((i) => i.asset));
  let after: string | null = existing.length ? existing[existing.length - 1]!.id : null;
  const ops: MediaBinOp[] = [];
  for (const f of files) {
    if (have.has(f.asset)) continue;
    have.add(f.asset);
    const item = newBinItem(f.asset, f.kind, makeId());
    ops.push(after ? { type: "addItem", after, item } : { type: "addItem", item });
    after = item.id;
  }
  return ops;
}

/** The ops adding files at the end of a media playlist (`existing`: its items in order). */
export function addToPlaylistOps(
  existing: { id: string }[],
  files: { asset: string; kind?: MediaKind }[],
  makeId: () => string = uuid,
): PlaylistOp[] {
  let after: string | null = existing.length ? existing[existing.length - 1]!.id : null;
  return files.map((f) => {
    const item: PlaylistItem = { id: makeId(), kind: { type: "media", asset: f.asset, layer: layerFor(f.kind) } };
    const op: PlaylistOp = after ? { type: "addItem", after, item } : { type: "addItem", item };
    after = item.id;
    return op;
  });
}

/** A media playlist of the playlist tree ("bin playlists"). */
export interface MediaPlaylistRow {
  id: string;
  name: string;
  path: string[];
  itemCount: number;
}

/** The playlists with `purpose: media`, in tree order, with their folder path. */
export function mediaPlaylists(tree: PlaylistTreeNode[], path: string[] = []): MediaPlaylistRow[] {
  const out: MediaPlaylistRow[] = [];
  for (const n of tree) {
    if (n.kind === "playlist" && n.purpose === "media") out.push({ id: n.id, name: n.name, path, itemCount: n.itemCount });
    if (n.children?.length) out.push(...mediaPlaylists(n.children, [...path, n.name]));
  }
  return out;
}

/** A bin or playlist name the server takes: trimmed, 1–200 characters, no control characters. */
export function nameOk(name: string): boolean {
  const t = name.trim();
  // eslint-disable-next-line no-control-regex
  return t.length > 0 && Array.from(t).length <= 200 && !/[\u0000-\u001f\u007f-\u009f]/.test(t);
}

/** A colour from `#rrggbb` (the colour input's value). */
export function colorFromHex(hex: string): { r: number; g: number; b: number } | null {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  if (!m) return null;
  return { r: parseInt(m[1]!, 16) / 255, g: parseInt(m[2]!, 16) / 255, b: parseInt(m[3]!, 16) / 255 };
}

/** `#rrggbb` of a colour (for the colour input). */
export function hexOf(c: { r: number; g: number; b: number } | null | undefined): string {
  if (!c) return "#808080";
  const h = (v: number) =>
    Math.round(Math.min(1, Math.max(0, v)) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${h(c.r)}${h(c.g)}${h(c.b)}`;
}
