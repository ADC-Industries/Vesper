// Shots of the SongSelect Downloads watcher (desktop app; see scripts/shots.mjs for `ctx`): a
// second tab of the same browser context with a stand-in for the desktop shell
// (`__TAURI_INTERNALS__`: an empty Downloads folder at first, then a SongSelect lyrics file in
// it) and the watcher switched on, so the notice "New SongSelect song: …" shows; then its
// Import… opening the SongSelect import preview with the file, and Settings ▸ CCLI with the
// switch on. The song is made up. Nothing is imported.

const SONG = `Lanterns Over Kettle Bay

Verse 1
Lanterns over Kettle Bay
Calling every sleeper home

Chorus
Rise and sing the harbour song
Rise and sing it all night long

Alex Example | Jamie Sample
CCLI Song # 1234567
© 2019 Example Songs Publishing | Sample Music Co.
For use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com
CCLI License # 7654321
`;

export default async function ({ page, stub, step, shotOf }) {
  const tab = await page.context().newPage();
  const bytes = Array.from(Buffer.from(SONG, "utf8"));
  await tab.addInitScript(
    ({ bytes }) => {
      localStorage.setItem("oriel.songselectWatch", "1");
      const since = Date.now() - 60_000;
      let lists = 0;
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
        transformCallback: () => 0,
        invoke: async (cmd, args) => {
          if (cmd === "downloads_list") {
            lists += 1;
            return lists === 1 ? [] : [{ name: "Lanterns Over Kettle Bay.txt", size: bytes.length, modifiedMs: since }];
          }
          if (cmd === "downloads_song") return { name: args.name, title: "Lanterns Over Kettle Bay", data: bytes };
          if (cmd === "discover_servers") return [];
          throw new Error(`${cmd}: not in the shots`);
        },
      };
    },
    { bytes },
  );
  const settle = async (ms) => {
    await tab.waitForLoadState("networkidle").catch(() => {});
    await new Promise((r) => setTimeout(r, ms));
  };
  try {
    await tab.goto(stub.url + "/");
    await step("songselect watcher: the notice", async () => {
      await tab.getByTestId("songselect-download-notice").waitFor({ timeout: 15_000 });
      await settle(300);
      await shotOf(tab, "songselect-download-notice");
    });
    await step("songselect watcher: Import…", async () => {
      await tab.getByTestId("songselect-download-notice").getByRole("button", { name: "Import…" }).click();
      await tab.getByTestId("songselect-list").locator("tbody tr").first().waitFor({ timeout: 5000 });
      await settle(400);
      await shotOf(tab, "songselect-download-import");
      await tab.keyboard.press("Escape");
      await settle(200);
    });
    await step("songselect watcher: Settings ▸ CCLI", async () => {
      await tab.keyboard.press("Control+Comma");
      await settle(500);
      await tab.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "CCLI", exact: true }).click();
      await settle(500);
      await shotOf(tab, "settings-ccli-desktop");
    });
  } finally {
    await tab.close();
  }
}

