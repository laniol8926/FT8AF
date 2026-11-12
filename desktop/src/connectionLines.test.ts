import { describe, expect, test } from "vitest";
import { buildConnectionLines, type StationLine } from "./connectionLines";

const stations: StationLine[] = [
  { callsign: "W1AW", grid: "FN31", isToMe: false },
  { callsign: "VE3XY", grid: "FN03", isToMe: true },
  { callsign: "G4ABC", grid: "IO91", isToMe: true },
];

describe("buildConnectionLines", () => {
  test("no target: only the calling-me dashed lines", () => {
    const lines = buildConnectionLines(stations, null);
    expect(lines).toHaveLength(2);
    expect(lines.every((l) => l.style === "DASHED_CALLING_ME")).toBe(true);
    expect(lines.map((l) => l.callsign).sort()).toEqual(["G4ABC", "VE3XY"]);
  });

  test("blank or 'CQ' target is treated the same as no target", () => {
    expect(buildConnectionLines(stations, "")).toHaveLength(2);
    expect(buildConnectionLines(stations, "  ")).toHaveLength(2);
    expect(buildConnectionLines(stations, "cq")).toHaveLength(2);
  });

  test("a target found among stations gets exactly one solid line, first in the list", () => {
    const lines = buildConnectionLines(stations, "W1AW");
    expect(lines[0]).toEqual({ toGrid: "FN31", style: "SOLID_TX", callsign: "W1AW" });
    expect(lines.filter((l) => l.callsign === "W1AW")).toHaveLength(1);
  });

  test("a target not present in stations gets no line at all", () => {
    const lines = buildConnectionLines(stations, "N0SUCH");
    expect(lines.some((l) => l.callsign === "N0SUCH")).toBe(false);
    // The calling-me lines are unaffected.
    expect(lines).toHaveLength(2);
  });

  test("the target station, if also calling me, gets only the solid line -- not a duplicate dashed one", () => {
    const lines = buildConnectionLines(stations, "VE3XY");
    const forTarget = lines.filter((l) => l.callsign === "VE3XY");
    expect(forTarget).toHaveLength(1);
    expect(forTarget[0].style).toBe("SOLID_TX");
    // The other calling-me station is still present.
    expect(lines.some((l) => l.callsign === "G4ABC" && l.style === "DASHED_CALLING_ME")).toBe(true);
  });

  test("target matching is case-insensitive", () => {
    const lines = buildConnectionLines(stations, "w1aw");
    expect(lines.some((l) => l.callsign === "W1AW" && l.style === "SOLID_TX")).toBe(true);
  });
});
