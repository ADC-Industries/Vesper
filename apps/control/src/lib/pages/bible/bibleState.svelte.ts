// The Bible page's state (Svelte 5 runes): the translations and the chosen one's books, the
// verses shown (a chapter, or a typed passage), the selection, the reference box's parse
// feedback and word search, the slide options and their split preview, and the preferences
// Settings ▸ Bible changes (default translation, verse numbers, slide split) and the recent
// references, remembered per browser (and in Oriel's preferences backup).

import type { ScriptureBook, ScriptureParse, ScriptureSearch, ScriptureSlides, ScriptureTranslation, ScriptureVerse, ThemeSummary } from "$protocol";
import { ScriptureClient } from "../../server/scripture";
import { errorText } from "../../state/errors";
import { creds } from "../../state/prefs";
import { session } from "../../state/session.svelte";
import {
  clickVerse,
  parseRecent,
  parseSecondLayout,
  parseSplit,
  pushRecent,
  referenceFromSelection,
  selectAll,
  slideRequest,
  verseKey,
  type Recent,
  type SecondLayout,
  type Split,
  type VerseKey,
} from "./bible";

/** Preference keys (`creds.pref`), also in Oriel's preferences backup. */
export const BIBLE_PREFS = {
  translation: "bibleTranslation",
  verseNumbers: "bibleVerseNumbers",
  split: "bibleSplit",
  recent: "bibleRecent",
  secondTranslation: "bibleSecondTranslation",
  secondLayout: "bibleSecondLayout",
} as const;

class BibleState {
  translations = $state.raw<ScriptureTranslation[]>([]);
  /** The chosen translation's ID. */
  translation = $state<string>(creds.pref(BIBLE_PREFS.translation) ?? "");
  books = $state.raw<ScriptureBook[]>([]);
  book = $state<string | null>(null);
  chapter = $state<number | null>(null);
  /** The verses shown, with the heading above them. */
  verses = $state.raw<ScriptureVerse[]>([]);
  heading = $state("");
  selected = $state.raw<Set<VerseKey>>(new Set());
  anchor = $state<number | null>(null);
  /** The reference box. */
  query = $state("");
  parsed = $state.raw<ScriptureParse | null>(null);
  results = $state.raw<ScriptureSearch | null>(null);
  /** The words `results` were searched for. */
  searched = $state("");
  loading = $state(false);
  problem = $state<string | null>(null);
  /** Slide options. */
  split = $state<Split>(parseSplit(creds.pref(BIBLE_PREFS.split)));
  verseNumbers = $state(creds.pref(BIBLE_PREFS.verseNumbers) === "1");
  showReference = $state(true);
  /** A second translation shown beside the first (parallel Bible; "" = none). */
  second = $state<string>(creds.pref(BIBLE_PREFS.secondTranslation) ?? "");
  secondLayout = $state<SecondLayout>(parseSecondLayout(creds.pref(BIBLE_PREFS.secondLayout)));
  theme = $state<string>("");
  themes = $state.raw<ThemeSummary[]>([]);
  preview = $state.raw<ScriptureSlides | null>(null);
  previewProblem = $state<string | null>(null);
  recent = $state.raw<Recent[]>(parseRecent(creds.pref(BIBLE_PREFS.recent)));
  busy = $state(false);
  /** False until the first load (the page shows "Reading…"). */
  ready = $state(false);

  private parseSeq = 0;
  private previewSeq = 0;

  client(): ScriptureClient | null {
    const conn = session.conn;
    return conn ? new ScriptureClient(conn.rest) : null;
  }

  get bookInfo(): ScriptureBook | null {
    return this.books.find((b) => b.key === this.book) ?? null;
  }

  get currentTranslation(): ScriptureTranslation | null {
    return this.translations.find((t) => t.id === this.translation) ?? null;
  }

  /** The selection as a reference ("Psalm 23:1–4, 6"), "" when nothing is selected. */
  get reference(): string {
    const b = this.bookInfo;
    return b ? referenceFromSelection(b.name, this.selected) : "";
  }

