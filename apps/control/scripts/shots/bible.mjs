// Shots of the Bible page (see scripts/shots.mjs for `ctx`), over the stub's made-up mini
// translation (test/stub/routes/bible.mjs, which also lists two made-up scripture themes while
// `bibleStub.on`): the page before a passage is chosen, a typed reference with its parse
// feedback, the passage with verses selected and its generated slides with a theme, a parallel
// passage (a second translation side by side, then stacked), a word search, a bad reference,
// and Settings ▸ Bible.
import { bibleStub } from "../../test/stub/routes/bible.mjs";

export default async function ({ page, openPage, settle, shot, step }) {
  bibleStub.on = true;
  try {
    await openPage("Bible");
    await shot("page-bible");
    const box = page.getByTestId("bible-reference");
    await step("bible passage", async () => {
      await box.fill("Ps 23:1-4, 6");
      await settle(500);
      await shot("bible-reference-typed");
      await box.press("Enter");
      await settle(700);
      await page.getByTestId("bible-theme").selectOption({ label: "Scripture — Lower third" });
      await settle(900);
      await shot("bible-passage");
    });
    await step("bible two verses per slide", async () => {
      await page.getByRole("radiogroup", { name: "Verses per slide" }).getByRole("radio", { name: "2 verses" }).click();
      await page.getByRole("switch", { name: "Verse numbers" }).click();
      await settle(900);
      await shot("bible-passage-split");
    });
    await step("bible parallel", async () => {
      // Parallel Bible (batch 16): a second translation beside the first, then stacked.
      const second = page.getByTestId("bible-second");
      try {
        await second.selectOption({ value: "PRU" });
        await settle(900);
        await shot("bible-parallel");
        await page.getByRole("radiogroup", { name: "Second translation layout" }).getByRole("radio", { name: "Stacked" }).click();
        await settle(900);
        await shot("bible-parallel-stacked");
        await page.getByRole("radiogroup", { name: "Second translation layout" }).getByRole("radio", { name: "Side by side" }).click();
      } finally {
        await second.selectOption({ value: "" });
        await settle(300);
      }
    });
    await step("bible search", async () => {
      await box.fill("lantern road");
      await box.press("Enter");
      await settle(700);
      await shot("bible-search");
    });
    await step("bible bad reference", async () => {
      await box.fill("John 3:40");
      await settle(600);
      await shot("bible-reference-bad");
    });
  } finally {
    bibleStub.on = false;
  }
  await step("settings bible", async () => {
    await page.keyboard.press("Control+Comma");
    await settle(600);
    try {
      await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Bible", exact: true }).click();
      await settle(400);
      await shot("settings-bible");
    } finally {
      await page.getByRole("toolbar", { name: "Settings" }).getByRole("button", { name: "Close" }).click();
      await settle(200);
    }
  });
}
