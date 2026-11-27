// Shots of Import ▸ PowerPoint / Keynote / Google Slides (see scripts/shots.mjs for `ctx`; the
// stub's routes/slidesImport.mjs): the dialog with its "export as .pptx first" hint, a Keynote
// file refused, a .pptx chosen, the import in progress and the report (made-up slide titles).
// Closed with Esc at the end; nothing changes in the stub's library.
const ctl = (url, body) => fetch(`${url}/api/v1/__stub/pptx`, { method: "POST", body: JSON.stringify(body) });

export default async function ({ page, stub, settle, shot, step, esc }) {
  await ctl(stub.url, { reset: true, hold: false });
  const open = async () => {
    await page.locator(".oriel .logo").click();
    await settle(150);
    await page.locator('[data-cat="3"]').hover();
    await settle(150);
    await page.getByRole("menu").getByRole("menuitem", { name: "PowerPoint / Keynote / Google Slides…", exact: true }).click();
    await settle(500);
  };
  const input = () => page.locator('.ui-dialog input[type="file"]');
  const pk = Buffer.concat([Buffer.from("PK\u0003\u0004"), Buffer.alloc(40_000, 7)]);
  try {
    await step("import pptx: the dialog", async () => {
      await open();
      await shot("import-pptx");
    });
    await step("import pptx: a Keynote file", async () => {
      await input().setInputFiles({ name: "Spring Retreat.key", mimeType: "application/octet-stream", buffer: pk });
      await settle(200);
      await shot("import-pptx-keynote");
    });
    await step("import pptx: a .pptx chosen", async () => {
      await input().setInputFiles({ name: "Spring Retreat.pptx", mimeType: "application/vnd.openxmlformats-officedocument.presentationml.presentation", buffer: pk });
      await settle(200);
      await shot("import-pptx-chosen");
    });
    await step("import pptx: in progress", async () => {
      await page.getByRole("button", { name: "Import", exact: true }).click();
      // One step on, then held there.
      await settle(500);
      await ctl(stub.url, { hold: true });
      await settle(900);
      await shot("import-pptx-progress");
    });
    await step("import pptx: the report", async () => {
      await ctl(stub.url, { hold: false });
      await page.getByRole("list", { name: "Imported slides" }).waitFor({ timeout: 10_000 });
      await settle(300);
      await shot("import-pptx-report");
    });
  } finally {
    await ctl(stub.url, { hold: false });
    await esc();
  }
}
