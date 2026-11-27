import { describe, expect, it } from "vitest";
import type { CcliReport } from "$protocol";
import { ccliCsv, csvField, day, rangeOf, totals } from "./ccli";

const report: CcliReport = {
  from: "2026-06-01",
  to: "2026-11-30",
  rows: [
    { title: "Morning Light", ccliNumber: "7654321", authors: "A. Writer, B. Composer", count: 3, projected: 3, streamed: 1, recorded: 0, lastUsed: "2026-11-29" },
    { title: "River Song", ccliNumber: "", authors: "", count: 1, projected: 1, streamed: 0, recorded: 0, lastUsed: "2026-11-22" },
  ],
  uses: [
    { date: "2026-11-22", at: "2026-11-22T15:00:00Z", service: "Sunday", presentation: "p1", title: "Morning Light", projected: true, streamed: false, recorded: false },
    { date: "2026-11-22", at: "2026-11-22T15:10:00Z", service: "Sunday", presentation: "p2", title: "River Song", projected: true, streamed: false, recorded: false },
    { date: "2026-11-29", at: "2026-11-29T15:00:00Z", service: "Sunday", presentation: "p1", title: "Morning Light", projected: true, streamed: true, recorded: false },
    { date: "2026-11-29", at: "2026-11-29T19:00:00Z", presentation: "p1", title: "Morning Light", projected: true, streamed: false, recorded: false },
  ],
};

describe("CCLI usage report", () => {
  it("writes CCLI's columns, quoting where needed", () => {
    const lines = ccliCsv(report).split("\r\n");
    expect(lines[0]).toBe("Song Title,CCLI Song #,Authors,Times Used,Projected,Streamed,Recorded,From,To");
    expect(lines[1]).toBe('Morning Light,7654321,"A. Writer, B. Composer",3,3,1,0,2026-06-01,2026-11-30');
    expect(lines[2]).toBe("River Song,,,1,1,0,0,2026-06-01,2026-11-30");
    expect(lines[3]).toBe("");
    expect(csvField("=HYPERLINK()")).toBe("'=HYPERLINK()");
    expect(csvField('say "hi"')).toBe('"say ""hi"""');
  });

  it("counts songs, uses, services and songs without a number", () => {
    expect(totals(report)).toEqual({ songs: 2, uses: 4, services: 3, noNumber: 1 });
  });

  it("names ranges ending today", () => {
    const today = new Date(2026, 10, 26);
    expect(day(today)).toBe("2026-11-26");
    expect(rangeOf("month", today)).toEqual({ from: "2026-10-27", to: "2026-11-26" });
    expect(rangeOf("half", today)).toEqual({ from: "2026-05-27", to: "2026-11-26" });
    expect(rangeOf("year", new Date(2026, 1, 28))).toEqual({ from: "2025-03-01", to: "2026-02-28" });
    // The end of a long month: from the 1st, never a few days into the month.
    expect(rangeOf("half", new Date(2026, 7, 31))).toEqual({ from: "2026-03-01", to: "2026-08-31" });
    expect(rangeOf("month", new Date(2026, 2, 31))).toEqual({ from: "2026-03-01", to: "2026-03-31" });
    expect(rangeOf("quarter", new Date(2026, 4, 31))).toEqual({ from: "2026-03-01", to: "2026-05-31" });
    expect(rangeOf("year", new Date(2028, 1, 29))).toEqual({ from: "2027-03-01", to: "2028-02-29" });
    expect(rangeOf("month", new Date(2026, 0, 31))).toEqual({ from: "2026-01-01", to: "2026-01-31" });
  });
});
