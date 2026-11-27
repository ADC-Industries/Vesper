// Shots of Site ▸ Displays (Vesper Mosaic; see scripts/shots.mjs for `ctx`) on the stub's
// synthetic site (test/stub/routes/mosaic.mjs): the grid of display screens with a conflict (a
// Vesper box whose Node also runs the display service), a screen picked (its feed, Identify, Test
// pattern, settings), a feed changed to a channel, the channels with a new one, this server's own
// displays (read-only, Configure Screens), and Site ▸ Devices with the boxes' Mosaic status.
// Video walls (round M2): the "Stage wall" card (its picture cut by the tiles, in step), the wall
// designer with two screens dragged into a new wall's cells, the new wall made, and a wall being
// identified (each tile's row and column), a wall's Delete asking first, and a wall whose
// display box left the site, its cells kept "(not listed now)". Batch 15 (M3): a box's address
// (from the site) and its display service, "Other machines of the site" with "Use as a display
// box", the channels' routers, and the site logo card.
import { screensStub } from "../../test/stub/routes/screens.mjs";

const ctl = (url, body) => fetch(`${url}/api/v1/__stub/mosaic`, { method: "POST", body: JSON.stringify(body) });

export default async function ({ page, openPage, settle, shot, step, stub }) {
  await ctl(stub.url, { reset: true });
  // This server's own displays (GET /displays, /outputs: the Screens stub).
  screensStub.start();
  try {
    await openPage("Site");
    await settle(300);
    await step("site devices: mosaic status", async () => {
      await page.getByTestId("device-mosaic").first().waitFor({ timeout: 5000 });
      await settle(200);
      await shot("site-devices-mosaic");
    });
    await step("site displays", async () => {
      await page.getByTestId("site-tab-displays").click();
      await page.getByTestId("mosaic-grid").waitFor({ timeout: 5000 });
      await settle(600);
      await shot("site-displays");
    });
    await step("site displays: a screen picked", async () => {
      await page.getByTestId("mosaic-tile").nth(2).click();
      await page.getByTestId("mosaic-detail").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-displays-screen");
      await page.getByTestId("mosaic-feed").selectOption({ label: "Channel: Main" });
      await page.getByTestId("mosaic-tile").nth(2).getByText("Channel Main").waitFor({ timeout: 5000 });
      await settle(500);
      await shot("site-displays-feed-changed");
    });
    await step("site displays: test pattern on all", async () => {
      await page.getByRole("button", { name: "Test pattern on all" }).click();
      await page.getByRole("button", { name: "Clear test patterns" }).waitFor({ timeout: 5000 });
      await settle(400);
      await shot("site-displays-test-pattern");
      await page.getByRole("button", { name: "Clear test patterns" }).click();
      await settle(300);
    });
    await step("site displays: video walls", async () => {
      const wall = page.getByTestId("mosaic-wall").first();
      await wall.waitFor({ timeout: 5000 });
      await wall.scrollIntoViewIfNeeded();
      await settle(600);
      await shot("site-displays-walls");
      await wall.getByRole("button", { name: "Identify" }).click();
      await settle(300);
      await shot("site-displays-wall-identify");
    });
    await step("site displays: the wall designer", async () => {
      await page.getByTestId("mosaic-new-wall").click();
      const d = page.getByTestId("mosaic-wall-designer");
      await d.waitFor({ timeout: 5000 });
      await d.getByLabel("Wall name").fill("Lobby wall");
      await d.getByLabel("Bezel between columns, mm").fill("12");
      await d.getByLabel("Screen's visible width, mm").fill("1210");
      await d.getByLabel("Screen's visible height, mm").fill("680");
      // Two screens dragged from the list into the cells.
      const chips = d.getByTestId("mosaic-wall-chip");
      const cells = d.getByTestId("mosaic-wall-cell");
      await page.getByTestId("mosaic-wall-cells").scrollIntoViewIfNeeded();
      await settle(300);
      for (const [n, cell] of [[1, 0], [2, 1]]) {
        await chips.filter({ hasText: `Lobby Display · ${n}` }).dragTo(cells.nth(cell));
        await chips.filter({ hasText: `Lobby Display · ${n}` }).getByText(`R1 C${cell + 1}`).waitFor({ timeout: 3000 });
      }
      await d.getByLabel("What the wall shows").selectOption({ label: "Channel: Lobby" });
      await d.scrollIntoViewIfNeeded();
      await settle(600);
      await shot("site-displays-wall-designer");
      await page.getByTestId("mosaic-wall-save").click();
      await page.getByTestId("mosaic-wall").filter({ hasText: "Lobby wall" }).waitFor({ timeout: 5000 });
      await page.getByTestId("mosaic-wall").filter({ hasText: "Lobby wall" }).scrollIntoViewIfNeeded();
      await settle(600);
      await shot("site-displays-wall-created");
      // Delete asks first (H08-6); Cancel keeps the wall.
      const made = page.getByTestId("mosaic-wall").filter({ hasText: "Lobby wall" });
      await made.getByTestId("mosaic-wall-delete").click();
      await made.getByTestId("mosaic-wall-delete-ask").waitFor({ timeout: 3000 });
      await settle(200);
      await shot("site-displays-wall-delete-ask");
      await made.getByTestId("mosaic-wall-delete-ask").getByRole("button", { name: "Cancel" }).click();
      await page.getByTestId("mosaic-grid").scrollIntoViewIfNeeded();
      await settle(300);
      await shot("site-displays-wall-tiles");
    });
    await step("site displays: a wall whose box is gone (R1-5)", async () => {
      // The Stage wall's box left the site: its cells keep their screens, "(not listed now)",
      // and Save keeps them.
      await ctl(stub.url, { boxGone: "fp-stagewall" });
      const wall = page.getByTestId("mosaic-wall").filter({ hasText: "Stage wall" });
      await wall.getByTestId("mosaic-wall-status").getByText("Offline").waitFor({ timeout: 5000 });
      await wall.getByRole("button", { name: "Edit" }).click();
      const d = page.getByTestId("mosaic-wall-designer");
      await d.waitFor({ timeout: 5000 });
      const pick = d.getByLabel("Row 1, column 1: screen");
      if ((await pick.inputValue()) !== "fp-stagewall/1") throw new Error(`R1 C1 lost its screen: ${await pick.inputValue()}`);
      await pick.locator("option:checked", { hasText: "Screen 1 (not listed now)" }).waitFor({ state: "attached", timeout: 3000 });
      await page.getByTestId("mosaic-wall-cells").scrollIntoViewIfNeeded();
      await settle(400);
      await shot("site-displays-wall-box-gone");
      await d.getByLabel("Bezel between rows, mm").fill("4");
      await page.getByTestId("mosaic-wall-save").click();
      await d.waitFor({ state: "detached", timeout: 5000 });
      await wall.getByText("2 × 2 · 4 screens · bezels 3.5 / 4 mm").waitFor({ timeout: 5000 });
      await ctl(stub.url, { reset: true });
    });
    await step("site displays: channels and this server's displays", async () => {
      await page.getByLabel("New channel name").fill("Overflow");
      await page.getByRole("button", { name: "Create channel" }).click();
      await page.getByTestId("mosaic-channels").getByText("Overflow", { exact: true }).waitFor({ timeout: 5000 });
      await page.getByTestId("mosaic-channels").scrollIntoViewIfNeeded();
      await settle(400);
      await shot("site-displays-channels");
      const own = page.getByTestId("mosaic-own");
      if (await own.count()) {
        await own.scrollIntoViewIfNeeded();
        await settle(200);
        await shot("site-displays-own");
      }
    });
    await step("site displays: a box's address and display service (M3)", async () => {
      await page.getByTestId("mosaic-grid").scrollIntoViewIfNeeded();
      // The screen picked earlier may still be selected: a click would close it.
      const svc = page.getByTestId("mosaic-box-service");
      if (!(await svc.isVisible())) await page.getByTestId("mosaic-tile").nth(2).click();
      await svc.waitFor({ timeout: 5000 });
      await page.getByTestId("mosaic-address-from").getByText("where the site sees its IDN Node").waitFor({ timeout: 3000 });
      await svc.scrollIntoViewIfNeeded();
      await settle(300);
      await shot("site-displays-box-service");
    });
    await step("site displays: other machines and the site logo (M3)", async () => {
      const machines = page.getByTestId("mosaic-machines");
      await machines.scrollIntoViewIfNeeded();
      await settle(300);
      await shot("site-displays-machines");
      await machines.getByRole("button", { name: "Use as a display box (headless)" }).click();
      await page.getByTestId("mosaic-waiting").getByText("Hall Mini PC").waitFor({ timeout: 5000 });
      const logo = page.getByTestId("mosaic-logo");
      await logo.scrollIntoViewIfNeeded();
      await logo.getByText("on 1 of 4 display boxes (1 sending)").waitFor({ timeout: 5000 });
      await settle(400);
      await shot("site-displays-logo");
    });
    await page.getByTestId("site-tab-devices").click();
    await settle(200);
  } finally {
    screensStub.stop();
    await ctl(stub.url, { reset: true });
  }
}