  /** Loads the translations, the themes and the default translation's books. */
  async load() {
    const c = this.client();
    if (!c) return;
    try {
      const [ts, th] = await Promise.allSettled([c.translations(), c.rest.themes()]);
      if (th.status === "fulfilled") this.themes = th.value;
      if (ts.status === "rejected") throw ts.reason;
      this.translations = ts.value;
      if (!ts.value.some((t) => t.id === this.translation)) this.translation = ts.value[0]?.id ?? "";
      if (this.translation) await this.loadBooks();
      this.problem = null;
    } catch (e) {
      this.problem = `Could not read the translations: ${errorText(e)}`;
    } finally {
      this.ready = true;
    }
  }

  /** Switches the translation; false (with `problem` set, the old one kept) when this server
   * doesn't have it or its books can't be read. A recent reference or a taken-over preference
   * can name a translation of another server. */
  async setTranslation(id: string): Promise<boolean> {
    const was = this.translation;
    if (this.translations.length && !this.translations.some((t) => t.id === id)) {
      this.problem = `This server has no translation “${id}”.`;
      return false;
    }
    this.translation = id;
    try {
      await this.loadBooks();
    } catch (e) {
      this.translation = was;
      this.problem = `Could not read the books of ${id}: ${errorText(e)}`;
      return false;
    }
    this.problem = null;
    // The same passage in the new translation.
    if (this.book && this.chapter) {
      const keep = this.selected;
      await this.openChapter(this.book, this.chapter);
      this.selected = new Set([...keep].filter((k) => this.verses.some((v) => verseKey(v) === k)));
    }
    return true;
  }

  private async loadBooks() {
    const c = this.client();
    if (!c || !this.translation) return;
    this.books = await c.books(this.translation);
    if (this.book && !this.books.some((b) => b.key === this.book)) {
      this.book = null;
      this.chapter = null;
      this.verses = [];
    }
  }

  pickBook(key: string) {
    // Another book: the verses shown and their selection belong to the old one (the reference
    // is built from the book's name and the selected chapter:verse keys).
    if (key !== this.book) this.clearPassage();
    this.book = key;
    this.chapter = null;
    this.results = null;
  }

  private clearPassage() {
    this.verses = [];
    this.heading = "";
    this.selected = new Set();
    this.anchor = null;
  }

  /** Shows a whole chapter; `select` verses are selected. */
  async openChapter(book: string, chapter: number, select: number[] = []) {
    const c = this.client();
    const b = this.books.find((x) => x.key === book);
    if (!c || !b) return;
    this.loading = true;
    try {
      const p = await c.passage(this.translation, `${b.name} ${chapter}`);
      // The book and chapter change with their verses (a failed read keeps what was shown).
      this.book = book;
      this.chapter = chapter;
      this.results = null;
      this.verses = p.verses;
      this.heading = p.label;
      this.selected = new Set(select.map((v) => `${chapter}:${v}`));
      this.anchor = null;
      this.problem = null;
    } catch (e) {
      this.problem = `Could not read ${b.name} ${chapter}: ${errorText(e)}`;
    } finally {
      this.loading = false;
    }
  }

  /** Live feedback while typing (the server parses). */
  async typed(q: string) {
    this.query = q;
    const seq = ++this.parseSeq;
    const c = this.client();
    if (!c || !q.trim() || !this.translation) {
      this.parsed = null;
      return;
    }
    try {
      const p = await c.parse(this.translation, q);
      if (seq === this.parseSeq) this.parsed = p;
    } catch {
      if (seq === this.parseSeq) this.parsed = null;
    }
  }

  /** Enter in the reference box: open the reference, else search for the words. */
  async submit() {
    const q = this.query.trim();
    const c = this.client();
    if (!q || !c || !this.translation) return;
    let p = this.parsed;
    try {
      p = await c.parse(this.translation, q);
      this.parsed = p;
    } catch {
      /* searched below */
    }
    if (p?.ok && p.book) await this.openReference(p.book, p.ranges, p.label ?? q);
    else await this.search(q);
  }

