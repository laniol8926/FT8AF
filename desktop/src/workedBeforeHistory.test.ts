import { describe, expect, it } from "vitest";
import { formatQsoDate, summarizeWorkedBefore, type PriorQso } from "./workedBeforeHistory";

function qso(date: string, time = "120000", band: string | null = "20m", mode: string | null = "FT8"): PriorQso {
  return { qsoDate: date, timeOn: time, band, mode };
}

describe("summarizeWorkedBefore", () => {
  it("returns null for an empty log", () => {
    expect(summarizeWorkedBefore([])).toBeNull();
  });

  it("reports count 1 and a formatted recap line for a single QSO", () => {
    const s = summarizeWorkedBefore([qso("20250612", "143000", "20m", "FT8")])!;
    expect(s.count).toBe(1);
    expect(s.lastLine).toBe("2025-06-12 · 20m · FT8");
    expect(s.bands).toEqual(["20m"]);
  });

  it("picks the most recent QSO regardless of input order", () => {
    const s = summarizeWorkedBefore([
      qso("20240101", "090000", "40m", "FT8"),
      qso("20260724", "010000", "15m", "FT4"),
      qso("20250612", "143000", "20m", "FT8"),
    ])!;
    expect(s.count).toBe(3);
    expect(s.lastLine).toBe("2026-07-24 · 15m · FT4");
  });

  it("breaks a same-day tie on timeOn", () => {
    const s = summarizeWorkedBefore([
      qso("20250612", "235900", "20m", "FT8"),
      qso("20250612", "000100", "40m", "FT8"),
    ])!;
    expect(s.lastLine).toBe("2025-06-12 · 20m · FT8");
  });

  it("keeps distinct bands in first-worked order", () => {
    const s = summarizeWorkedBefore([
      qso("20240101", "090000", "40m"),
      qso("20250101", "090000", "20m"),
      qso("20250601", "090000", "40m"),
      qso("20260101", "090000", "15m"),
    ])!;
    expect(s.bands).toEqual(["40m", "20m", "15m"]);
  });

  it("skips blank band and mode parts in the recap line", () => {
    const s = summarizeWorkedBefore([qso("20250612", "120000", "  ", null)])!;
    expect(s.lastLine).toBe("2025-06-12");
    expect(s.bands).toEqual([]);
  });

  it("passes through an unparseable date so the recap is never empty", () => {
    const s = summarizeWorkedBefore([qso("2025-06-12", "120000", "20m", "FT8")])!;
    expect(s.lastLine).toBe("2025-06-12 · 20m · FT8");
  });
});

describe("formatQsoDate", () => {
  it("turns YYYYMMDD into YYYY-MM-DD", () => {
    expect(formatQsoDate("20250612")).toBe("2025-06-12");
  });

  it("leaves non-8-digit values untouched and trims", () => {
    expect(formatQsoDate(" 2025 ")).toBe("2025");
    expect(formatQsoDate("2025-06-12")).toBe("2025-06-12");
    expect(formatQsoDate("abcdefgh")).toBe("abcdefgh");
    expect(formatQsoDate(null)).toBe("");
    expect(formatQsoDate("")).toBe("");
  });
});
