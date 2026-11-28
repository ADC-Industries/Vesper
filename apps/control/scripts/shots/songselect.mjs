// Shots of Import ▸ Song from SongSelect… (see scripts/shots.mjs for `ctx`; the stub's
// routes/songselect.mjs reads the songs): the dialog with its download hint, three files chosen
// at once (a new-format .txt whose CCLI number is already in the library, flagged; an old-format
// one in UTF-16; a file with no lyrics, refused), the "Paste text" preview, and Reflow's smart
// paste (SongSelect lyrics pasted into a slide offer to become a new song) opening the dialog.
// Every song, writer and number here is made up. Nothing is imported.
import { reflowStub } from "../../test/stub/routes/reflow.mjs";

const NEW = `Lanterns Over Kettle Bay

Verse 1
Lanterns over Kettle Bay
Calling every sleeper home

Chorus
Rise and sing the harbour song
Rise and sing it all night long

Verse 2
Morning comes on silver wings

Alex Example | Jamie Sample
CCLI Song # 1234567
© 2019 Example Songs Publishing | Sample Music Co.
For use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com
CCLI License # 7654321
`;

const OLD = `Paper Boats

Verse 1
Paper boats upon the stream
Floating far away

Chorus
Sail on, sail on

Bridge
Every river finds the sea

CCLI Song # 7000001
Riley Placeholder | Casey Invented
© 2004 Imaginary Hymns
For use solely with the SongSelect Terms of Use. All rights reserved. www.ccli.com
CCLI License # 7654321
`;

const PASTE = `Quiet Harbour

Verse 1
Quiet harbour, evening tide
Lanterns burning side by side

Chorus
Hold on, hold on
Morning's coming soon

Morgan Sample | Taylor Example
CCLI Song # 7000002
© 2021 Example Songs Publishing
For use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com
CCLI License # 7654321
`;

const utf16 = (s) => Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(s, "utf16le")]);

export default async function ({ page, settle, shot, stub, step, esc }) {
  const open = async () => {
    await page.locator(".oriel .logo").click();
    await settle(150);
    await page.locator('[data-cat="3"]').hover();
    await settle(150);
    await page.getByRole("menu").getByRole("menuitem", { name: "Song from SongSelect…", exact: true }).click();
    await settle(400);
  };
  try {
    await step("songselect: the dialog", async () => {
      await open();
      await shot("songselect-dialog");
    });
    await step("songselect: three files", async () => {
      await page.getByTestId("songselect-files").setInputFiles([
        { name: "Lanterns Over Kettle Bay.txt", mimeType: "text/plain", buffer: Buffer.from(NEW) },
        { name: "Paper Boats.txt", mimeType: "text/plain", buffer: utf16(OLD) },
        { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("Just A Title\n") },
      ]);
      await page.getByTestId("songselect-list").locator("tbody tr").nth(2).waitFor({ timeout: 5000 });
      await settle(300);
      await shot("songselect-files");
    });
    await step("songselect: pasted lyrics", async () => {
      await page.getByRole("radio", { name: "Paste text" }).click();
      await page.getByTestId("songselect-text").fill(PASTE);
      await page.getByTestId("songselect-dialog").locator(".hint").click();
      await page.getByTestId("songselect-list").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("songselect-paste");
    });
  } finally {
    await esc();
  }

  // Reflow's smart paste.
  const changed = () => stub.push({ type: "documentChanged", kind: "presentation", id: reflowStub.id, change: "changed" });
  reflowStub.reset();
  reflowStub.on = true;
  try {
    changed();
    await settle(600);
    await page.keyboard.press("Control+r");
    await settle(1200);
    await step("reflow: smart paste", async () => {
      await page.locator("textarea[data-reflow-text]").first().evaluate((el, text) => {
        el.focus();
        const dt = new DataTransfer();
        dt.setData("text/plain", text);
        el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
      }, PASTE);
      await page.getByTestId("reflow-songselect").waitFor({ timeout: 3000 });
      await settle(200);
      await shot("reflow-songselect-paste");
      await page.getByTestId("reflow-songselect-import").click();
      await page.getByTestId("songselect-list").waitFor({ timeout: 5000 });
      await settle(300);
      await shot("reflow-songselect-dialog");
      await esc();
    });
  } finally {
    reflowStub.on = false;
    changed();
    await settle(400);
  }
}
