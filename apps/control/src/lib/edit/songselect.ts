// SongSelect files and pasted SongSelect lyrics (Import ▸ Song from SongSelect…, Reflow's
// smart paste). The server reads them (`POST /songselect/parse`, model::songselect): lyrics
// .txt, SongShow Plus .usr / .bin, ChordPro. Vesper never contacts CCLI: the user downloads the
// files from SongSelect and brings them here. Pure functions, unit tested.

import type { Presentation, PresentationOp, SongSelectParsed } from "$protocol";

/** The file types the dialog offers (the server sniffs the content first). */
export const SONGSELECT_ACCEPT = ".txt,.usr,.bin,.cho,.chordpro,.chopro,.pro,text/plain";

/** Largest file read (the server refuses more: model::songselect::MAX_FILE_BYTES). */
export const SONGSELECT_MAX_BYTES = 2 << 20;

/** Pasted text that looks like SongSelect lyrics: a `CCLI Song #` line or the terms line. */
export function looksLikeSongSelect(text: string): boolean {
  return /^\s*CCLI Song\s*(?:#|No\.?|Number)?\s*:?\s*\d+\s*$/im.test(text) || /^\s*For use solely with the SongSelect/im.test(text);
}

/** Bytes as base64 (the parse request's `data`). */
export function toBase64(bytes: Uint8Array): string {
  let s = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) s += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  return btoa(s);
}

/** What to do with a song whose CCLI number is already in the library. */
export type DuplicateAction = "skip" | "replace" | "keep";

/** One song in the dialog's list: a file (or the pasted text) and what the server read. */
export interface SongSelectItem {
  key: string;
  /** The file's name ("Pasted text" for a paste). */
  source: string;
  parsed: SongSelectParsed | null;
  error: string | null;
  /** For a duplicate: skip (the default), replace the library's song, or keep both. */
  action: DuplicateAction;
}

/** The new item for a parse result: duplicates are skipped unless the user says otherwise. */
export function itemFor(key: string, source: string, parsed: SongSelectParsed | null, error: string | null): SongSelectItem {
  return { key, source, parsed, error, action: parsed?.duplicates?.length ? "skip" : "keep" };
}

/** The sections' names in order ("Verse 1, Chorus, Verse 2"). */
export function sectionNames(p: Presentation): string {
  return (p.sections ?? []).map((s) => s.name).join(", ");
}

/** What Import does with each item (in list order): create, replace (the duplicate's ID) or skip. */
export function importPlan(items: SongSelectItem[]): { item: SongSelectItem; kind: "create" | "replace" | "skip"; target?: string }[] {
  return items
    .filter((i) => i.parsed)
    .map((item) => {
      const dup = item.parsed!.duplicates?.[0];
      if (!dup) return { item, kind: "create" as const };
      if (item.action === "skip") return { item, kind: "skip" as const };
      if (item.action === "replace") return { item, kind: "replace" as const, target: dup.id };
      return { item, kind: "create" as const };
    });
}

/** "Import 3 songs" / "Import 1 song" / "Import" (skipped and failed ones not counted). */
export function importLabel(items: SongSelectItem[]): string {
  const n = importPlan(items).filter((p) => p.kind !== "skip").length;
  return n === 0 ? "Import" : n === 1 ? "Import 1 song" : `Import ${n} songs`;
}

/**
 * Replacing a library song with a newly read one, keeping its ID (playlists and history still
 * point to it), name and library: every old section goes, the new ones come in order, the new
 * arrangements and song metadata replace the old.
 */
export function replaceOps(existing: Presentation, next: Presentation): PresentationOp[] {
  const ops: PresentationOp[] = (existing.sections ?? []).map((s) => ({ type: "deleteSection", section: s.id }));
  let after: string | undefined;
  for (const s of next.sections ?? []) {
    ops.push({ type: "addSection", ...(after ? { after } : {}), section: s });
    after = s.id;
  }
  ops.push({ type: "setArrangements", arrangements: next.arrangements ?? [], selected: next.selectedArrangement ?? null });
  ops.push({ type: "setPresentation", songInfo: next.songInfo ?? null });
  return ops;
}

/** The notice after an import: what was made, replaced and skipped. */
export function importSummary(made: number, replaced: number, skipped: number, failed: number): string {
  const n = (k: number, one: string, many = `${one}s`) => `${k} ${k === 1 ? one : many}`;
  const parts = [made ? `${n(made, "song")} imported` : "", replaced ? `${n(replaced, "song")} replaced` : "", skipped ? `${n(skipped, "duplicate")} skipped` : "", failed ? `${failed} failed` : ""].filter(Boolean);
  return parts.length ? `SongSelect: ${parts.join(", ")}.` : "SongSelect: nothing imported.";
}
