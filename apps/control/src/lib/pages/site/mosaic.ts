// Site ▸ Displays (Vesper Mosaic, docs/MOSAIC-PLAN.md): the pure logic, unit tested. Every
// screen of the site's display boxes as one grid, what each shows in words, the feed picker's
// choices, and the machines where a Vesper server and the display service would both drive the
// screens. The wall designer's logic (round M2) is at the end.

import { signageLabel } from "./signage";
import type { MosaicBox, MosaicChannel, MosaicFeed, MosaicScreen, MosaicScreenSettings, MosaicScreenStatus, MosaicState, MosaicWall, MosaicWallCell, PutMosaicWall } from "$protocol";
import type { Status } from "../../ui/types";

/** One tile of the grid: a screen of a box. */
export interface DisplayTile {
  box: MosaicBox;
  screen: MosaicScreen;
  /** Unique key (`machine/number`). */
  key: string;
}

/** Every screen of the display boxes (not the Vesper machines'), by box then number. */
export function displayTiles(state: MosaicState | null): DisplayTile[] {
  if (!state) return [];
  return state.boxes
    .filter((b) => !b.vesper)
    .flatMap((b) => b.screens.map((s) => ({ box: b, screen: s, key: `${b.machine}/${s.number}` })));
}

/** Boxes with no screen to show (not reached yet, address unknown): listed apart. */
export function boxesWithoutScreens(state: MosaicState | null): MosaicBox[] {
  return state ? state.boxes.filter((b) => !b.vesper && b.screens.length === 0) : [];
}

/** A screen's settings as the detail panel edits them: never its wall tile, which is the
 * wall's to set (H08-1: a tile sent back after the wall went would crop the screen again). */
export function editableSettings(s: MosaicScreenSettings): MosaicScreenSettings {
  const { tile: _tile, ...rest } = s;
  return { ...rest };
}

/** Two screens' settings are the same (the wall tile aside). */
export function sameSettings(a: MosaicScreenSettings, b: MosaicScreenSettings): boolean {
  const ea = editableSettings(a) as unknown as Record<string, unknown>;
  const eb = editableSettings(b) as unknown as Record<string, unknown>;
  const keys = new Set([...Object.keys(ea), ...Object.keys(eb)]);
  return [...keys].every((k) => JSON.stringify(ea[k]) === JSON.stringify(eb[k]));
}

/** What Save settings sends (H08-1): the screen's settings as the server has them now, with
 * only the fields the operator changed since the draft was copied (`base`), never a tile. A
 * change another admin made meanwhile to another field stays. */
export function settingsToSave(base: MosaicScreenSettings, draft: MosaicScreenSettings, current: MosaicScreenSettings): MosaicScreenSettings {
  const b = editableSettings(base) as unknown as Record<string, unknown>;
  const d = editableSettings(draft) as unknown as Record<string, unknown>;
  const out = editableSettings(current) as unknown as Record<string, unknown>;
  for (const k of new Set([...Object.keys(b), ...Object.keys(d)])) {
    if (JSON.stringify(b[k]) === JSON.stringify(d[k])) continue;
    if (d[k] === undefined) delete out[k];
    else out[k] = d[k];
  }
  const s = out as unknown as MosaicScreenSettings;
  return { ...s, rotation: Number(s.rotation) };
}

/** The machines where a Vesper server and the display service both run. */
export function conflicts(state: MosaicState | null): MosaicBox[] {
  return state ? state.boxes.filter((b) => b.vesper) : [];
}

/** A screen's name for people: "Lobby Display · 2 HDMI-1". */
export function screenLabel(t: Pick<DisplayTile, "box" | "screen">): string {
  return `${t.box.name} · ${t.screen.number}${t.screen.name ? ` ${t.screen.name}` : ""}`;
}

