// Shots of Site ▸ Updates (G10; see scripts/shots.mjs for `ctx`) on the synthetic site of
// test/stub/routes/updates.mjs: three machines' Launchers (an update available, one that can
// roll back, one offline after an automatic rollback) and the update channel's packages; then
// an update queued, "Add app" on a display box and an app queued, the GitHub releases check, a
// signed package uploaded, the queued actions done, and a delete asked.
const ctl = (url, body) => fetch(`${url}/api/v1/__stub/updates`, { method: "POST", body: JSON.stringify(body) });

const SIG = [
  "untrusted comment: signature from minisign secret key",
  "RUQAAAAAAAAAAA==",
  "trusted comment: vesper-package product=vesper version=0.3.0 platform=linux-x86_64",
  "AAAA",
  "",
].join("\n");

export default async function ({ page, openPage, settle, shot, step, stub }) {
  await ctl(stub.url, { reset: true });
  try {
    await openPage("Site");
    await page.getByTestId("site-tab-updates").click();
    await page.getByTestId("updates-machines").waitFor({ timeout: 5000 });
    await settle(400);
    await shot("site-updates");
    await step("site updates: queue an update", async () => {
      await page.getByTestId("updates-machines").getByRole("button", { name: "Update to 0.2.0" }).first().click();
      await page.getByText("Update to 0.2.0 queued").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-updates-queued");
    });
    await step("site updates: add an app to a display box", async () => {
      await page.getByRole("button", { name: "Add app to Chapel Box…" }).click();
      await page.getByTestId("updates-catalogue").getByRole("button", { name: "Add Vesper Oriel" }).waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-updates-add-app");
      await page.getByTestId("updates-catalogue").getByRole("button", { name: "Add Vesper Oriel" }).click();
      await page.getByText("Adding Vesper Oriel 0.2.0 (queued)").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-updates-add-app-queued");
    });
    await step("site updates: releases from GitHub", async () => {
      await page.getByTestId("updates-github").scrollIntoViewIfNeeded();
      await page.getByTestId("updates-github-status").getByText("Last checked").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("site-updates-github");
    });
    await step("site updates: upload a signed package", async () => {
      await page.getByTestId("updates-package-file").setInputFiles({ name: "vesper_0.3.0-1_amd64.deb", mimeType: "application/octet-stream", buffer: Buffer.from("synthetic package") });
      await page.getByTestId("updates-signature-file").setInputFiles({ name: "vesper_0.3.0-1_amd64.deb.minisig", mimeType: "text/plain", buffer: Buffer.from(SIG) });
      await page.getByTestId("updates-upload").getByRole("button", { name: "Upload" }).click();
      await page.getByTestId("updates-packages").getByText("0.3.0").first().waitFor({ timeout: 5000 });
      await settle(300);
      await page.getByTestId("updates-packages").scrollIntoViewIfNeeded();
      await shot("site-updates-uploaded");
    });
    await step("site updates: the Launchers did it; delete asked", async () => {
      await ctl(stub.url, { apply: true });
      // The tab reads again every 5 s: the queued update is done.
      await page.getByText("Update to 0.2.0 queued").waitFor({ state: "detached", timeout: 8000 });
      await page.getByTestId("updates-packages").getByRole("button", { name: "Delete…" }).last().click();
      await settle(300);
      await shot("site-updates-applied");
    });
  } finally {
    await ctl(stub.url, { reset: true });
  }
}
