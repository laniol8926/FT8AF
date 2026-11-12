import { describe, expect, test } from "vitest";
import { applyPskBandFilter, initialPskFilter, pskTimeLabel, resolvePskModeString } from "./pskFilter";

describe("pskTimeLabel", () => {
  test("formats sub-hour presets in minutes", () => {
    expect(pskTimeLabel(900)).toBe("15m");
  });

  test("formats hour-plus presets in hours", () => {
    expect(pskTimeLabel(3600)).toBe("1h");
    expect(pskTimeLabel(86400)).toBe("24h");
  });
});

describe("resolvePskModeString", () => {
  test("CURRENT resolves to the live operating mode, uppercased", () => {
    expect(resolvePskModeString("CURRENT", "ft8")).toBe("FT8");
    expect(resolvePskModeString("CURRENT", "ft4")).toBe("FT4");
  });

  test("an explicit filter selection is used as-is, ignoring the live mode", () => {
    expect(resolvePskModeString("FT4", "ft8")).toBe("FT4");
  });
});

describe("initialPskFilter", () => {
  test("seeds the band from the operator's current dial frequency", () => {
    const f = initialPskFilter(14_074_000, () => "20m");
    expect(f.band).toBe("20m");
    expect(f.timeWindowSec).toBe(3600);
    expect(f.mode).toBe("CURRENT");
    expect(f.direction).toBe("WHO_HEARD_ME");
  });

  test("an unresolvable dial frequency leaves the band unset (All)", () => {
    const f = initialPskFilter(0, () => null);
    expect(f.band).toBeNull();
  });
});

describe("applyPskBandFilter", () => {
  const spots = [{ band: "20m" }, { band: "40m" }, { band: "20m" }];

  test("null band keeps everything", () => {
    expect(applyPskBandFilter(spots, null)).toHaveLength(3);
  });

  test("a specific band keeps only matching spots", () => {
    expect(applyPskBandFilter(spots, "20m")).toHaveLength(2);
    expect(applyPskBandFilter(spots, "6m")).toHaveLength(0);
  });
});
