// Chords (batch 16): the same rules as the server's `model::chords` (keys, chord spelling,
// transposition, capo, sharps / flats, Nashville numbers; chords that follow edited words), for
// the slide editor's chord lines, the song's key control and the stage layouts' chord views.
// Pure functions, unit tested (chords.test.ts).

import type { Accidentals, ChordMark, ChordView, Paragraph, TextBox } from "$protocol";

const SHARPS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const FLATS = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

/** The keys offered by a key picker (major, then minor). */
export const KEYS: readonly string[] = [
  ...["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"],
  ...["Cm", "C#m", "Dm", "Ebm", "Em", "Fm", "F#m", "Gm", "G#m", "Am", "Bbm", "Bm"],
];

const mod12 = (n: number) => ((n % 12) + 12) % 12;

/** A note at the start of `s`: pitch class (C = 0), spelled with a flat, characters used. */
export function parseNote(s: string): { pc: number; flat: boolean; len: number } | null {
  const base = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }[s[0] ?? ""];
  if (base === undefined) return null;
  const a = s[1];
  if (a === "#" || a === "♯") return { pc: mod12(base + 1), flat: false, len: 2 };
  if (a === "b" || a === "♭") return { pc: mod12(base - 1), flat: true, len: 2 };
  return { pc: base, flat: false, len: 1 };
}

export function noteName(pc: number, flats: boolean): string {
  return (flats ? FLATS : SHARPS)[mod12(pc)] ?? "C";
}

export interface Key {
  root: number;
  minor: boolean;
  flat: boolean;
}

/** `"G"`, `"Bb"`, `"F#m"`, `"C minor"` (null when it isn't a key). */
export function parseKey(s: string | null | undefined): Key | null {
  const t = (s ?? "").trim();
  const n = parseNote(t);
  if (!n) return null;
  const rest = t.slice(n.len).trim().toLowerCase();
  const minor = ["m", "min", "minor", "-"].includes(rest);
  if (!minor && !["", "maj", "major"].includes(rest)) return null;
  return { root: n.pc, minor, flat: n.flat };
}

function usesFlats(k: Key): boolean {
  const major = k.minor ? mod12(k.root + 3) : k.root;
  return [5, 10, 3, 8, 1].includes(major) || (major === 6 && k.flat);
}

export function transposeKey(k: Key, semitones: number): Key {
  return { root: mod12(k.root + semitones), minor: k.minor, flat: k.flat && mod12(semitones) === 0 };
}

export function keyName(k: Key, acc: Accidentals = "auto"): string {
  const flats = acc === "flats" ? true : acc === "sharps" ? false : usesFlats(k);
  return noteName(k.root, flats) + (k.minor ? "m" : "");
}

