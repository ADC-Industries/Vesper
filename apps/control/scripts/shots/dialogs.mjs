// Shots of the shared dialogs (see scripts/shots.mjs for `ctx`): Diagnostics (Oriel menu ▸
// Help), a name prompt (File ▸ New Presentation…), a playlist header prompt and a confirmation
// (a playlist item's context menu). Each is cancelled with Esc, so nothing changes. The edit
// conflict panel is in reflow.mjs.
export default async function ({ page, settle, shot, step, esc }) {
  const menuItem = async (pane, name) => {
    await page.locator(".oriel .logo").click();
    await settle(150);
    await page.locator(`[data-cat="${pane}"]`).hover();
    await settle(150);
    await page.getByRole("menu").getByRole("menuitem", { name, exact: true }).click();
    await settle(500);
  };
  const itemMenu = async (name) => {
    await page.getByRole("list", { name: "Playlist items" }).getByText("Morning Lantern").click({ button: "right" });
    await settle(200);
    await page.getByRole("menuitem", { name, exact: true }).click();
    await settle(400);
  };

  await step("diagnostics", async () => {
    await menuItem(7, "Diagnostics…");
    await shot("dialog-diagnostics");
  });
  await esc();
  await step("name prompt", async () => {
    await menuItem(0, "New Presentation…");
    await shot("dialog-prompt-name");
  });
  await esc();
  await step("header prompt", async () => {
    await itemMenu("Add Header After…");
    await shot("dialog-prompt-header");
  });
  await esc();
  await step("confirmation", async () => {
    await itemMenu("Remove from Playlist…");
    await shot("dialog-confirm");
  });
  await esc();
}
