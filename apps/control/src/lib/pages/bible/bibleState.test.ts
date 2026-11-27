import { afterEach, describe, expect, it } from "vitest";
import type { Connection } from "../../server";
import { ApiRequestError } from "../../server";
import { session } from "../../state/session.svelte";
import { bible } from "./bibleState.svelte";

// Synthetic books and verses (made-up text).
const BOOKS = [
  { key: "AAA", name: "Alpha", testament: "new", chapters: [20, 20, 20] },
  { key: "BBB", name: "Beta", testament: "new", chapters: [20, 20, 20] },
];
const verses = (book: string, chapter: number) =>
  Array.from({ length: 20 }, (_, i) => ({ book, chapter, verse: i + 1, text: `Line ${i + 1}` }));

function fakeServer(opts: { failPassage?: boolean } = {}) {
  const rest = {
    get: async (path: string, params?: Record<string, string>) => {
      if (path === "/scripture/translations") return [{ id: "T1", abbreviation: "T1", name: "Test one", language: "en", license: "", origin: "bundled", books: 2 }];
      if (path.endsWith("/books")) {
        if (!path.includes("/T1/")) throw new ApiRequestError("notFound", "no such translation", 404);
        return BOOKS;
      }
      if (path === "/scripture/passage") {
        if (opts.failPassage) throw new ApiRequestError("internal", "broken", 500);
        const [name, ch] = params!.ref!.split(" ");
        const b = BOOKS.find((x) => x.name === name)!;
        return { translation: "T1", book: b.key, label: params!.ref, ranges: [], verses: verses(b.key, Number(ch)) };
      }
      throw new Error(`unexpected ${path}`);
    },
    themes: async () => [],
  };
  session.conn = { rest } as unknown as Connection;
}

afterEach(() => {
  session.conn = null;
});

describe("the Bible page's state", () => {
  it("picking another book drops the old chapter's verses and selection (A16-1)", async () => {
    fakeServer();
    await bible.load();
    await bible.openChapter("AAA", 3, [16, 17, 18]);
    expect(bible.request()?.reference).toBe("Alpha 3:16–18");
    bible.pickBook("BBB");
    expect(bible.request()).toBeNull();
    expect(bible.verses).toEqual([]);
    expect(bible.heading).toBe("");
    // The same book again keeps what is shown.
    await bible.openChapter("AAA", 2, [1]);
    bible.pickBook("AAA");
    expect(bible.request()?.reference).toBe("Alpha 2:1");
  });

  it("a chapter that can't be read keeps the book and chapter that were shown (A16-1)", async () => {
    fakeServer();
    await bible.load();
    await bible.openChapter("AAA", 3, [16]);
    fakeServer({ failPassage: true });
    await bible.openChapter("BBB", 1);
    expect(bible.book).toBe("AAA");
    expect(bible.chapter).toBe(3);
    expect(bible.request()?.reference).toBe("Alpha 3:16");
    expect(bible.problem).toContain("Could not read Beta 1");
  });

  it("a translation this server doesn't have is refused and the old one kept (A16-9)", async () => {
    fakeServer();
    await bible.load();
    expect(bible.translation).toBe("T1");
    expect(await bible.setTranslation("ELSEWHERE")).toBe(false);
    expect(bible.translation).toBe("T1");
    expect(bible.problem).toContain("ELSEWHERE");
    // Not yet in the list (it wasn't loaded): the books' 404 says so, the old one stays.
    bible.translations = [];
    expect(await bible.setTranslation("ELSEWHERE")).toBe(false);
    expect(bible.translation).toBe("T1");
    expect(bible.problem).toContain("Could not read the books of ELSEWHERE");
    expect(await bible.setTranslation("T1")).toBe(true);
    expect(bible.problem).toBeNull();
  });
});
