// Media preparation and CCLI usage reports (docs/MEDIA-PREP-PLAN.md), for the Library's media
// item, its tiles, Settings ▸ Media and Settings ▸ Reports: the library stub's videos get their
// preparation (one converted with a trim, one analysed, one converting, one failed), the jobs and
// settings of `GET/PUT /codex/media-prep`, `POST …/media/{asset}/prepare`, `PUT …/trim`, `POST
// …/original`, and a made-up CCLI report (`GET /reports/ccli`). Named to sort before
// `library.mjs`, whose routes take every other `…/media/…` write.

import { stubCodex } from "./library.mjs";

export const prepStub = { live: false };

/** The stub box's encoders per codec: a GPU's for both, a CPU one only for H.264 (as Ubuntu's
 *  FFmpeg). */
const ENCODERS = { h264: { gpu: "VA-API", cpu: "OpenH264 (CPU)" }, hevc: { gpu: "VA-API" } };

const body = (req) =>
  new Promise((resolve) => {
    let b = "";
    req.on("data", (d) => (b += d));
    req.on("end", () => {
      try {
        resolve(JSON.parse(b || "{}"));
      } catch {
        resolve({});
      }
    });
  });

const PREP = {
  "Particles Rising": {
    loudness: -21.4,
    peak: -6.2,
    gainDb: 5.4,
    posterAt: 2,
    trimIn: 1.5,
    trimOut: 18,
    analysed: "5eed",
    at: "2026-11-25T18:04:00Z",
    original: {
      location: { type: "library", id: "00000000000000a1", sha256: "0".repeat(64) },
      probe: { width: 1920, height: 1080, duration: 20, frameRate: { num: 30000, den: 1001 }, codec: "prores", hasAudio: true },
      reasons: ["ProRes video isn't decoded by the GPU", "sound at 44.1 kHz (the house rate is 48 kHz)"],
      format: "H.264 1920×1080 29.97 fps, AAC 48 kHz",
    },
  },
  "Light Leaks": { loudness: -14.2, peak: -1.1, gainDb: -1.8, posterAt: 1.8, analysed: "5eee", at: "2026-11-25T18:05:00Z" },
  "Ocean Shimmer": { error: "converting Ocean Shimmer.mp4: Invalid data found when processing input", at: "2026-11-25T18:06:00Z" },
};

