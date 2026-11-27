// Reports ▸ CCLI usage (docs/MEDIA-PREP-PLAN.md): the date range, the CSV in the columns CCLI's
// reporting asks for (the same as the server's `/reports/ccli.csv`), and the totals. Plain
// TypeScript, unit tested.

import type { CcliReport } from "$protocol";

/** `YYYY-MM-DD` of a local date. */
export function day(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Named ranges ending today. */
export type RangeName = "month" | "quarter" | "half" | "year";

export const RANGES: { value: RangeName; label: string }[] = [
  { value: "month", label: "Last month" },
  { value: "quarter", label: "Last 3 months" },
  { value: "half", label: "Last 6 months" },
  { value: "year", label: "Last 12 months" },
];

/**
 * The range `name` ending on `today`: from the day after the same day `months` earlier (the
 * last day of that month when it is shorter: on 31 August, "Last 6 months" starts on 1 March).
 */
export function rangeOf(name: RangeName, today: Date): { from: string; to: string } {
  const months = { month: 1, quarter: 3, half: 6, year: 12 }[name];
  const y = today.getFullYear();
  const m = today.getMonth() - months;
  const last = new Date(y, m + 1, 0).getDate();
  const from = new Date(y, m, Math.min(today.getDate(), last) + 1);
  return { from: day(from), to: day(today) };
}

/** A CSV field, quoted when needed, never read as a formula. */
export function csvField(v: string): string {
  const s = /^[=+\-@]/.test(v) ? `'${v}` : v;
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** The report as CSV: Song Title, CCLI Song #, Authors, Times Used, Projected, Streamed, Recorded, From, To. */
export function ccliCsv(r: CcliReport): string {
  const head = "Song Title,CCLI Song #,Authors,Times Used,Projected,Streamed,Recorded,From,To";
  const rows = r.rows.map((x) =>
    [csvField(x.title), csvField(x.ccliNumber), csvField(x.authors), x.count, x.projected, x.streamed, x.recorded, r.from, r.to].join(","),
  );
  return [head, ...rows].join("\r\n") + "\r\n";
}

/** Totals for the summary line. */
export function totals(r: CcliReport): { songs: number; uses: number; services: number; noNumber: number } {
  const services = new Set(r.uses.map((u) => `${u.date}|${u.service ?? ""}`));
  return {
    songs: r.rows.length,
    uses: r.rows.reduce((n, x) => n + x.count, 0),
    services: services.size,
    noNumber: r.rows.filter((x) => !x.ccliNumber).length,
  };
}
