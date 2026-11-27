// Shots of the Reflow page (see scripts/shots.mjs for `ctx`): the page with a synthetic song
// (groups, hotkeys, media cues: test/stub/routes/reflow.mjs), a slide's "…" menu opened from
// the keyboard (Shift+F10), the second-language column and its "Paste two languages" dialog
// (plain text, then ChordPro), and the "Keep mine / Use theirs" conflict prompt.
import { reflowStub } from "../../test/stub/routes/reflow.mjs";

export default async function ({ page, settle, shot, stub, step }) {
  // The song stands in for the presentation the Show page has open; the change event reloads it.
  const changed = () => stub.push({ type: "documentChanged", kind: "presentation", id: reflowStub.id, change: "changed" });
  reflowStub.reset();
  reflowStub.on = true;
  try {
    changed();
    await settle(600);
    await page.keyboard.press("Control+r");
    await settle(1200);
    await shot("page-reflow");

    await step("reflow slide menu", async () => {
      const text = page.locator("textarea[data-reflow-text]").nth(3);
      await text.click();
      await page.keyboard.press("End");
      await page.keyboard.press("Shift+F10");
      await page.getByRole("menu", { name: "Options for slide 4" }).waitFor({ timeout: 2000 });
      await settle(200);
      await shot("reflow-menu");
      await page.keyboard.press("Escape");
      await settle(200);
    });

    await step("reflow conflict", async () => {
      reflowStub.conflictText = "Carry the light, carry it high\nOver the river, under the sky";
      const text = page.locator("textarea[data-reflow-text]").nth(3);
      await text.click();
      await page.keyboard.press("End");
      await page.keyboard.type(" tonight");
      await page.getByRole("button", { name: "Use theirs" }).waitFor({ timeout: 3000 });
      await settle(300);
      await shot("reflow-conflict");
      await page.getByRole("button", { name: "Use theirs" }).click();
      await settle(300);
    });

    await step("reflow second language", async () => {
      // Dual-language slides (batch 16): the second-language column, typed into, and the
      // "Paste two languages" dialog with a preview.
      await page.getByRole("button", { name: "Second language", exact: true }).click();
      await settle(300);
      const code = page.getByRole("combobox", { name: "Second language code" });
      await code.fill("es");
      await code.press("Tab");
      await settle(300);
      const lines = ["Faroles en la colina\nbrillan para el que vuelve", "Lleva la luz, llévala en alto\nsobre el río, bajo el cielo"];
      for (const [i, t] of lines.entries()) {
        await page.locator("textarea[data-reflow-second]").nth(i + 1).fill(t);
      }
      await settle(1200);
      await shot("reflow-second-language");
      await page.getByRole("button", { name: "Paste two languages…" }).click();
      await page.getByTestId("paste-two-languages").fill(
        "Lanterns on the hill tonight\nFaroles en la colina esta noche\nShining for the ones who roam\nBrillan para los que vagan\n\nCarry the light, carry it high\nLleva la luz, llévala en alto",
      );
      await settle(400);
      await shot("reflow-paste-two-languages");
      // ChordPro pasted: the server reads it (its chords stay on the first language).
      await page.getByTestId("paste-two-languages").fill(
        "{meta: second_language es}\n{sov}\n[G]Lanterns on the [C]hill tonight\nFaroles en la colina esta noche\n[Em]Shining for the [D]ones who roam\nBrillan para los que vagan\n{eov}\n\n{soc}\n[C]Carry the light, [G]carry it high\nLleva la luz, llévala en alto\n{eoc}",
      );
      await page.getByTestId("paste-chordpro").filter({ hasText: "with chords" }).waitFor({ timeout: 3000 });
      await settle(300);
      await shot("reflow-paste-chordpro");
      // Esc closes it (a page-local dialog, H11-6).
      await page.keyboard.press("Escape");
      await page.getByTestId("paste-two-languages").waitFor({ state: "hidden", timeout: 2000 });
      await page.getByRole("button", { name: "Second language", exact: true }).click();
      await settle(300);
    });
  } finally {
    reflowStub.on = false;
    reflowStub.conflictText = null;
    changed();
    await settle(400);
  }
}
