import { describe, expect, it } from "vitest";
import type { Presentation, Slide, TranslateJob, TranslatedSlide, TranslationModel } from "$protocol";
import {
  chordOps,
  chordProPaste,
  chordProSplit,
  defaultSource,
  languageChoices,
  languageCode,
  translateSourceOrder,
  looksLikeChordPro,
  isMachineTranslation,
  keepTranslationOps,
  pasteOps,
  presentationLanguage,
  proposedText,
  secondText,
  secondTextOp,
  slideLanguage,
  splitTwoLanguages,
  translateJobText,
  translationSources,
  translationTargets,
  withSecondText,
} from "./language";

function slide(id: string, translation?: Slide["translation"]): Slide {
  return {
    id,
    elements: [{ id: `${id}-t`, bounds: { x: 0, y: 0, width: 100, height: 100 }, kind: { type: "text", text: { paragraphs: [{ runs: [{ text: "Line one" }] }] } } }],
    ...(translation ? { translation } : {}),
  } as Slide;
}

describe("second-language text", () => {
  it("reads a slide's second text and language", () => {
    const s = slide("a", { language: "es", texts: [{ element: "a-t", paragraphs: [{ runs: [{ text: "Línea " }, { text: "uno" }] }, {}, { runs: [{ text: "dos" }] }] }] });
    expect(secondText(s, "a-t")).toBe("Línea uno\n\ndos");
    expect(secondText(s, "other")).toBe("");
    expect(secondText(s, null)).toBe("");
    expect(slideLanguage(s)).toBe("es");
    expect(presentationLanguage([s, slide("b"), slide("c", { language: "pt" }), slide("d", { language: "es" })])).toBe("es");
    expect(presentationLanguage([slide("b")])).toBe("");
  });

  it("sets one element's text, keeping the others and the layout", () => {
    const s = slide("a", { language: "es", layout: "sideBySide", texts: [{ element: "x", paragraphs: [{ runs: [{ text: "Otro" }] }] }] });
    expect(withSecondText(s, "a-t", "", "Uno\n\nDos")).toEqual({
      language: "es",
      layout: "sideBySide",
      texts: [
        { element: "x", paragraphs: [{ runs: [{ text: "Otro" }] }] },
        { element: "a-t", paragraphs: [{ runs: [{ text: "Uno" }] }, {}, { runs: [{ text: "Dos" }] }] },
      ],
    });
    // A new language code wins; clearing the last visible text removes the layer.
    expect(withSecondText(slide("b"), "b-t", "pt", "Um")?.language).toBe("pt");
    expect(withSecondText(slide("b", { language: "es", texts: [{ element: "b-t", paragraphs: [{ runs: [{ text: "x" }] }] }] }), "b-t", "es", "  ")).toBeNull();
    expect(secondTextOp(slide("b"), "b-t", "fr", "Un")).toEqual({
      type: "setSlideTranslation",
      slide: "b",
      translation: { language: "fr", texts: [{ element: "b-t", paragraphs: [{ runs: [{ text: "Un" }] }] }] },
    });
  });
});

describe("pasting two languages", () => {
  it("splits alternating lines into stanzas", () => {
    const text = "Line one\nLínea uno\nLine two\nLínea dos\n\n\n  Chorus here \nCoro aquí\n";
    expect(splitTwoLanguages(text, { type: "alternating" })).toEqual([
      ["Line one\nLine two", "Línea uno\nLínea dos"],
      ["Chorus here", "Coro aquí"],
    ]);
  });

  it("splits by a separator line or a separator inside lines", () => {
    expect(splitTwoLanguages("One\nTwo\n---\nUno\nDos\n\nThree", { type: "separator", separator: "---" })).toEqual([
      ["One\nTwo", "Uno\nDos"],
      ["Three", ""],
    ]);
    expect(splitTwoLanguages("Three / Tres\r\nFour/Cuatro", { type: "separator", separator: "/" })).toEqual([["Three\nFour", "Tres\nCuatro"]]);
    // An empty separator falls back to alternating lines.
    expect(splitTwoLanguages("A\nB", { type: "separator", separator: " " })).toEqual([["A", "B"]]);
  });

  it("maps pasted slides onto the presentation's slides in order", () => {
    const a = slide("a");
    const b = slide("b");
    const slides = new Map([
      ["a", a],
      ["b", b],
    ]);
    const targets = [
      { slideId: "a", elementId: "a-t", text: "Line one" },
      { slideId: "b", elementId: "b-t", text: "Old" },
    ];
    const pairs: [string, string][] = [
      ["Line one", "Línea uno"],
      ["New", "Nuevo"],
      ["Extra", "Más"],
    ];
    const only = pasteOps(targets, slides, pairs, "es", false);
    expect(only.ops.map((o) => o.type)).toEqual(["setSlideTranslation", "setSlideTranslation"]);
    expect(only.leftover).toBe(1);
    const both = pasteOps(targets, slides, pairs, "es", true);
    // The unchanged primary is not sent again.
    expect(both.ops.map((o) => o.type)).toEqual(["setSlideTranslation", "setSlideText", "setSlideTranslation"]);
    expect(both.ops[1]).toEqual({ type: "setSlideText", slide: "b", text: "New", expectText: "Old" });
    expect(both.used).toBe(2);
  });
});