/** What a feed is, in words. */
export function feedLabel(feed: MosaicFeed, channels: readonly MosaicChannel[]): string {
  switch (feed.kind) {
    case "none":
      return "Nothing (desktop)";
    case "source":
      return feed.source;
    case "channel": {
      const c = channels.find((x) => x.id === feed.channel);
      return c ? `Channel ${c.name}${c.source ? ` · ${c.source}` : " · no source"}` : "A deleted channel";
    }
  }
}

const STATUS: Record<MosaicScreenStatus, { label: string; status: Status }> = {
  showing: { label: "Showing", status: "ok" },
  connecting: { label: "Connecting", status: "info" },
  lost: { label: "Feed lost · last picture", status: "warn" },
  fallback: { label: "Fallback", status: "warn" },
  idle: { label: "Idle", status: "off" },
  offline: { label: "Offline", status: "error" },
  error: { label: "Error", status: "error" },
};

/** A screen's state in words and colour (a test pattern up says so). */
export function screenStatus(s: Pick<MosaicScreen, "status" | "testPattern" | "problem">): { label: string; status: Status; title?: string } {
  const base = STATUS[s.status] ?? { label: s.status, status: "off" as Status };
  const label = s.testPattern ? `${base.label} · test pattern` : base.label;
  return { label, status: s.testPattern && base.status === "ok" ? "info" : base.status, title: s.problem ?? undefined };
}

/** The feed picker's value for a feed: `none`, `s:<source>`, `c:<channel id>`. */
export function feedValue(feed: MosaicFeed): string {
  return feed.kind === "none" ? "none" : feed.kind === "source" ? `s:${feed.source}` : `c:${feed.channel}`;
}

/** The feed a picker value names. */
export function parseFeed(v: string): MosaicFeed {
  if (v.startsWith("s:")) return { kind: "source", source: v.slice(2) };
  if (v.startsWith("c:")) return { kind: "channel", channel: v.slice(2) };
  return { kind: "none" };
}

/** The feed picker's choices: nothing, every channel, every source on the network (and the
 * current source when it isn't listed now); a signage channel's source reads "Signage: Lobby". */
export function feedChoices(sources: readonly string[], channels: readonly MosaicChannel[], current?: MosaicFeed): { value: string; label: string }[] {
  const out = [{ value: "none", label: "Nothing (desktop)" }];
  for (const c of channels) out.push({ value: `c:${c.id}`, label: `Channel: ${c.name}` });
  const names = [...sources];
  if (current?.kind === "source" && !names.includes(current.source)) names.unshift(current.source);
  for (const s of names) out.push({ value: `s:${s}`, label: signageLabel(s) ?? s });
  return out;
}

/** The sources a channel can point at (OMT sources are the server's own to receive, not a
 * display box's: IDN's display service shows IDN sources). */
export function displaySources(names: readonly string[]): string[] {
  return names.filter((n) => !n.startsWith("omt:"));
}

/** A one-line summary of a box's screens for Site ▸ Devices: "2 screens · Main, Idle". */
export function boxSummary(b: MosaicBox, channels: readonly MosaicChannel[]): string {
  if (b.vesper) return "Conflict: a Vesper server and the display service";
  if (!b.reachable) return b.problem ? `Displays: ${b.problem}` : "Displays: not reached yet";
  const n = b.screens.length;
  const what = b.screens.map((s) => (s.feed.kind === "none" ? "idle" : feedLabel(s.feed, channels).replace(/ · .*$/, ""))).join(", ");
  return `${n} screen${n === 1 ? "" : "s"}${n ? ` · ${what}` : ""}`;
}

/** One of this server's own displays, read-only here: what drives it (Vesper's outputs). */
export interface OwnDisplay {
  id: string;
  name: string;
  connected: boolean;
  /** "Stage output → Stage screen" for each output on it. */
  drivenBy: string[];
}