const SUFFIX = /^(maj|min|dim|aug|sus|add|alt|no|M|m|\+|-|°|ø|Δ|#|b|♯|♭|\(|\)|\/|,|[0-9])*$/;

/** A chord symbol (null for `N.C.`, `(x2)`, words). */
export function parseChord(s: string): { root: number; flat: boolean; suffix: string; bass: { pc: number; flat: boolean } | null } | null {
  const t = s.trim();
  const r = parseNote(t);
  if (!r) return null;
  let rest = t.slice(r.len);
  let bass: { pc: number; flat: boolean } | null = null;
  const slash = rest.lastIndexOf("/");
  if (slash >= 0) {
    const b = parseNote(rest.slice(slash + 1));
    if (b && slash + 1 + b.len === rest.length) {
      bass = { pc: b.pc, flat: b.flat };
      rest = rest.slice(0, slash);
    }
  }
  if (!SUFFIX.test(rest)) return null;
  return { root: r.pc, flat: r.flat, suffix: rest, bass };
}

/** What a written chord shows as: moved by `shift` semitones, spelled by `acc` (and the key). */
export function showChord(
  written: string,
  shift: number,
  opts: { key?: string | null; accidentals?: Accidentals; nashville?: boolean } = {},
): string {
  const c = parseChord(written);
  if (!c) return written;
  const key = parseKey(opts.key);
  const DEG = ["1", "b2", "2", "b3", "3", "4", "#4", "5", "b6", "6", "b7", "7"];
  if (opts.nashville && key) {
    return DEG[mod12(c.root - key.root)] + c.suffix + (c.bass ? "/" + DEG[mod12(c.bass.pc - key.root)] : "");
  }
  const acc = opts.accidentals ?? "auto";
  if (mod12(shift) === 0 && acc === "auto") return written.trim();
  const shown = key ? transposeKey(key, shift) : null;
  const flats = (wasFlat: boolean) => (acc === "flats" ? true : acc === "sharps" ? false : shown ? usesFlats(shown) : wasFlat);
  return noteName(c.root + shift, flats(c.flat)) + c.suffix + (c.bass ? "/" + noteName(c.bass.pc + shift, flats(c.bass.flat)) : "");
}

/** Transposes a chord (`auto`: as written without a key). */
export function transposeChord(chord: string, semitones: number, acc: Accidentals = "auto"): string {
  return showChord(chord, semitones, { accidentals: acc });
}

/** The key a song plays in: its written key moved by its transposition ("A"; "" without one). */
export function playedKey(written: string | null | undefined, transpose: number): string {
  const k = parseKey(written);
  return k ? keyName(transposeKey(k, transpose)) : "";
}

/** Semitones from key `from` to key `to` (-5..6; null when either isn't a key). */
export function keyDistance(from: string, to: string): number | null {
  const a = parseKey(from);
  const b = parseKey(to);
  if (!a || !b) return null;
  const d = mod12(b.root - a.root);
  return d > 6 ? d - 12 : d;
}

/** A stage layout's key line (as the server draws it). */
export function keyLabel(view: ChordView | undefined, written: string | null | undefined, songTranspose: number): string {
  const v = view ?? {};
  const k = parseKey(written);
  const capo = v.capo ?? 0;
  const acc = v.accidentals ?? "auto";
  if (!k) return capo ? `Capo ${capo}` : "";
  const concert = transposeKey(k, songTranspose + (v.transpose ?? 0));
  return capo ? `Key ${keyName(concert, acc)} · Capo ${capo} (${keyName(transposeKey(concert, -capo), acc)} shapes)` : `Key ${keyName(concert, acc)}`;
}

/** A chord view in a few words ("Capo 2 · flats", "Concert pitch"). */
export function chordViewText(v: ChordView | undefined): string {
  const parts: string[] = [];
  if (v?.transpose) parts.push(`${v.transpose > 0 ? "+" : ""}${v.transpose}`);
  if (v?.capo) parts.push(`Capo ${v.capo}`);
  if (v?.accidentals === "sharps" || v?.accidentals === "flats") parts.push(v.accidentals);
  if (v?.notation === "nashville") parts.push("Numbers");
  return parts.length ? parts.join(" · ") : "Concert pitch";
}

// ---- chords on lines ----------------------------------------------------------------------------

/** Chords from `old` text moved to `new` text (see the server's `model::chords::remap_chords`). */
export function remapChords(old: string, next: string, chords: ChordMark[]): ChordMark[] {
  if (!chords.length || old === next) return chords.map((c) => ({ ...c }));
  const a = [...old];
  const b = [...next];
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  const maxSuffix = Math.min(a.length, b.length) - prefix;
  let suffix = 0;
  while (suffix < maxSuffix && a[a.length - 1 - suffix] === b[b.length - 1 - suffix]) suffix++;
  const aEnd = a.length - suffix;
  const bEnd = b.length - suffix;
  return chords.map((c) => {
    const x = Math.min(c.at ?? 0, a.length);
    const at = x < prefix ? x : x >= aEnd ? x - aEnd + bEnd : Math.min(x, bEnd > prefix ? bEnd - 1 : prefix);
    return { ...c, at: Math.min(at, b.length) };
  });
}

/**
 * Chords of `old` text on the lines `lines` it became (a line split with Enter, lines
 * joined): the lines are remapped as one text joined by `\n`, and each chord goes to the line
 * holding its character (a line's end included), so no chord is on two lines.
 */
export function splitChords(old: string, lines: string[], chords: ChordMark[]): ChordMark[][] {
  const moved = remapChords(old, lines.join("\n"), chords);
  const out: ChordMark[][] = lines.map(() => []);
  let start = 0;
  lines.forEach((line, j) => {
    const len = [...line].length;
    for (const c of moved) {
      const x = c.at ?? 0;
      if (x >= start && x <= start + len) out[j]!.push({ ...c, at: x - start });
    }
    start += len + 1;
  });
  return out;
}

export function paragraphText(p: Paragraph): string {
  return (p.runs ?? []).map((r) => r.text ?? "").join("");
}

/** The paragraph's chords with `chord` at character `at` (replacing one there; empty removes it). */
export function withChord(chords: ChordMark[] | undefined, at: number, chord: string): ChordMark[] {
  const rest = (chords ?? []).filter((c) => (c.at ?? 0) !== at);
  const t = chord.trim();
  if (t) rest.push({ at, chord: t });
  return rest.sort((x, y) => (x.at ?? 0) - (y.at ?? 0));
}

/** A copy of `tb` with paragraph `i`'s chords replaced. */
export function setParagraphChords(tb: TextBox, i: number, chords: ChordMark[]): TextBox {
  const paragraphs = (tb.paragraphs ?? []).map((p, k) => {
    if (k !== i) return p;
    const q: Paragraph = { ...p };
    if (chords.length) q.chords = chords;
    else delete q.chords;
    return q;
  });
  return { ...tb, paragraphs };
}

/**
 * `tb` with a chord set at character `at` of line `line` (empty `chord` removes it), or null when
 * that line no longer reads `expect` (the chart the operator clicked was built from other text,
 * e.g. before typing in the in-place editor: the chord would land on the wrong character).
 */
export function chordOnLine(tb: TextBox, line: number, expect: string, at: number, chord: string): TextBox | null {
  const p = tb.paragraphs?.[line];
  if (!p || paragraphText(p) !== expect) return null;
  return setParagraphChords(tb, line, withChord(p.chords, at, chord));
}

/** A line with its chords inline (`[G]Morning [D]light`). */
export function inlineChords(text: string, chords: ChordMark[] | undefined): string {
  const chars = [...text];
  const sorted = [...(chords ?? [])].sort((x, y) => (x.at ?? 0) - (y.at ?? 0));
  let out = "";
  let k = 0;
  const take = () => `[${sorted[k++]?.chord ?? ""}]`;
  chars.forEach((ch, i) => {
    while (k < sorted.length && (sorted[k]?.at ?? 0) <= i) out += take();
    out += ch;
  });
  while (k < sorted.length) out += take();
  return out;
}

/** A text box has a chord anywhere. */
export function hasChords(tb: TextBox | null | undefined): boolean {
  return (tb?.paragraphs ?? []).some((p) => (p.chords?.length ?? 0) > 0);
}