describe("pasting two languages in ChordPro", () => {
  it("knows ChordPro from plain lyrics", () => {
    expect(looksLikeChordPro("{title: Lanterns}\nWords")).toBe(true);
    expect(looksLikeChordPro("[G]Lanterns on the [C/E]shore")).toBe(true);
    expect(looksLikeChordPro("Calling [Am7]us [Dsus4]home [F#m7b5]now [Cmaj7]")).toBe(true);
    expect(looksLikeChordPro("Lanterns on the shore\nFaroles en la orilla")).toBe(false);
    // Brackets that are no chord, and braces inside a line.
    expect(looksLikeChordPro("[Chorus]\nSing {softly} now\n[Bridge]")).toBe(false);
  });

  it("asks the server for the paste's split", () => {
    expect(chordProSplit("alternating", "/")).toEqual({ type: "alternating" });
    expect(chordProSplit("line", "/")).toEqual({ type: "line" });
    expect(chordProSplit("inline", "|")).toEqual({ type: "inline", separator: "|" });
    expect(chordProSplit("comments", "/")).toEqual({ type: "comments" });
  });

  // What `POST /chordpro/parse` answers for a made-up two-language song.
  const el = (id: string, lines: [string, { at: number; chord: string }[]][]) => ({
    id,
    bounds: { x: 0, y: 0, width: 100, height: 100 },
    kind: { type: "text", text: { paragraphs: lines.map(([t, chords]) => ({ runs: [{ text: t }], ...(chords.length ? { chords } : {}) })) } },
  });
  const parsed = {
    id: "p",
    sections: [
      {
        id: "s",
        slides: [
          {
            id: "a",
            elements: [el("a-t", [["Lanterns on the shore", [{ at: 0, chord: "G" }, { at: 16, chord: "C" }]], ["Calling us home", []]])],
            translation: { language: "es", texts: [{ element: "a-t", paragraphs: [{ runs: [{ text: "Faroles en la orilla" }] }, { runs: [{ text: "Llamándonos a casa" }] }] }] },
          },
          { id: "b", elements: [el("b-t", [["Sing, sing", [{ at: 0, chord: "D" }]]])] },
        ],
      },
    ],
  } as unknown as Presentation;

  it("turns the parsed song into pasted slides, chords kept for applyChordPro", () => {
    const r = chordProPaste(parsed);
    expect(r.pairs).toEqual([
      ["Lanterns on the shore\nCalling us home", "Faroles en la orilla\nLlamándonos a casa"],
      ["Sing, sing", ""],
    ]);
    expect(r.chords).toEqual(["[G]Lanterns on the [C]shore", "[D]Sing, sing"]);
    expect(r.language).toBe("es");
  });

  it("adds the chords of the slides it filled, only when the first language is replaced", () => {
    const { chords } = chordProPaste(parsed);
    expect(chordOps(chords, 2, true)).toEqual([{ type: "applyChordPro", text: "[G]Lanterns on the [C]shore\n\n[D]Sing, sing" }]);
    // One slide to fill: the left-over slide's chords are not sent (they would match nothing).
    expect(chordOps(chords, 1, true)).toEqual([{ type: "applyChordPro", text: "[G]Lanterns on the [C]shore" }]);
    expect(chordOps(chords, 2, false)).toEqual([]);
    expect(chordOps(["", ""], 2, true)).toEqual([]);
  });
});

