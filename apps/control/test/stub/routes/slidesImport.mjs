// PowerPoint import's stub routes (see server.mjs's header for the interface): a `.pptx` posted
// to `/codex/imports/pptx` becomes a job that moves on one step per `GET` (queued → reading →
// pictures → media → saving → done) and ends with a synthetic report (made-up slide titles).
// A `.key` or `.ppt` file name is refused as the server does.
//
// Stub control (not part of the API): `POST /api/v1/__stub/pptx` with
//   {"hold": true}    the job stays where it is (for the progress shot)
//   {"hold": false}   it moves on again
//   {"reset": true}   no jobs

const STEPS = [
  ["queued", 0, 0],
  ["reading", 2, 8],
  ["reading", 5, 8],
  ["pictures", 0, 2],
  ["media", 3, 6],
  ["saving", 0, 0],
  ["done", 0, 0],
];

const TITLES = ["Spring Retreat", "Welcome", "Schedule", "Attendance so far", "Small groups", "Budget", "Meals and rooms", "Questions"];

function report(name) {
  const slides = TITLES.map((title, i) => ({ number: i + 1, title, editable: 3 + (i % 3) }));
  slides[3].pictured = [{ kind: "chart", name: "Attendance", detail: "drawn from the slide's picture" }];
  slides[3].picture = "libreOffice";
  slides[5].pictured = [{ kind: "table", name: "Budget table", detail: "drawn from the slide's picture" }];
  slides[5].picture = "libreOffice";
  // Two identical notes on one slide (two ink drawings): the report lists both.
  slides[6].notes = [
    { kind: "video", name: "Walkthrough", detail: "video or sound not imported (its poster picture is)" },
    { kind: "ink", detail: "ink drawing not imported" },
    { kind: "ink", detail: "ink drawing not imported" },
  ];
  slides[7].hidden = true;
  return {
    presentation: "5a1f0c2e-7b3d-4e8f-9a01-23456789abcd",
    name,
    library: "8c2e4f60-1a3b-4c5d-8e7f-0123456789ef",
    width: 1920,
    height: 1080,
    slides,
    editableSlides: 6,
    picturedSlides: 2,
    media: 6,
    renderer: "/usr/bin/soffice",
  };
}

export default function slidesImportRoutes(ctx) {
  const { json, apiError } = ctx;
  let jobs = [];
  let hold = false;
  let seq = 0;

  const view = (j) => {
    const [state, done, total] = STEPS[j.step];
    return {
      id: j.id,
      collection: j.collection,
      fileName: j.fileName,
      state,
      done,
      total,
      createdAt: j.createdAt,
      ...(state === "done" ? { report: report(j.name) } : {}),
    };
  };

  return function handle(req, res, { path, query }) {
    const m = req.method;
    if (path === "/__stub/pptx" && m === "POST") {
      let b = "";
      req.on("data", (d) => (b += d));
      req.on("end", () => {
        const v = b ? JSON.parse(b) : {};
        if ("hold" in v) hold = !!v.hold;
        if (v.reset) jobs = [];
        json(res, 200, { ok: true });
      });
      return true;
    }
    if (path === "/codex/imports/pptx" && m === "GET") {
      json(res, 200, [...jobs].reverse().map(view));
      return true;
    }
    if (path === "/codex/imports/pptx" && m === "POST") {
      // Drain the body (the file), then answer.
      req.on("data", () => {});
      req.on("end", () => {
        const fileName = query.get("fileName") ?? "presentation.pptx";
        const ext = fileName.split(".").pop().toLowerCase();
        if (ext === "key") return apiError(res, 400, "badRequest", "a Keynote file: in Keynote choose File ▸ Export To ▸ PowerPoint, then import the .pptx");
        if (ext === "ppt") return apiError(res, 400, "badRequest", "an old PowerPoint file (.ppt): open it in PowerPoint and save it as .pptx first");
        const name = query.get("name") || fileName.replace(/\.[^.]+$/, "");
        const j = { id: `job-${++seq}`, collection: query.get("collection") ?? "c-main", fileName, name, step: 0, createdAt: new Date().toISOString() };
        jobs.push(j);
        json(res, 202, view(j));
      });
      return true;
    }
    const mm = /^\/codex\/imports\/pptx\/([^/]+)$/.exec(path);
    if (mm && m === "GET") {
      const j = jobs.find((x) => x.id === decodeURIComponent(mm[1]));
      if (!j) return apiError(res, 404, "notFound", "no such import"), true;
      if (!hold && j.step < STEPS.length - 1) j.step++;
      json(res, 200, view(j));
      return true;
    }
    return false;
  };
}