/** This server's own displays with the outputs (and their screens) that drive them. */
export function ownDisplays(
  displays: readonly { id: string; name: string; connected: boolean; boundTo: readonly string[] }[],
  outputs: readonly { id: string; name?: string; screen: string }[],
  screens: readonly { id: string; name?: string }[],
): OwnDisplay[] {
  return displays.map((d) => ({
    id: d.id,
    name: d.name && d.name !== d.id ? `${d.name} (${d.id})` : d.id,
    connected: d.connected,
    drivenBy: d.boundTo.map((oid) => {
      const o = outputs.find((x) => x.id === oid);
      const s = o && screens.find((x) => x.id === o.screen);
      return o ? `${o.name || "Output"}${s?.name ? ` → ${s.name}` : ""}` : "An output";
    }),
  }));
}

// ---- Video walls (round M2): the wall designer's pure logic. ----

/** The drag type of a display screen (`machine/number`): from the Displays grid or the
 * designer's list into a wall's cell. */
export const SCREEN_DRAG = "application/x-vesper-mosaic-screen";

/** A wall being designed (`PutMosaicWall` with its cells always listed). */
export type WallDraft = Omit<PutMosaicWall, "cells" | "feed"> & { cells: MosaicWallCell[]; feed: MosaicFeed };

/** A new wall: 1 × 2, no bezels, nothing shown. */
export function emptyWall(name = ""): WallDraft {
  return { name, rows: 1, columns: 2, bezelMm: [0, 0], feed: { kind: "none" }, cells: [] };
}

/** A wall as a draft to edit. */
export function draftOf(w: MosaicWall): WallDraft {
  return {
    name: w.name,
    rows: w.rows,
    columns: w.columns,
    bezelMm: [...w.bezelMm] as [number, number],
    ...(w.screenMm ? { screenMm: [...w.screenMm] as [number, number] } : {}),
    feed: w.feed,
    cells: w.cells.map((c) => ({ ...c })),
  };
}

/** What Save wall sends (H08-2): the draft, but while the designer's own feed picker wasn't
 * changed (`feedChanged`), the feed the wall shows now: a feed changed meanwhile (the card's
 * Shows, another Oriel) isn't reverted by a change of geometry or cells. */
export function wallBody(d: WallDraft, live: Pick<MosaicWall, "feed"> | undefined, feedChanged: boolean): WallDraft {
  const body: WallDraft = { ...d, name: d.name.trim(), feed: !feedChanged && live ? live.feed : d.feed };
  if (!body.screenMm || !(body.screenMm[0] > 0 && body.screenMm[1] > 0)) delete body.screenMm;
  return body;
}