describe("machine translation", () => {
  const model = (id: string, from: string, to: string[], status: TranslationModel["status"]): TranslationModel => ({
    id,
    from,
    to,
    model: `opus-mt-${id}`,
    repo: `Example/opus-mt-${id}`,
    revision: "0".repeat(40),
    card: "https://example.invalid",
    licence: "Apache-2.0",
    attribution: "",
    size: 300_000_000,
    status,
  });

  it("offers the targets of the source language's models", () => {
    const models = [model("en-es", "en", ["es"], "ready"), model("en-pt", "en", ["pt", "pt-BR"], "notDownloaded"), model("es-en", "es", ["en"], "ready")];
    expect(translationTargets(models, "en-US")).toEqual([
      { code: "es", name: "Spanish", pair: "en-es", ready: true },
      { code: "pt", name: "Portuguese", pair: "en-pt", ready: false },
      { code: "pt-BR", name: "Portuguese (pt-BR)", pair: "en-pt", ready: false },
    ]);
    expect(translationTargets(models, "es").map((t) => t.code)).toEqual(["en"]);
    expect(translationTargets([], "en")).toEqual([]);
  });

  it("offers the source languages the models translate from, English first", () => {
    const models = [model("ko-en", "ko", ["en"], "notDownloaded"), model("es-en", "es", ["en"], "ready"), model("en-es", "en", ["es"], "ready"), model("en-pt", "en", ["pt"], "ready")];
    expect(translationSources(models)).toEqual([
      { code: "en", name: "English" },
      { code: "ko", name: "Korean" },
      { code: "es", name: "Spanish" },
    ]);
    expect(translationSources([])).toEqual([]);
  });

  it("starts from the presentation's language if known, else English, else what is offered", () => {
    const sources = [{ code: "en" }, { code: "es" }, { code: "ko" }];
    // What the operator last picked for this presentation, then its Scripture translation's language.
    expect(defaultSource(sources, ["es", "ko"])).toBe("es");
    expect(defaultSource(sources, [null, "ko-KR"])).toBe("ko");
    // A language no model translates from: English.
    expect(defaultSource(sources, ["sw", undefined])).toBe("en");
    expect(defaultSource(sources, [])).toBe("en");
    // No English model: the first offered.
    expect(defaultSource([{ code: "ko" }], [])).toBe("ko");
    expect(defaultSource([], ["es"])).toBe("en");
  });

  it("starts from the presentation's own language, then the last pick, then Scripture, then English", () => {
    const sources = [{ code: "en" }, { code: "es" }, { code: "ko" }, { code: "pt" }];
    const order = (presentation: string | null, remembered: string | null, scripture: string | null) =>
      defaultSource(sources, translateSourceOrder({ presentation, remembered, scripture }));
    expect(order("pt-BR", "es", "ko")).toBe("pt");
    expect(order(null, "es", "ko")).toBe("es");
    expect(order(null, null, "ko")).toBe("ko");
    expect(order(null, null, null)).toBe("en");
    // A language no model translates from is passed over.
    expect(order("sw", "es", null)).toBe("es");
    expect(order("sw", null, null)).toBe("en");
    expect(translateSourceOrder({ presentation: "es", remembered: "en", scripture: "ko" })).toEqual(["es", "en", "ko"]);
  });

  it("checks a typed language code as the server does", () => {
    expect(languageCode("es")).toBe("es");
    expect(languageCode(" pt_br ")).toBe("pt-BR");
    expect(languageCode("zh-hant-tw")).toBe("zh-Hant-TW");
    expect(languageCode("es-419")).toBe("es-419");
    for (const bad of ["", "e", "Spanish please", "en-", "en--US", "1en", "en-toolongsubtag", "español", "x".repeat(36)]) expect(languageCode(bad)).toBeNull();
  });

  it("offers the common languages, plus the presentation's when it is another", () => {
    expect(languageChoices(null).map((l) => l.code)).toContain("es");
    expect(languageChoices("es").length).toBe(languageChoices(null).length);
    expect(languageChoices("pt-BR").at(-1)).toEqual({ code: "pt-BR", name: "Portuguese (pt-BR)" });
    expect(languageChoices("haw").at(-1)).toEqual({ code: "haw", name: "haw" });
  });

  const proposal: TranslatedSlide = {
    slide: "a",
    translation: { language: "es", texts: [{ element: "a-t", paragraphs: [{ runs: [{ text: "Línea uno" }] }, {}, { runs: [{ text: "Dos" }] }] }], machine: { engine: "opus-mt (candle)", model: "opus-mt-en-es" } },
  };

  it("shows a proposal's text and keeps it as one op per slide, marked", () => {
    expect(proposedText(proposal, "a-t")).toBe("Línea uno\n\nDos");
    expect(proposedText(proposal, "other")).toBe("");
    expect(proposedText(undefined, "a-t")).toBe("");
    const ops = keepTranslationOps([proposal]);
    expect(ops).toEqual([{ type: "setSlideTranslation", slide: "a", translation: proposal.translation }]);
    expect(isMachineTranslation(slide("a", proposal.translation))).toBe(true);
    expect(isMachineTranslation(slide("a", { language: "es" }))).toBe(false);
  });

  it("drops the machine mark once someone edits the second text", () => {
    const s = slide("a", proposal.translation);
    const edited = withSecondText(s, "a-t", "es", "Línea corregida");
    expect(edited?.machine).toBeUndefined();
    expect(edited?.texts).toEqual([{ element: "a-t", paragraphs: [{ runs: [{ text: "Línea corregida" }] }] }]);
  });

  it("describes a preview's progress", () => {
    const job: TranslateJob = { id: "j", presentation: "p", from: "en", to: "es", model: "opus-mt-en-es", status: "downloading", done: 0, total: 3, downloaded: 150_000_000, downloadSize: 300_000_000, slides: [], notice: "" };
    expect(translateJobText(job)).toBe("Downloading the opus-mt-en-es model (first use): 150 of 300 MB");
    expect(translateJobText({ ...job, status: "translating", done: 1 })).toBe("Translating: 1 of 3 slides");
    expect(translateJobText({ ...job, status: "done", slides: [proposal], seconds: 1.234 })).toBe("1 slide translated in 1.2 s");
    expect(translateJobText({ ...job, status: "failed", detail: "no model" })).toBe("Translation failed: no model");
  });
});
