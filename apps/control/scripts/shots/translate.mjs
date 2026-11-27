// Shots of machine translation (docs/DUAL-LANGUAGE-PLAN.md §5; see scripts/shots.mjs for
// `ctx`): Reflow ▸ Second language ▸ "Translate to…" (the dialog), the preview in the
// second-language column with the machine translation banner, the slides after Keep (each
// second text tagged "MT"), the slide editor's Inspector ▸ Slide ▸ Second language with its own
// "Translate to…" (dialog with the source language, preview, Discard), Inspector ▸ Presentation
// ▸ Language (a typed code refused, then set; the dialog then starts from it), and Settings ▸
// Translation (pairs, licences, a model downloading, one failed). The stub translates the
// Reflow song (test/stub/routes/translate.mjs).
import { reflowStub } from "../../test/stub/routes/reflow.mjs";

export default async function ({ page, settle, shot, stub, step }) {
  const changed = () => stub.push({ type: "documentChanged", kind: "presentation", id: reflowStub.id, change: "changed" });
  reflowStub.reset();
  reflowStub.on = true;
  try {
    changed();
    await settle(600);
    await page.keyboard.press("Control+r");
    await settle(1200);
    await step("reflow translate", async () => {
      await page.getByRole("button", { name: "Second language", exact: true }).click();
      await settle(300);
      await page.getByRole("button", { name: "Translate to…" }).click();
      await page.getByTestId("translate-dialog").waitFor({ timeout: 2000 });
      await page.getByRole("combobox", { name: "Translate into" }).selectOption("es");
      await settle(300);
      await shot("reflow-translate-dialog");
      await page.getByRole("button", { name: "Translate", exact: true }).click();
      await page.getByRole("button", { name: /^Keep \d+ slides?$/ }).waitFor({ timeout: 5000 });
      await settle(600);
      await shot("reflow-translate-preview");
      await page.getByRole("button", { name: /^Keep \d+ slides?$/ }).click();
      await settle(1200);
      await shot("reflow-translate-kept");
      await page.getByRole("button", { name: "Second language", exact: true }).click();
      await settle(300);
    });
    // The slide editor: Inspector ▸ Slide ▸ Second language (the kept text, tagged "MT"), then
    // "Translate to…" for that slide: the same dialog, preview and Keep / Discard.
    await step("edit translate slide", async () => {
      await page.keyboard.press("Control+e");
      await settle(1500);
      // The second slide (the first is the empty title slide).
      await page.locator("[data-slide-row]").nth(1).click();
      await settle(400);
      await page.getByRole("tab", { name: "Slide", exact: true }).click();
      const sec = page.getByTestId("slide-language");
      await sec.scrollIntoViewIfNeeded();
      await settle(400);
      await shot("edit-language");
      await sec.getByTestId("slide-translate").click();
      await page.getByTestId("translate-dialog").waitFor({ timeout: 2000 });
      // The source language: Spanish offers only the Spanish → English model.
      await page.getByRole("combobox", { name: "Translate from" }).selectOption("es");
      await settle(300);
      const into = await page.getByRole("combobox", { name: "Translate into" }).locator("option").allTextContents();
      if (into.join() !== "English (model downloads first)") throw new Error(`from Spanish offers ${into.join()}`);
      await shot("edit-translate-dialog");
      await page.getByRole("combobox", { name: "Translate from" }).selectOption("en");
      await page.getByRole("button", { name: "Translate", exact: true }).click();
      await sec.getByRole("button", { name: /^Keep \d+ slides?$/ }).waitFor({ timeout: 5000 });
      await sec.scrollIntoViewIfNeeded();
      await settle(600);
      await shot("edit-translate-preview");
      await sec.getByRole("button", { name: "Discard" }).click();
      await settle(300);
      if (await sec.getByTestId("slide-proposal").count()) throw new Error("Discard left the proposal");
    });
    // The presentation's own language (Inspector ▸ Presentation ▸ Language, "Other…" for a code)
    // is the Translate dialog's first choice for From, before what the operator last picked.
    await step("edit presentation language", async () => {
      await page.getByRole("tab", { name: "Presentation", exact: true }).click();
      const sec = page.getByTestId("pres-language");
      await sec.scrollIntoViewIfNeeded();
      const pick = sec.getByRole("combobox", { name: "Presentation language" });
      await pick.selectOption("__other");
      const code = sec.getByRole("textbox", { name: "Language code" });
      await code.fill("Spanish please");
      await sec.getByRole("button", { name: "Set", exact: true }).click();
      await settle(300);
      await shot("edit-pres-language-other");
      await code.fill("es_mx");
      await sec.getByRole("button", { name: "Set", exact: true }).click();
      await settle(1200);
      if ((await pick.inputValue()) !== "es-MX") throw new Error(`language is ${await pick.inputValue()}`);
      await sec.scrollIntoViewIfNeeded();
      await shot("edit-pres-language");
      await page.getByRole("tab", { name: "Slide", exact: true }).click();
      const slide = page.getByTestId("slide-language");
      await slide.scrollIntoViewIfNeeded();
      await slide.getByTestId("slide-translate").click();
      await page.getByTestId("translate-dialog").waitFor({ timeout: 2000 });
      await settle(400);
      const from = await page.getByRole("combobox", { name: "Translate from" }).inputValue();
      if (from !== "es") throw new Error(`From starts at ${from}, not the presentation's Spanish`);
      await shot("edit-translate-from-language");
      await page.getByRole("dialog").getByRole("button", { name: "Cancel", exact: true }).click();
      await settle(300);
    });
  } finally {
    reflowStub.on = false;
    reflowStub.reset();
    changed();
    await settle(400);
  }
  await step("settings translation", async () => {
    await page.keyboard.press("Control+Comma");
    await settle(600);
    try {
      const nav = page.getByRole("navigation", { name: "Settings sections" });
      await nav.getByRole("button", { name: "Translation", exact: true }).click();
      await page.getByTestId("translation-settings").getByRole("table").waitFor({ timeout: 3000 });
      await settle(400);
      await shot("settings-translation");
    } finally {
      await page.getByRole("toolbar", { name: "Settings" }).getByRole("button", { name: "Close" }).click();
      await settle(200);
    }
  });
}
