// Shots of chords (batch 16, see scripts/shots.mjs for `ctx`), with the slide editor's stub song
// (test/stub/routes/edit.mjs: written in G, played in A, chords on Verse 1): the Presentation
// tab's Key & Chords, "Add Chords from ChordPro", the Text tab's chord lines (and a chord's
// menu), Import ▸ ChordPro Song (test/stub/routes/chords.mjs reads the pasted song; then a
// two-language song, alternating lines), and the
// Stage page's chord view on a musician's layout (capo 2). Every song text here is made up.
import { editStub } from "../../test/stub/routes/edit.mjs";
import { stageStub } from "../../test/stub/routes/stage.mjs";

const SONG = `{title: Harbour Lights}
{key: D}

{start_of_verse}
[D]Lanterns on the [G]harbour wall
[Bm]Calling every [A]sailor home
{end_of_verse}

{start_of_chorus}
[G]Sing, [D/F#]sing the [Em7]evening [A]in
{end_of_chorus}`;

/** The same song in two languages, alternating lines (made-up Spanish). */
const TWO = `{title: Harbour Lights}
{key: D}
{meta: second_language es}

{start_of_verse}
[D]Lanterns on the [G]harbour wall
Faroles en el muro del puerto
[Bm]Calling every [A]sailor home
Llamando a cada marinero
{end_of_verse}

{start_of_chorus}
[G]Sing, [D/F#]sing the [Em7]evening [A]in
Canta, canta, que llega la tarde
{end_of_chorus}`;

/** A musician's layout: a key line, the current and next chord charts, a clock. */
function bandLayout() {
  const W = { r: 1, g: 1, b: 1 };
  const text = (id, name, bounds, line, size, link, color = W) => ({
    id,
    name,
    bounds,
    kind: { type: "text", text: { paragraphs: [{ runs: [{ text: line }] }], defaultStyle: { fontFamily: "DejaVu Sans", fontWeight: 700, size, color }, defaultParagraph: { alignment: "left" }, verticalAlign: "top", fit: "shrinkToFit", chords: { show: true } } },
    dataLink: link,
  });
  return {
    revision: "stub-band",
    schemaVersion: 2,
    id: "c4000000-0000-4000-8000-000000000001",
    name: "Guitar",
    sortKey: "W",
    size: { width: 1920, height: 1080 },
    chords: { capo: 2 },
    slide: {
      id: "c4000000-0000-4000-8000-000000000002",
      elements: [
        text("c4000000-0000-4000-8000-000000000003", "Song Key", { x: 40, y: 16, width: 900, height: 90 }, "Key G", 64, { type: "songKey" }, { r: 1, g: 0.84, b: 0.25 }),
        text("c4000000-0000-4000-8000-000000000004", "Chord Chart", { x: 40, y: 136, width: 1840, height: 520 }, "Current Slide", 96, { type: "chordChart" }),
        text("c4000000-0000-4000-8000-000000000005", "Next Chord Chart", { x: 40, y: 752, width: 1840, height: 150 }, "Next Slide", 60, { type: "chordChart", which: "next" }),
      ],
    },
  };
}

