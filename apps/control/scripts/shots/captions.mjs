// Shots of the Show page's captions strip (docs/CAPTIONS-PLAN.md; see scripts/shots.mjs for
// `ctx`), against the stub's captions (test/stub/routes/captions.mjs: running on the program
// audio with a synthetic line): on, with the newest line as a live preview; switched off (no
// line, the status says so); switched on again.

export default async function ({ page, settle, shot, step }) {
  const strip = page.getByTestId("captions-strip");
  const toggle = page.getByTestId("captions-toggle");
  await step("captions strip: on, with the newest line", async () => {
    await strip.scrollIntoViewIfNeeded();
    await settle(400);
    if ((await toggle.getAttribute("aria-checked")) !== "true") throw new Error("captions are not on in the stub");
    if (!(await page.getByTestId("captions-line").isVisible())) throw new Error("the newest line is not shown");
    await shot("show-captions");
  });
  await step("captions strip: switched off, then on again", async () => {
    await toggle.click();
    await settle(900);
    if ((await toggle.getAttribute("aria-checked")) !== "false") throw new Error("captions did not switch off");
    if (await page.getByTestId("captions-line").isVisible().catch(() => false)) throw new Error("a line is still shown while off");
    await shot("show-captions-off");
    await toggle.click();
    await settle(900);
    if ((await toggle.getAttribute("aria-checked")) !== "true") throw new Error("captions did not switch on again");
  });
}
