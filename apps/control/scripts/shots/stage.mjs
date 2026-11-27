// Shots of the Stage page (see scripts/shots.mjs for `ctx`): the stage screen with its layout
// and live picture (a stage-style picture stands in for the stub's preview), the stage message
// up (test/stub/routes/stage.mjs), then "Next slide" and "Timer" checked on the layout.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { slideSvg } from "../../test/stub/server.mjs";
import { stageStub } from "../../test/stub/routes/stage.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

const STAGE_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360" viewBox="0 0 1920 1080">
<rect width="1920" height="1080" fill="#000"/>
<text x="40" y="110" font-family="DejaVu Sans, sans-serif" font-weight="700" font-size="84" fill="#fff">10:42 AM</text>
<text x="1880" y="110" text-anchor="end" font-family="DejaVu Sans, sans-serif" font-weight="700" font-size="84" fill="#7fd39a">28:15</text>
<text x="960" y="430" text-anchor="middle" font-family="DejaVu Sans, sans-serif" font-weight="700" font-size="110" fill="#fff">Golden fields sing</text>
<text x="960" y="570" text-anchor="middle" font-family="DejaVu Sans, sans-serif" font-weight="700" font-size="110" fill="#fff">Rise with the morning</text>
<rect x="0" y="918" width="1920" height="4" fill="#8c8c8c"/>
<rect x="40" y="960" width="1840" height="96" fill="#111"/>
<text x="960" y="1030" text-anchor="middle" font-family="DejaVu Sans, sans-serif" font-weight="700" font-size="60" fill="#fff">Bridge twice, then out</text>
</svg>`;

async function jpeg(page, html) {
  const p = await page.context().newPage();
  await p.setViewportSize({ width: 640, height: 360 });
  await p.setContent(`<body style="margin:0;background:#000">${html}</body>`);
  const b = await p.screenshot({ type: "jpeg", quality: 85 });
  await p.close();
  return b;
}

/** The audience picture shots.mjs starts with (the fixtures' live slide). */
async function audienceJpeg(page) {
  const state = JSON.parse(readFileSync(join(ROOT, "test/stub/fixtures/ws__state.json"), "utf8")).state;
  const live = state.layers.slide;
  const doc = JSON.parse(readFileSync(join(ROOT, `test/stub/fixtures/presentations__${live.presentation}.json`), "utf8"));
  const slide = doc.sections.flatMap((s) => s.slides).find((s) => s.id === live.slide);
  const lines = slide.elements.flatMap((e) => e.kind?.text?.paragraphs ?? []).map((p) => p.runs.map((r) => r.text).join(""));
  return jpeg(page, slideSvg(lines, "#2F5FB8", 640, 360));
}

export default async function ({ page, openPage, settle, shot, step, stub }) {
  stub.setPreviewJpeg(await jpeg(page, STAGE_SVG));
  stageStub.start();
  try {
    await settle(300);
    await openPage("Stage");
    await settle(800);
    await shot("page-stage");
    await step("stage compose", async () => {
      await page.getByTestId("stage-el-next").check();
      await settle(500);
      await page.getByTestId("stage-el-timer").check();
      await settle(700);
      await shot("stage-compose");
    });
  } finally {
    stageStub.stop();
    stub.setPreviewJpeg(await audienceJpeg(page));
    await settle(300);
  }
}