export default async function ({ page, settle, shot, stub, step, esc, openPage }) {
  const changed = () => stub.push({ type: "documentChanged", kind: "presentation", id: editStub.id, change: "changed" });
  const row = (name) => page.locator("[data-object-row]").filter({ has: page.locator(".name").getByText(name, { exact: true }) });
  const tab = (name) => page.getByRole("tab", { name, exact: true });
  editStub.reset();
  editStub.on = true;
  try {
    changed();
    await settle(600);
    await page.keyboard.press("Control+e");
    await settle(1500);
    await page.mouse.move(5, 300);

    await step("song key and transposition", async () => {
      await page.getByTestId("song-key").scrollIntoViewIfNeeded();
      await settle(300);
      const played = await page.getByTestId("played-key").textContent();
      if (!played?.startsWith("A")) throw new Error(`the song plays in ${played}, not A`);
      await shot("edit-song-key");
    });

    await step("add chords from ChordPro", async () => {
      await page.getByTestId("add-chordpro").click();
      await settle(400);
      await page.getByTestId("chordpro-text").fill(SONG);
      await settle(200);
      await shot("chordpro-apply");
      await page.getByRole("dialog").getByRole("button", { name: "Add Chords" }).click();
      await settle(600);
      const sent = editStub.edits.flatMap((e) => e.ops).filter((o) => o.type === "applyChordPro");
      if (!sent.length) throw new Error("no applyChordPro edit was sent");
    });

    await step("chord lines of a text object", async () => {
      await page.locator('[data-slide-row="51000000-0000-4000-8000-000000000021"]').click();
      await settle(800);
      await row("Lyrics").click();
      await settle(500);
      await tab("Text").click();
      await settle(300);
      await page.getByTestId("insp-chords").scrollIntoViewIfNeeded();
      await settle(300);
      await shot("edit-chords");
      await page.getByTestId("insp-chords").locator(".chord.set").first().click();
      await settle(300);
      await shot("edit-chord-menu");
      await esc();
    });
  } finally {
    editStub.on = false;
    changed();
    await settle(400);
  }

  await step("import a ChordPro song", async () => {
    await page.keyboard.press("Control+s");
    await settle(500);
    await page.locator(".oriel .logo").click();
    await settle(200);
    await page.locator('[data-cat="3"]').hover();
    await settle(150);
    await page.getByRole("menu").getByRole("menuitem", { name: "ChordPro Song…", exact: true }).click();
    await settle(400);
    await page.getByTestId("chordpro-text").fill(SONG);
    await page.getByTestId("chordpro-lines").selectOption("2");
    await settle(600);
    await page.getByTestId("chordpro-preview").waitFor();
    await shot("chordpro-import");
    await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();
    await settle(300);
  });

  await step("import a two-language ChordPro song", async () => {
    await page.keyboard.press("Control+s");
    await settle(500);
    await page.locator(".oriel .logo").click();
    await settle(200);
    await page.locator('[data-cat="3"]').hover();
    await settle(150);
    await page.getByRole("menu").getByRole("menuitem", { name: "ChordPro Song…", exact: true }).click();
    await settle(400);
    await page.getByTestId("chordpro-text").fill(TWO);
    await page.getByTestId("chordpro-second").selectOption("alternating");
    await settle(600);
    const preview = page.getByTestId("chordpro-preview");
    await preview.waitFor();
    if (!/2 slides with a second language \(es\)/.test(await preview.textContent())) throw new Error(`preview: ${await preview.textContent()}`);
    await shot("chordpro-import-two-languages");
    // Two lines a slide, a verse line without its translation: the second lines don't pair, so
    // they stay on the stanza's first slide and the preview says so (H06-1).
    await page.getByTestId("chordpro-text").fill(TWO.replace("Llamando a cada marinero\n", "Llamando a cada marinero\n[G]Over the [D]water\n"));
    await page.getByTestId("chordpro-lines").selectOption("2");
    await settle(600);
    await page.getByTestId("chordpro-preview").filter({ hasText: "don't pair" }).waitFor({ timeout: 3000 });
    await shot("chordpro-import-unpaired");
    await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();
    await settle(300);
    if (await page.getByTestId("chordpro-dialog").count()) throw new Error("Cancel left the dialog open");
  });

  stageStub.extra = [bandLayout()];
  stageStub.start();
  stub.push({ type: "documentChanged", kind: "stageLayout", id: stageStub.extra[0].id, change: "added" });
  try {
    await step("a musician's stage layout", async () => {
      await settle(300);
      await openPage("Stage");
      await settle(800);
      await page.locator(".side.left").getByText("Guitar", { exact: true }).first().click();
      await settle(600);
      await page.getByTestId("stage-chords").scrollIntoViewIfNeeded();
      await page.getByTestId("stage-accidentals").selectOption("flats");
      await settle(600);
      await shot("stage-chords");
    });
  } finally {
    stageStub.stop();
    stub.push({ type: "documentChanged", kind: "stageLayout", id: stageStub.extra[0].id, change: "deleted" });
    stageStub.extra = [];
    await settle(300);
  }
}
