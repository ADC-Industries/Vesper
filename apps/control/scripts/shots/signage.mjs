// Shots of Site ▸ Signage (docs/SIGNAGE-PLAN.md; see scripts/shots.mjs for `ctx`) on the stub's
// synthetic site (test/stub/routes/signage.mjs): the channels with their pictures, now/next and
// health, the Lobby's layout and week calendar (a slot past midnight, the now line), a slot
// picked, a slot made by dragging down a day, the playlist editor, the feeds with a preview and
// column mapping, the weather and service times, the Override dialog (confirm) and the override
// showing, and Mosaic's source picker naming the signage channels.

const ctl = (url, body) => fetch(`${url}/api/v1/__stub/signage`, { method: "POST", body: JSON.stringify(body) });

export default async function ({ page, openPage, settle, shot, step, stub }) {
  await ctl(stub.url, { reset: true });
  try {
    await openPage("Site");
    await settle(300);
    await step("site signage", async () => {
      await page.getByTestId("site-tab-signage").click();
      await page.getByTestId("signage-channels").waitFor({ timeout: 5000 });
      await page.getByTestId("signage-calendar").waitFor({ timeout: 5000 });
      await settle(600);
      await shot("site-signage");
    });
    await step("site signage: a slot", async () => {
      await page.getByTestId("signage-calendar").scrollIntoViewIfNeeded();
      await page.getByTestId("signage-block").first().click();
      await page.getByTestId("signage-slot").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-signage-calendar");
      // Drag down Tuesday from about 06:00 to 09:00: a new slot.
      const col = page.getByTestId("signage-day").nth(1);
      const box = await col.boundingBox();
      if (box) {
        await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.25);
        await page.mouse.down();
        await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.3, { steps: 4 });
        await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.375, { steps: 4 });
        await page.mouse.up();
      }
      await page.getByTestId("signage-block").nth(5).waitFor({ timeout: 5000 });
      await settle(400);
      await shot("site-signage-slot-made");
    });
    await step("site signage: playlists", async () => {
      const p = page.getByTestId("signage-playlists");
      await p.scrollIntoViewIfNeeded();
      await p.getByTestId("signage-item").first().waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-signage-playlists");
    });
    await step("site signage: feeds and settings", async () => {
      const f = page.getByTestId("signage-feeds");
      await f.scrollIntoViewIfNeeded();
      await f.getByRole("button", { name: "Add feed" }).click();
      const form = page.getByTestId("signage-feed-form");
      await form.getByPlaceholder("Events").fill("Calendar");
      await form.getByPlaceholder(/docs.google.com/).fill("https://calendar.example.org/events.csv");
      await form.getByRole("button", { name: "Preview" }).click();
      await page.getByTestId("signage-feed-preview").waitFor({ timeout: 5000 });
      await form.getByLabel("Title column").selectOption("Title");
      await form.getByLabel("When column").selectOption("Date");
      await page.getByTestId("signage-settings").scrollIntoViewIfNeeded();
      await settle(300);
      await shot("site-signage-feeds");
      await form.getByRole("button", { name: "Cancel" }).click();
    });
    await step("site signage: override", async () => {
      await page.getByTestId("signage-override").click();
      const d = page.getByTestId("signage-override-dialog");
      await d.waitFor({ timeout: 5000 });
      await d.getByRole("textbox").fill("Severe weather: please move to the hallways away from windows");
      await page.getByRole("button", { name: "Override…" }).click();
      await page.getByTestId("signage-override-go").waitFor({ timeout: 5000 });
      await settle(200);
      await shot("site-signage-override-confirm");
      await page.getByTestId("signage-override-go").click();
      await page.getByTestId("signage-override-on").waitFor({ timeout: 5000 });
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.getByTestId("signage-override-on").scrollIntoViewIfNeeded();
      await settle(800);
      await shot("site-signage-override-on");
      await page.getByTestId("signage-override-clear").click();
      await page.getByTestId("signage-override-on").waitFor({ state: "detached", timeout: 5000 });
    });
    await step("site displays: signage sources", async () => {
      await page.getByTestId("site-tab-displays").click();
      await page.getByTestId("mosaic-grid").waitFor({ timeout: 5000 });
      await page.getByTestId("mosaic-tile").first().click();
      const feed = page.getByTestId("mosaic-feed");
      await feed.waitFor({ timeout: 5000 });
      await feed.locator("option", { hasText: "Signage: Lobby" }).waitFor({ state: "attached", timeout: 5000 });
      await settle(200);
      await shot("site-displays-signage-sources");
    });
    await page.getByTestId("site-tab-devices").click();
    await settle(200);
  } finally {
    await ctl(stub.url, { reset: true });
  }
}
