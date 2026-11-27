// Shots of media preparation and the CCLI usage report (docs/MEDIA-PREP-PLAN.md; see
// scripts/shots.mjs for `ctx`): the Library's Videos with preparation badges (one converting, one
// failed), a converted video's Preparation card and trim bar, the failed one, Settings ▸ Media
// (with something live: conversions go on at the lowest priority; the encoders this box has;
// then "wait until nothing is live" chosen and the page scrolled to its end; then HEVC with the
// CPU's encoder chosen, which this box has none for) and Settings ▸
// Reports (CCLI usage). The stub's media preparation is test/stub/routes/codex-prep.mjs.
import { prepStub } from "../../test/stub/routes/codex-prep.mjs";

export default async function ({ page, openPage, shot, settle, step, esc }) {
  await openPage("Library");
  await settle(800);
  const where = page.getByRole("navigation", { name: "Library" });
  const node = (name) => where.locator(".node", { hasText: name }).first();
  const tile = (name) => page.locator(".what .tile", { hasText: name }).first();

  await step("a converted video", async () => {
    await node("Videos").click();
    await settle(800);
    await tile("Particles Rising").click();
    await settle(800);
    await page.mouse.move(5, 300);
    await shot("page-library-media-prep");
  });

  await step("a video whose preparation failed", async () => {
    await tile("Ocean Shimmer").click();
    await settle(600);
    await page.mouse.move(5, 300);
    await shot("page-library-media-prep-failed");
  });

  await step("a trim being dragged", async () => {
    await tile("Light Leaks").click();
    await settle(600);
    const handle = page.getByRole("button", { name: "Out point" });
    const box = await handle.boundingBox();
    if (!box) throw new Error("no out point handle");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x - 60, box.y + box.height / 2, { steps: 6 });
    await page.mouse.up();
    await settle(300);
    await shot("page-library-media-trim");
    await page.getByTestId("media-trim").getByRole("button", { name: "Cancel" }).click();
    await settle(200);
  });

  await step("back to All songs", async () => {
    await node("All songs").click();
    await settle(300);
  });
  await esc();

  await page.keyboard.press("Control+Comma");
  await settle(600);
  try {
    const nav = page.getByRole("navigation", { name: "Settings sections" });
    await step("settings media", async () => {
      prepStub.live = true;
      await nav.getByRole("button", { name: "Media", exact: true }).click();
      await settle(2400);
      await shot("settings-media");
      await page.getByLabel("While live").selectOption("wait");
      await page.getByLabel("Bit rate").fill("25");
      await settle(300);
      await page.getByTestId("media-prep-box").scrollIntoViewIfNeeded();
      await settle(300);
      await shot("settings-media-wait");
      // HEVC with the CPU's encoder, not saved: this box has none for it (the codec being
      // edited, not the saved one).
      await page.getByLabel("Codec").selectOption("hevc");
      await page.getByLabel("Encoder", { exact: true }).selectOption("cpu");
      await settle(300);
      await page.getByTestId("media-prep-encoders").scrollIntoViewIfNeeded();
      await settle(300);
      await shot("settings-media-hevc-cpu");
    });
    await step("settings reports", async () => {
      await nav.getByRole("button", { name: "Reports", exact: true }).click();
      await settle(800);
      await shot("settings-reports-ccli");
    });
  } finally {
    prepStub.live = false;
    await page.getByRole("toolbar", { name: "Settings" }).getByRole("button", { name: "Close" }).click();
    await settle(200);
  }
}