export default function codexPrepRoutes({ json, apiError }) {
  /** The library stub's state last decorated (its reset makes a new one). */
  let decorated = null;
  const DEFAULTS = {
    enabled: true,
    transcode: true,
    video: "h264",
    loudnessTarget: -16,
    maxJobs: 1,
    whileLive: "lowPriority",
    priority: "low",
    quality: "standard",
    maxSize: "keep",
    encoder: "auto",
  };
  let settings = { ...DEFAULTS };
  let jobs = [];

  /** The active collection's media (the library stub's), with their preparation added once. */
  function media() {
    const c = stubCodex();
    if (!c) return null;
    const list = c.content[c.active]?.media ?? [];
    if (decorated !== c) {
      decorated = c;
      jobs = [];
      for (const m of list) {
        const name = (m.asset.name ?? "").replace(/\.[^.]+$/, "");
        if (PREP[name]) m.asset.prep = structuredClone(PREP[name]);
        if (name === "Particles Rising") m.asset.probe = { ...m.asset.probe, codec: "h264", frameRate: { num: 30000, den: 1001 }, hasAudio: true };
        if (name === "Slow Clouds") jobs.push({ asset: m.asset.id, collection: c.active, name: m.asset.name, step: "converting", progress: 0.42 });
        if (name === "Ocean Shimmer") jobs.push({ asset: m.asset.id, collection: c.active, name: m.asset.name, step: "failed", progress: 0, error: PREP[name].error });
      }
    }
    return list;
  }

  return (req, res, { path, parts, query }) => {
    const list = media();
    const m = req.method;
    if (path === "/codex/media-prep" && m === "GET") {
      return (
        json(res, 200, {
          settings,
          jobs: jobs.map((j) => (prepStub.live && settings.whileLive === "wait" && j.step === "converting" ? { ...j, step: "waitingForLive", progress: 0 } : j)),
          converter: "IDN media conversion, FFmpeg 8.0.1 (libavcodec 62.11.100, libavformat 62.3.100)",
          encoder: prepStub.live && settings.encoder === "auto" ? "OpenH264 (CPU)" : settings.encoder === "cpu" ? "OpenH264 (CPU)" : "VA-API",
          gpuEncoder: ENCODERS[settings.video]?.gpu,
          cpuEncoder: ENCODERS[settings.video]?.cpu,
          codecEncoders: ENCODERS,
          live: prepStub.live,
        }),
        true
      );
    }
    if (path === "/codex/media-prep" && m === "PUT") {
      void body(req).then((b) => {
        // Missing fields take their defaults (as the server's), the whole object replaced.
        settings = { ...DEFAULTS, ...b, loudnessTarget: Math.min(-5, Math.max(-30, Number(b.loudnessTarget ?? DEFAULTS.loudnessTarget))) };
        if (settings.videoKbps != null) settings.videoKbps = Math.min(60000, Math.max(1000, Number(settings.videoKbps)));
        else delete settings.videoKbps;
        json(res, 200, settings);
      });
      return true;
    }
    if (path.startsWith("/reports/ccli") && m === "GET") {
      return json(res, 200, ccliReport(query)), true;
    }
    if (parts[0] !== "codex" || parts[1] !== "collections" || parts[3] !== "media" || parts.length !== 6 || !list) return false;
    const asset = list.find((x) => x.asset.id === parts[4]);
    const verb = parts[5];
    if (!["prepare", "trim", "original"].includes(verb)) return false;
    if (!asset) return apiError(res, 404, "notFound", "media asset not found"), true;
    if (verb === "prepare" && m === "POST") {
      jobs = jobs.filter((j) => j.asset !== asset.asset.id);
      jobs.push({ asset: asset.asset.id, collection: parts[2], name: asset.asset.name, step: "queued", progress: 0 });
      return (res.writeHead(202), res.end(), true);
    }
    if (verb === "trim" && m === "PUT") {
      void body(req).then((b) => {
        const p = { ...(asset.asset.prep ?? {}) };
        delete p.trimIn;
        delete p.trimOut;
        if (b.trimIn != null) p.trimIn = b.trimIn;
        if (b.trimOut != null) p.trimOut = b.trimOut;
        asset.asset.prep = p;
        json(res, 200, { asset: asset.asset, revision: asset.revision });
      });
      return true;
    }
    if (verb === "original" && m === "POST") {
      const p = asset.asset.prep;
      if (!p?.original) return apiError(res, 409, "conflict", "this media plays its original already"), true;
      asset.asset.probe = p.original.probe ?? asset.asset.probe;
      asset.asset.prep = { ...p, original: undefined, notes: ["Plays the original (chosen by hand)"] };
      return json(res, 200, { asset: asset.asset, revision: asset.revision }), true;
    }
    return false;
  };
}

const SONGS = [
  ["Cedar and Stone", "7000101", "A. Writer, B. Composer", 9, 4, 2],
  ["Lanterns Over Water", "7000102", "C. Lyricist", 7, 5, 0],
  ["Quiet Harvest", "7000103", "D. Hymnwriter", 5, 2, 1],
  ["Northbound Praise", "7000104", "E. Songsmith, F. Arranger", 4, 4, 4],
  ["Open Fields Hymn", "", "Traditional (arr. G. Organist)", 2, 0, 0],
  // The same title, no number, other authors: a row of its own (the table lists both).
  ["Open Fields Hymn", "", "Traditional (arr. H. Pianist)", 1, 0, 0],
];

function ccliReport(q) {
  const from = q.get("from") ?? "2026-05-27";
  const to = q.get("to") ?? "2026-11-26";
  const rows = SONGS.map(([title, ccliNumber, authors, count, streamed, recorded], i) => ({
    title,
    ccliNumber,
    authors,
    count,
    projected: count,
    streamed,
    recorded,
    lastUsed: `2026-11-${String(22 - i * 3).padStart(2, "0")}`,
  }));
  const uses = rows.flatMap((r, i) =>
    Array.from({ length: r.count }, (_, k) => ({
      date: `2026-11-${String(1 + k * 3).padStart(2, "0")}`,
      at: "2026-11-01T15:00:00Z",
      service: k % 3 === 2 ? "Wednesday Evening" : "Sunday Morning",
      presentation: `5eed0000-0000-4000-8000-0000000c000${i}`,
      title: r.title,
      ...(r.ccliNumber ? { ccliNumber: r.ccliNumber } : {}),
      authors: r.authors,
      projected: true,
      streamed: k < r.streamed,
      recorded: k < r.recorded,
    })),
  );
  return { from, to, rows, uses };
}
