// Machine translation (docs/DUAL-LANGUAGE-PLAN.md §5): `GET /translate/models` (synthetic
// statuses: one pair downloaded, one downloading, one failed), the admin's download / stop /
// delete, and previews (`POST /translate/preview`, `GET|DELETE /translate/jobs/{id}`) that
// "translate" the Reflow song (routes/reflow.mjs) with a fixed table of made-up Spanish lines;
// a job moves queued → translating → done over its first reads. Adds the `translate`
// capability to the stub's server info.
import { reflowStub } from "./reflow.mjs";

const NOTICE = "Machine translation: check it, and check your licence allows a translation of this song";
const ENGINE = "opus-mt (candle)";

const pair = (id, from, to, repo, licence, size, status = "notDownloaded", extra = {}) => ({
  id,
  from,
  to,
  model: repo.split("/")[1],
  repo,
  revision: "0123456789abcdef0123456789abcdef01234567",
  card: `https://huggingface.co/${repo}`,
  licence,
  attribution: `${repo.split("/")[1]} by Helsinki-NLP (OPUS-MT, University of Helsinki), ${licence} licence, from https://huggingface.co/${repo} at revision 0123456789ab`,
  size,
  status,
  ...extra,
});

const MODELS = () => [
  pair("en-es", "en", ["es"], "Helsinki-NLP/opus-mt-en-es", "Apache-2.0", 315_312_000, "ready"),
  pair("es-en", "es", ["en"], "Helsinki-NLP/opus-mt-es-en", "Apache-2.0", 315_312_000),
  pair("en-fr", "en", ["fr"], "Helsinki-NLP/opus-mt-en-fr", "Apache-2.0", 303_745_000),
  pair("en-de", "en", ["de"], "Helsinki-NLP/opus-mt-en-de", "CC-BY-4.0", 300_765_000),
  pair("en-pt", "en", ["pt", "pt-BR"], "Helsinki-NLP/opus-mt-tc-big-en-pt", "CC-BY-4.0", 468_046_000, "downloading", { downloaded: 141_000_000 }),
  pair("ko-en", "ko", ["en"], "Helsinki-NLP/opus-mt-ko-en", "Apache-2.0", 315_220_000),
  pair("en-zh", "en", ["zh", "zh-Hans", "zh-Hant"], "Helsinki-NLP/opus-mt-en-zh", "Apache-2.0", 315_241_000, "failed", {
    detail: "the download of pytorch_model.bin broke off: connection reset",
  }),
];

/** Made-up Spanish for the Reflow song's lines (any other line comes back bracketed). */
const ES = {
  "Lanterns on the hill tonight": "Faroles en la colina esta noche",
  "Glowing soft against the rain": "Brillando suaves contra la lluvia",
  "Every road that led me here": "Cada camino que me trajo aquí",
  "Turns to gold and home again": "Se vuelve oro y hogar otra vez",
  "Carry the light, carry it high": "Lleva la luz, llévala en alto",
  "Over the water, under the sky": "Sobre el agua, bajo el cielo",
  "Nothing is lost that love can find": "Nada se pierde que el amor pueda hallar",
  "Neighbours gather at the gate": "Los vecinos se reúnen en la puerta",
  "Bread and stories on the table": "Pan e historias en la mesa",
  "Every window lit and warm": "Cada ventana encendida y cálida",
  "Every heart as wide as able": "Cada corazón tan abierto como puede",
  "Slow now, the fields are still": "Despacio ahora, los campos están quietos",
  "Hold on, the dawn is near": "Espera, el alba está cerca",
  "Lanterns on the hill": "Faroles en la colina",
  "Carry the light home": "Lleva la luz a casa",
};

const text = (paragraphs) => (paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join(""));

function propose(slide, to) {
  const texts = [];
  for (const e of slide.elements ?? []) {
    if (e.kind?.type !== "text") continue;
    const lines = text(e.kind.text.paragraphs);
    if (!lines.some((l) => l.trim())) continue;
    const out = lines.map((l) => (l.trim() ? (ES[l.trim()] ?? `[${to}] ${l.trim()}`) : ""));
    texts.push({ element: e.id, paragraphs: out.map((l) => (l ? { runs: [{ text: l }] } : {})) });
  }
  if (!texts.length) return null;
  return { language: to, texts, machine: { engine: ENGINE, model: "opus-mt-en-es" } };
}

export default function translateRoutes({ fx, json, apiError }) {
  const server = fx.get("server");
  if (server && !server.capabilities.includes("translate")) server.capabilities.push("translate");
  let models = MODELS();
  const jobs = new Map();
  let next = 1;
  const read = (req) =>
    new Promise((ok) => {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => ok(body ? JSON.parse(body) : {}));
    });

  return (req, res, { parts }) => {
    if (parts[0] !== "translate") return false;
    if (parts[1] === "models" && parts.length === 2 && req.method === "GET") {
      json(res, 200, { engine: ENGINE, models, notice: NOTICE });
      return true;
    }
    if (parts[1] === "models" && parts[2]) {
      const m = models.find((x) => x.id === parts[2]);
      if (!m) {
        apiError(res, 404, "notFound", `no language pair "${parts[2]}"`);
        return true;
      }
      if (req.method === "POST" && parts[3] === "download") Object.assign(m, { status: "downloading", downloaded: 0, detail: undefined });
      else if (req.method === "POST" && parts[3] === "stop") Object.assign(m, { status: "notDownloaded", downloaded: undefined });
      else if (req.method === "DELETE" && parts.length === 3) Object.assign(m, { status: "notDownloaded", downloaded: undefined, detail: undefined });
      else return false;
      json(res, 200, m);
      return true;
    }
    if (parts[1] === "preview" && req.method === "POST") {
      void read(req).then((b) => {
        const doc = reflowStub.on && reflowStub.doc()?.id === b.presentation ? reflowStub.doc() : null;
        const slides = (doc?.sections ?? []).flatMap((s) => s.slides ?? []).filter((s) => !b.slides || b.slides.includes(s.id));
        const proposals = slides.map((s) => ({ slide: s.id, translation: propose(s, b.to) })).filter((p) => p.translation);
        const id = `job-${next++}`;
        const job = { id, presentation: b.presentation, from: b.from ?? "en", to: b.to, model: "opus-mt-en-es", status: "queued", done: 0, total: proposals.length, slides: [], notice: NOTICE, reads: 0, proposals };
        jobs.set(id, job);
        const { reads: _r, proposals: _p, ...pub } = job;
        json(res, 202, pub);
      });
      return true;
    }
    if (parts[1] === "jobs" && parts[2]) {
      const job = jobs.get(parts[2]);
      if (!job) {
        apiError(res, 404, "notFound", "no such translation preview");
        return true;
      }
      if (req.method === "DELETE") {
        job.status = "cancelled";
        res.writeHead(204);
        res.end();
        return true;
      }
      job.reads += 1;
      if (job.status === "queued") Object.assign(job, { status: "translating", done: Math.floor(job.total / 2) });
      else if (job.status === "translating") Object.assign(job, { status: "done", done: job.total, slides: job.proposals, seconds: 1.8 });
      const { reads: _r, proposals: _p, ...pub } = job;
      json(res, 200, pub);
      return true;
    }
    return false;
  };
}
