// Shots of the Site page (see scripts/shots.mjs for `ctx`) on the synthetic two-box site of
// test/stub/routes/site.mjs: each tab (Devices with a device's services to restart, its logs
// and the Remove confirmation, a Vesper box relaying pairings (its Relay off / on), Join requests with a machine and an Oriel computer waiting, Codex with an
// export in progress and an import preview, Backups with a restore preview, also onto this box,
// Security), the unlock prompt on a 403 at access level "pin", and the warning while the Site
// Manager runs without its admin panel (its port taken), and the warning after a start from
// damaged site settings (Open Security, Keep the access).
const ctl = (url, body) => fetch(`${url}/api/v1/__stub/site`, { method: "POST", body: JSON.stringify(body) });

export default async function ({ page, openPage, settle, shot, step, stub }) {
  await ctl(stub.url, { reset: true });
  try {
    await openPage("Site");
    await settle(500);
    await shot("page-site");
    const tab = async (id) => {
      await page.getByTestId(`site-tab-${id}`).click();
      await settle(400);
    };
    await step("site devices: restart a service", async () => {
      // The device's own row (the services row under it names the device too, and has a
      // Restart button per service).
      const row = page.getByTestId("site-devices").locator("tr", { hasText: "Lobby Display" }).first();
      await row.getByRole("button", { name: "Restart" }).click();
      await settle(300);
      await shot("site-devices-restart");
      await row.getByRole("button", { name: "Restart" }).click();
      await settle(200);
    });
    await step("site devices: relays pairings", async () => {
      // Chapel Box, a Vesper box, relays pairings (the site's server marks Vesper boxes; an
      // output box like Lobby Display never is). An admin turns it off and on again by hand.
      const row = page.getByTestId("site-devices").locator("tr", { hasText: "Chapel Box" }).first();
      await row.getByText("Relays pairings").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-devices-relay");
      await row.getByRole("button", { name: "Relay off" }).click();
      await row.getByRole("button", { name: "Relay on" }).waitFor({ timeout: 5000 });
      await row.getByRole("button", { name: "Relay on" }).click();
      await row.getByRole("button", { name: "Relay off" }).waitFor({ timeout: 5000 });
      // Only a Vesper box is offered the toggle (M-6): never an output box.
      const lobby = page.getByTestId("site-devices").locator("tr", { hasText: "Lobby Display" }).first();
      if (await lobby.getByRole("button", { name: /^Relay/ }).count()) throw new Error("an output box is offered Relay on");
    });
    await step("site devices: logs and remove", async () => {
      const row = page.getByTestId("site-devices").locator("tr", { hasText: "Lobby Display" }).first();
      await row.getByRole("button", { name: "Logs" }).click();
      await settle(300);
      await page.getByRole("button", { name: /node\.log/ }).first().click();
      await settle(300);
      await shot("site-devices-logs");
      await row.getByRole("button", { name: "Remove from site" }).click();
      await settle(200);
      await shot("site-devices-remove");
    });
    await step("site join requests", async () => {
      await tab("join");
      await shot("site-join");
    });
    await step("site codex", async () => {
      await tab("codex");
      await shot("site-codex");
      await ctl(stub.url, { exportDelay: 4000 });
      const card = page.getByTestId("export-card");
      await card.locator('input[type="password"]').nth(0).fill("blue lantern 42");
      await card.locator('input[type="password"]').nth(1).fill("blue lantern 42");
      await card.locator('input[type="checkbox"]').check();
      await card.getByRole("button", { name: "Download export" }).click();
      await settle(400);
      await shot("site-codex-exporting");
      await settle(4000);
      await ctl(stub.url, { exportDelay: 0 });
    });
    await step("site import preview", async () => {
      await page.getByTestId("import-file").setInputFiles({ name: "vesper-codex-2026-11-22.vcodex", mimeType: "application/octet-stream", buffer: Buffer.from("VCODEX stub file") });
      await settle(400);
      const card = page.getByTestId("import-card");
      await card.locator('input[type="password"]').fill("blue lantern 42");
      await card.getByRole("button", { name: "Open" }).click();
      await settle(400);
      await card.scrollIntoViewIfNeeded();
      await shot("site-codex-import");
      await card.getByRole("button", { name: "Cancel" }).click();
      await settle(200);
    });
    await step("site backups and a restore preview", async () => {
      await tab("backups");
      await page.getByTestId("backup-sets").locator("tr", { hasText: "Chapel Box" }).getByRole("button", { name: "Restore…" }).click();
      await settle(500);
      await shot("site-backups-restore");
    });
    await step("site restore onto this box (a replacement)", async () => {
      // The preview follows the box restored onto (R1-2): this box's own changes.
      await page.getByTestId("restore-target").selectOption({ index: 1 });
      await page.getByText("1 output binding removed", { exact: false }).waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-backups-restore-onto");
    });
    await step("site security", async () => {
      await tab("security");
      await shot("site-security");
    });
    await step("site unlock prompt", async () => {
      await ctl(stub.url, { access: "pin" });
      await page.getByRole("button", { name: "New code" }).click();
      await settle(400);
      await shot("site-unlock");
      await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();
      await settle(200);
    });
    await step("site join mode after a cancelled unlock", async () => {
      // The radios show the site's mode again (A16-10).
      await tab("join");
      await page.getByRole("radio", { name: "Approval only" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();
      await settle(400);
      if (!(await page.getByRole("radio", { name: "Site code or approval" }).isChecked())) throw new Error("the join mode radios show a mode the site doesn't use");
    });
    await tab("devices");
    await step("site running without its admin panel", async () => {
      await ctl(stub.url, { access: "open", apiError: "Address already in use (os error 98) (API port 29202)" });
      await openPage("Library");
      await openPage("Site");
      await page.getByTestId("site-api-error").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-api-error");
      await ctl(stub.url, { apiError: null });
    });
    await step("site started from damaged settings", async () => {
      // A paired computer that only views: the warning, with the way to Security.
      await ctl(stub.url, { access: "users", settingsDamaged: true });
      await openPage("Library");
      await openPage("Site");
      await page.getByTestId("site-settings-damaged").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-settings-damaged");
      await page.getByTestId("site-settings-damaged").getByRole("button", { name: "Open Security" }).click();
      await page.getByTestId("site-keep-access").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-settings-damaged-security");
      // An admin keeps the access: the warning goes.
      await ctl(stub.url, { access: "open", settingsDamaged: true });
      await openPage("Library");
      await openPage("Site");
      await page.getByTestId("site-keep-access").getByRole("button", { name: "Keep Open" }).click();
      await page.getByTestId("site-settings-damaged").waitFor({ state: "detached", timeout: 5000 });
      await settle(300);
      await shot("site-settings-damaged-cleared");
    });
  } finally {
    await ctl(stub.url, { reset: true });
  }
}
