// Uploading a show box's own, dormant collection to the site's Codex (G15), in words. Unit
// tested.

import type { DormantUpload, TransferCount } from "$protocol";

function bytes(n: number): string {
  if (n < 1024 * 1024) return `${Math.max(0, Math.round(n / 1024))} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function of(c: TransferCount, what: string): string {
  return `${c.done} of ${c.total} ${what}`;
}

/** The offer's text: "This box's own collection Youth (12 songs) isn't in the site's Codex." */
export function offerText(u: Pick<DormantUpload, "name" | "songs">): string {
  const name = u.name?.trim() || "Untitled";
  return `This box's own collection “${name}” (${u.songs} song${u.songs === 1 ? "" : "s"}) isn't in the site's Codex.`;
}

/** Progress in words, and how far along (0–1) for the bar. */
export function progressText(u: DormantUpload): { text: string; fraction: number } {
  const docsLeft = u.documents.total > u.documents.done;
  if (u.state === "copying" && (docsLeft || u.media.total === 0)) {
    const f = u.documents.total ? u.documents.done / u.documents.total : 0;
    return { text: `Copying documents: ${of(u.documents, "")}`.trim(), fraction: f * 0.3 };
  }
  const f = u.bytes.total ? u.bytes.done / u.bytes.total : u.media.total ? u.media.done / u.media.total : 1;
  const files = of(u.media, `file${u.media.total === 1 ? "" : "s"}`);
  const size = u.bytes.total ? ` · ${bytes(u.bytes.done)} of ${bytes(u.bytes.total)}` : "";
  return { text: `Copying media: ${files}${size}`, fraction: 0.3 + 0.7 * Math.min(1, f) };
}

/** What the card says for each state (null: nothing to show). */
export function dormantCard(u: DormantUpload | null): { title: string; detail: string; tone: "info" | "busy" | "ok" | "error" } | null {
  if (!u) return null;
  const into = u.collectionName ? `“${u.collectionName}”` : "a new collection";
  switch (u.state) {
    case "idle":
      return { title: "Upload this box's collection", detail: offerText(u), tone: "info" };
    case "copying":
      return { title: `Uploading to ${into}`, detail: progressText(u).text, tone: "busy" };
    case "done":
      return { title: "Uploaded", detail: `Everything from “${u.name}” is in the site's Codex as ${into}.`, tone: "ok" };
    case "failed":
      return { title: "Upload stopped", detail: u.error ?? "The upload stopped.", tone: "error" };
    case "cancelled":
      return { title: "Upload cancelled", detail: `What was copied stays in ${into}. Upload again to go on.`, tone: "info" };
  }
}