  /** Opens a parsed reference: its chapter with its verses selected (a passage over several
   * chapters shows just those verses, all selected). */
  async openReference(book: string, ranges: ScriptureParse["ranges"], label: string) {
    const first = ranges[0];
    if (!first) return;
    const oneChapter = ranges.every((r) => r.chapter === first.chapter && r.endChapter === first.chapter);
    if (oneChapter) {
      const vs: number[] = [];
      for (const r of ranges) for (let v = r.verse; v <= r.endVerse; v++) vs.push(v);
      await this.openChapter(book, first.chapter, vs);
    } else {
      const c = this.client();
      if (!c) return;
      this.loading = true;
      try {
        const p = await c.passage(this.translation, label);
        this.book = book;
        this.chapter = first.chapter;
        this.results = null;
        this.verses = p.verses;
        this.heading = p.label;
        this.selected = selectAll(p.verses);
        this.anchor = null;
      } catch (e) {
        this.problem = errorText(e);
      } finally {
        this.loading = false;
      }
    }
    this.remember(label);
  }

  async search(q: string) {
    const c = this.client();
    if (!c) return;
    this.loading = true;
    try {
      this.results = await c.search(this.translation, q);
      this.searched = q;
      this.problem = null;
    } catch (e) {
      this.problem = `Could not search: ${errorText(e)}`;
    } finally {
      this.loading = false;
    }
  }

  clickVerse(index: number, shift: boolean) {
    const r = clickVerse(this.verses, this.selected, this.anchor, index, shift);
    this.selected = r.selected;
    this.anchor = r.anchor;
  }
  selectAll() {
    this.selected = selectAll(this.verses);
  }
  selectNone() {
    this.selected = new Set();
    this.anchor = null;
  }

  remember(reference: string) {
    if (!reference || !this.translation) return;
    this.recent = pushRecent(this.recent, { translation: this.translation, reference });
    creds.setPref(BIBLE_PREFS.recent, JSON.stringify(this.recent));
  }

  /** The slide request for the selection and options (null when nothing is selected). */
  request() {
    const reference = this.reference;
    if (!reference || !this.translation) return null;
    return slideRequest(this.translation, reference, {
      split: this.split,
      showReference: this.showReference,
      verseNumbers: this.verseNumbers,
      theme: this.theme || null,
      second: this.translations.some((t) => t.id === this.second) ? this.second : null,
      secondLayout: this.secondLayout,
    });
  }

  /** Picks the second translation ("" = none); remembered. */
  setSecond(id: string) {
    this.second = id;
    creds.setPref(BIBLE_PREFS.secondTranslation, id);
  }
  setSecondLayout(l: SecondLayout) {
    this.secondLayout = l;
    creds.setPref(BIBLE_PREFS.secondLayout, l);
  }

  /** Refreshes the split preview (the server splits; nothing is written). */
  async refreshPreview() {
    const req = this.request();
    const seq = ++this.previewSeq;
    const c = this.client();
    if (!req || !c) {
      this.preview = null;
      return;
    }
    try {
      const s = await c.slides(req);
      if (seq === this.previewSeq) {
        this.preview = s;
        this.previewProblem = null;
      }
    } catch (e) {
      if (seq === this.previewSeq) {
        this.preview = null;
        this.previewProblem = errorText(e);
      }
    }
  }

  /** Creates the presentation: in the open playlist, or shown now. */
  async create(then: "playlist" | "show"): Promise<boolean> {
    const req = this.request();
    const conn = session.conn;
    if (!req || !conn) return false;
    const playlist = session.playlistId;
    if (then === "playlist" && !playlist) return false;
    this.busy = true;
    try {
      const r = await new ScriptureClient(conn.rest).create({ ...req, then, ...(then === "playlist" ? { playlist: playlist! } : {}) });
      this.remember(req.reference);
      const where = then === "show" ? "is live" : `was added to ${session.playlist?.name ?? "the playlist"}`;
      conn.pushNotice("info", `${r.name} ${where}`);
      if (then === "playlist" && playlist) void session.selectPlaylist(playlist, true);
      return true;
    } catch (e) {
      conn.pushNotice("error", `Could not create the passage: ${errorText(e)}`);
      return false;
    } finally {
      this.busy = false;
    }
  }

  // ---- Settings ▸ Bible

  setDefaultTranslation(id: string) {
    creds.setPref(BIBLE_PREFS.translation, id);
    if (id !== this.translation) void this.setTranslation(id);
  }
  setVerseNumbers(on: boolean) {
    this.verseNumbers = on;
    creds.setPref(BIBLE_PREFS.verseNumbers, on ? "1" : "0");
  }
  setSplit(s: Split) {
    this.split = s;
    creds.setPref(BIBLE_PREFS.split, s);
  }
}

export const bible = new BibleState();