/** A cell's place in the wall's picture, as fractions of the whole (0 to 1). */
export interface CellBox {
  row: number;
  column: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The wall's picture: its aspect (width / height, bezels included) and each cell's box. A
 * screen without a size is taken as 16:9; bezels then count as millimetres of a 1000 mm wide
 * screen. */
export function wallLayout(w: Pick<PutMosaicWall, "rows" | "columns" | "bezelMm" | "screenMm">): { aspect: number; cells: CellBox[] } {
  const rows = Math.max(1, w.rows);
  const cols = Math.max(1, w.columns);
  const [sw, sh] = w.screenMm && w.screenMm[0] > 0 && w.screenMm[1] > 0 ? w.screenMm : [1000, 562.5];
  const bh = Math.max(0, w.bezelMm[0] || 0);
  const bv = Math.max(0, w.bezelMm[1] || 0);
  const W = cols * sw + (cols - 1) * bh;
  const H = rows * sh + (rows - 1) * bv;
  const cells: CellBox[] = [];
  for (let r = 0; r < rows; r++)
    for (let c = 0; c < cols; c++) cells.push({ row: r, column: c, x: (c * (sw + bh)) / W, y: (r * (sh + bv)) / H, w: sw / W, h: sh / H });
  return { aspect: W / H, cells };
}

/** Puts a screen in a cell (`null`: empties it). A screen already elsewhere in the wall moves;
 * whoever was in the cell leaves. */
export function placeScreen(cells: readonly MosaicWallCell[], row: number, column: number, screen: { machine: string; screen: number } | null): MosaicWallCell[] {
  const out = cells.filter((c) => !(c.row === row && c.column === column) && !(screen && c.machine === screen.machine && c.screen === screen.screen));
  if (screen) out.push({ row, column, machine: screen.machine, screen: screen.screen });
  return out.sort((a, b) => a.row - b.row || a.column - b.column);
}

/** The draft with another size: cells outside it leave. A size that isn't a number (an
 * emptied field) keeps the current one (H08-3). */
export function resizeWall(d: WallDraft, rows: number, columns: number): WallDraft {
  const size = (v: number, cur: number) => (Number.isFinite(v) ? Math.min(16, Math.max(1, Math.round(v))) : cur);
  const r = size(rows, d.rows);
  const c = size(columns, d.columns);
  return { ...d, rows: r, columns: c, cells: d.cells.filter((x) => x.row < r && x.column < c) };
}

/** What's wrong with a draft before it is sent (the server checks again), or null. */
export function wallProblem(d: WallDraft, walls: readonly MosaicWall[], editing: string | null): string | null {
  const name = d.name.trim();
  if (!name) return "Give the wall a name";
  if ([...name].length > 48) return "The name is at most 48 characters";
  if (walls.some((w) => w.id !== editing && w.name.toLowerCase() === name.toLowerCase())) return "A wall has that name";
  if (d.rows < 1 || d.rows > 16 || d.columns < 1 || d.columns > 16) return "1 to 16 rows and columns";
  if (d.bezelMm.some((b) => !(b >= 0 && b <= 10000))) return "Bezels: 0 to 10000 mm";
  const hasSize = !!d.screenMm && d.screenMm[0] > 0 && d.screenMm[1] > 0;
  if ((d.bezelMm[0] > 0 || d.bezelMm[1] > 0) && !hasSize) return "With bezels, give the screens' visible size";
  return null;
}

/** The screens a wall may use: every display screen not in another wall. */
export function freeScreens(tiles: readonly DisplayTile[], editing: string | null): DisplayTile[] {
  return tiles.filter((t) => !t.screen.wall || t.screen.wall === editing);
}

/** A wall cell's screen as a key (`machine/number`), or "" for none. */
export function cellKey(cells: readonly MosaicWallCell[], row: number, column: number): string {
  const c = cells.find((x) => x.row === row && x.column === column);
  return c ? `${c.machine}/${c.screen}` : "";
}

/** A wall cell's screen that no box lists now (its box was removed, or doesn't list it). */
export function unlistedScreenLabel(screen: number): string {
  return `Screen ${screen} (not listed now)`;
}

/** A designer cell's picker: Empty, then every free screen ("(moves here)" when it is in
 * another cell). The cell's own screen stays listed and selected even when no box lists it now
 * (R1-5), so Save keeps it: the server doesn't check a wall's screens against their boxes again. */
export function cellChoices(cells: readonly MosaicWallCell[], row: number, column: number, free: readonly DisplayTile[]): { value: string; label: string }[] {
  const cur = cellKey(cells, row, column);
  const placed = new Set(cells.map((c) => `${c.machine}/${c.screen}`));
  const out = [{ value: "", label: "Empty" }];
  const p = cur ? parseKey(cur) : null;
  if (p && !free.some((t) => t.key === cur)) out.push({ value: cur, label: unlistedScreenLabel(p.screen) });
  for (const t of free) out.push({ value: t.key, label: screenLabel(t) + (placed.has(t.key) && t.key !== cur ? " (moves here)" : "") });
  return out;
}

/** The screen a key names. */
export function parseKey(key: string): { machine: string; screen: number } | null {
  const i = key.lastIndexOf("/");
  if (i <= 0) return null;
  const n = Number(key.slice(i + 1));
  return Number.isInteger(n) && n >= 1 ? { machine: key.slice(0, i), screen: n } : null;
}

/** A wall's state in words and colour: its tiles' worst, and whether they are in step. */
export function wallStatus(w: Pick<MosaicWall, "status" | "synced" | "cells" | "delayMs" | "lateRecently">): { label: string; status: Status } {
  if (!w.cells.length) return { label: "No screens yet", status: "off" };
  const base = STATUS[w.status] ?? { label: w.status, status: "off" as Status };
  if (w.status !== "showing") return base;
  if (!w.synced) return { label: w.lateRecently ? "Showing · out of step" : "Showing · not in sync", status: "warn" };
  return { label: `In step${w.delayMs != null ? ` · ${Math.round(w.delayMs)} ms` : ""}`, status: "ok" };
}

/** The source a feed shows now (a channel resolved; "" none). */
export function feedSource(feed: MosaicFeed, channels: readonly MosaicChannel[]): string {
  if (feed.kind === "source") return feed.source;
  if (feed.kind === "channel") return channels.find((c) => c.id === feed.channel)?.source ?? "";
  return "";
}

/** "Row 1, column 2". */
export function cellName(row: number, column: number): string {
  return `Row ${row + 1}, column ${column + 1}`;
}

/** Where a box's address came from, in words (M3-2): an admin's overrides the site's. */
export function addressNote(b: Pick<MosaicBox, "address" | "addressFrom" | "name" | "reachable">): string {
  const from =
    b.addressFrom === "admin"
      ? "set here (it overrides where the site sees the box)"
      : b.addressFrom === "site"
        ? "where the site sees its IDN Node"
        : b.addressFrom === "join"
          ? "where it asked to join from"
          : "not known yet (its IDN Node isn't online in the site)";
  return b.reachable ? `Address ${from}` : `${b.name} isn't answering · address ${from}`;
}

/** A box's display service in words (M3-5): its state and its mode, as its Node reports it or
 * as Vesper last set it. */
export function serviceNote(b: Pick<MosaicBox, "service" | "serviceMode" | "serviceModeReported">): string {
  const state = b.service ? `Display service ${b.service}` : "Display service not reported";
  const how = b.serviceModeReported ? "" : " (as set here)";
  const mode = b.serviceMode === "headless" ? ` · headless${how}` : b.serviceMode === "desktop" ? ` · in the desktop session${how}` : b.serviceMode === "off" ? ` · off${how}` : "";
  return state + mode;
}

/** Who updates a machine's IDN Node, when another product does it instead of the Site Manager
 *  (IDN Beta14: `Vesper` for the vesper-idn-node package); `null`: the Site Manager. */
export function updatesNote(b: { updateManagedBy?: string | null }): string | null {
  return b.updateManagedBy ? `Updates managed by ${b.updateManagedBy}` : null;
}

/** How many boxes have the logo on their screens, in words. */
export function logoNote(state: MosaicState | null): string {
  const boxes = (state?.boxes ?? []).filter((b) => !b.vesper);
  if (!state?.logo) return "No logo: boxes show their own, or black.";
  const on = boxes.filter((b) => b.logoState === "onBox").length;
  const sending = boxes.filter((b) => b.logoState === "sending").length;
  const failed = boxes.filter((b) => b.logoState === "failed").length;
  const more = [sending ? `${sending} sending` : "", failed ? `${failed} failed` : ""].filter(Boolean).join(", ");
  return `${state.logo.width} × ${state.logo.height} · on ${on} of ${boxes.length} display box${boxes.length === 1 ? "" : "es"}${more ? ` (${more})` : ""}`;
}

/** A box's logo state in words. */
export function boxLogoNote(b: Pick<MosaicBox, "logoState">): string {
  return b.logoState === "onBox" ? "On the box" : b.logoState === "sending" ? "Sending" : b.logoState === "failed" ? "Failed" : "Not sent yet";
}
