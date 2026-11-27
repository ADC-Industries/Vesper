import { describe, expect, it } from "vitest";
import { applySetting, formatDuration, formatTimeOfDay, nextTimerName, parseDuration, parseTimeOfDay, timerSetting } from "./timers";

describe("durations", () => {
  it("formats minutes and hours", () => {
    expect(formatDuration(300)).toBe("5:00");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatDuration(-61)).toBe("-1:01");
    expect(formatDuration(0)).toBe("0:00");
  });
  it("parses typed durations", () => {
    expect(parseDuration("5")).toBe(300);
    expect(parseDuration("1.5")).toBe(90);
    expect(parseDuration("4:30")).toBe(270);
    expect(parseDuration("1:00:00")).toBe(3600);
    expect(parseDuration("2:75")).toBeNull();
    expect(parseDuration("abc")).toBeNull();
    expect(parseDuration("")).toBeNull();
  });
});

describe("time of day", () => {
  it("formats 12- and 24-hour", () => {
    expect(formatTimeOfDay(43500, "pm")).toBe("12:05 PM");
    expect(formatTimeOfDay(0, "am")).toBe("12:00 AM");
    expect(formatTimeOfDay(13 * 3600 + 5 * 60, "h24")).toBe("13:05");
    expect(formatTimeOfDay(9 * 3600 + 30 * 60 + 15, "am")).toBe("9:30:15 AM");
  });
  it("parses AM / PM and 24-hour", () => {
    expect(parseTimeOfDay("1:05 pm")).toEqual({ secondsAfterMidnight: 13 * 3600 + 300, period: "pm" });
    expect(parseTimeOfDay("12:00am")).toEqual({ secondsAfterMidnight: 0, period: "am" });
    expect(parseTimeOfDay("12:30 PM")).toEqual({ secondsAfterMidnight: 12 * 3600 + 1800, period: "pm" });
    expect(parseTimeOfDay("9a")).toEqual({ secondsAfterMidnight: 9 * 3600, period: "am" });
    expect(parseTimeOfDay("23:59")).toEqual({ secondsAfterMidnight: 86340, period: "h24" });
    expect(parseTimeOfDay("13:00 pm")).toBeNull();
    expect(parseTimeOfDay("24:00")).toBeNull();
  });
});

describe("timer settings", () => {
  it("shows and applies a countdown duration", () => {
    expect(timerSetting({ type: "countdown", seconds: 900 })).toBe("15:00");
    expect(applySetting({ type: "countdown", seconds: 900 }, "10:00")).toEqual({ type: "countdown", seconds: 600 });
    expect(applySetting({ type: "countdown" }, "x")).toBeNull();
  });
  it("keeps a 12-hour target 12-hour", () => {
    expect(timerSetting({ type: "countdownToTime", secondsAfterMidnight: 43500, period: "pm" })).toBe("12:05 PM");
    expect(applySetting({ type: "countdownToTime", period: "pm" }, "18:30")).toEqual({ type: "countdownToTime", secondsAfterMidnight: 66600, period: "pm" });
    expect(applySetting({ type: "countdownToTime" }, "6:30 pm")).toEqual({ type: "countdownToTime", secondsAfterMidnight: 66600, period: "pm" });
  });
  it("sets and clears an elapsed timer's end", () => {
    expect(applySetting({ type: "elapsed" }, "2:00")).toEqual({ type: "elapsed", end: 120 });
    expect(applySetting({ type: "elapsed", end: 120 }, "")).toEqual({ type: "elapsed" });
  });
  it("names new timers", () => {
    expect(nextTimerName(["Timer 1", "timer 2", "Other"])).toBe("Timer 3");
  });
});

describe("the Timers page", () => {
  it("colours and describes timers", async () => {
    const { timerTone, timerStatus, timerSummary } = await import("./timers");
    expect(timerTone("running", 30)).toBe("run");
    expect(timerTone("overrun", -5)).toBe("over");
    expect(timerTone("paused", 30)).toBe("idle");
    expect(timerTone("running", 60, 60)).toBe("warn");
    expect(timerTone("paused", 30, 60)).toBe("warn");
    expect(timerTone("running", 61, 60)).toBe("run");
    expect(timerTone("stopped", 30, 60)).toBe("idle");
    expect(timerTone("expired", 0, 60)).toBe("over");
    expect(timerStatus({ type: "countdown", seconds: 60 }, "overrun")).toBe("Over time");
    expect(timerStatus({ type: "elapsed" }, "running")).toBe("Counting up");
    expect(timerStatus({ type: "countdownToTime", secondsAfterMidnight: 36900, period: "am" }, "running")).toBe("Counting down to 10:15 AM");
    expect(timerSummary({ type: "countdown", seconds: 1800 }, true)).toBe("Counts down from 30:00, then counts up in red");
    expect(timerSummary({ type: "elapsed", end: 600 }, false)).toBe("Counts up from 0:00 when started, stops at 10:00");
  });
  it("reads minute, second and time fields", async () => {
    const { durationFrom, kindFromTimeInput, timeInputValue, formatClock } = await import("./timers");
    expect(durationFrom(30, 5)).toBe(1805);
    expect(durationFrom(-1, 75)).toBe(59);
    expect(timeInputValue(36900)).toBe("10:15");
    expect(kindFromTimeInput({ type: "countdownToTime", period: "am" }, "18:30")).toEqual({ type: "countdownToTime", secondsAfterMidnight: 66600, period: "pm" });
    expect(kindFromTimeInput({ type: "countdownToTime" }, "18:30")).toEqual({ type: "countdownToTime", secondsAfterMidnight: 66600, period: "h24" });
    expect(kindFromTimeInput(undefined, "nope")).toBeNull();
    const d = new Date(2026, 0, 1, 14, 5, 9);
    expect(formatClock(d, false)).toBe("2:05:09 PM");
    expect(formatClock(d, true)).toBe("14:05:09");
    expect(formatClock(d, true, false)).toBe("14:05");
    expect(formatClock(d, false, false)).toBe("2:05 PM");
  });
});

describe("warning time", () => {
  it("reads minutes and writes them back", async () => {
    const { warningFrom, warningText } = await import("./timers");
    expect(warningFrom("2")).toBe(120);
    expect(warningFrom(" 0,5 ")).toBe(30);
    expect(warningFrom("")).toBeNull();
    expect(warningFrom("soon")).toBeUndefined();
    expect(warningFrom("-1")).toBeUndefined();
    expect(warningText(120)).toBe("2");
    expect(warningText(90)).toBe("1.5");
    expect(warningText(undefined)).toBe("");
  });
});
