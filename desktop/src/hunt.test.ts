import { describe, expect, it } from "vitest";
import {
  countRepliesTo,
  gridToLatLon,
  haversineKm,
  huntPriorityFromConfig,
  isPotaCq,
  maidenheadDistanceKm,
  passesSnrFloor,
  selectHuntCandidate,
  type HuntCandidate,
} from "./hunt";

function candidate(overrides: Partial<HuntCandidate> & { index: number }): HuntCandidate {
  return {
    snr: null,
    distanceKm: null,
    isActivator: false,
    isNewGrid: false,
    pileupCalls: 0,
    ...overrides,
  };
}

describe("huntPriorityFromConfig", () => {
  it("round-trips a valid stored value", () => {
    expect(huntPriorityFromConfig("strongest")).toBe("strongest");
    expect(huntPriorityFromConfig("STRONGEST")).toBe("strongest");
    expect(huntPriorityFromConfig("  weakest  ")).toBe("weakest");
  });

  it("falls back to latest for unknown/blank/missing values", () => {
    expect(huntPriorityFromConfig(null)).toBe("latest");
    expect(huntPriorityFromConfig(undefined)).toBe("latest");
    expect(huntPriorityFromConfig("")).toBe("latest");
    expect(huntPriorityFromConfig("new_dxcc_first")).toBe("latest"); // not supported here
    expect(huntPriorityFromConfig("garbage")).toBe("latest");
  });
});

describe("passesSnrFloor", () => {
  it("passes everything when there is no floor", () => {
    expect(passesSnrFloor(-20, null)).toBe(true);
  });
  it("passes an unknown SNR regardless of the floor", () => {
    expect(passesSnrFloor(null, 0)).toBe(true);
  });
  it("filters below the floor, keeps at/above it", () => {
    expect(passesSnrFloor(-5, 0)).toBe(false);
    expect(passesSnrFloor(0, 0)).toBe(true);
    expect(passesSnrFloor(5, 0)).toBe(true);
  });
});

describe("selectHuntCandidate", () => {
  it("returns null when nothing clears the SNR floor", () => {
    const pool = [candidate({ index: 0, snr: -10 }), candidate({ index: 1, snr: -5 })];
    expect(selectHuntCandidate(pool, "latest", false, 0)).toBeNull();
  });

  it("latest picks the freshest (first) eligible candidate", () => {
    const pool = [candidate({ index: 0, snr: -3 }), candidate({ index: 1, snr: 10 })];
    expect(selectHuntCandidate(pool, "latest", false, null)?.index).toBe(0);
  });

  it("strongest picks the highest SNR, ties broken by freshness", () => {
    const pool = [
      candidate({ index: 0, snr: 2 }),
      candidate({ index: 1, snr: 10 }),
      candidate({ index: 2, snr: 10 }), // tie with index 1: index 1 is fresher
    ];
    expect(selectHuntCandidate(pool, "strongest", false, null)?.index).toBe(1);
  });

  it("weakest picks the lowest SNR; an unknown SNR never wins the weakest crown", () => {
    const pool = [
      candidate({ index: 0, snr: null }),
      candidate({ index: 1, snr: -15 }),
      candidate({ index: 2, snr: -2 }),
    ];
    expect(selectHuntCandidate(pool, "weakest", false, null)?.index).toBe(1);
  });

  it("farthest picks the largest distance; an unknown distance never wins", () => {
    const pool = [
      candidate({ index: 0, distanceKm: null }),
      candidate({ index: 1, distanceKm: 500 }),
      candidate({ index: 2, distanceKm: 9000 }),
    ];
    expect(selectHuntCandidate(pool, "farthest", false, null)?.index).toBe(2);
  });

  it("pota_first prefers an activator, then higher SNR, then freshness", () => {
    const pool = [
      candidate({ index: 0, isActivator: false, snr: 20 }),
      candidate({ index: 1, isActivator: true, snr: -5 }),
      candidate({ index: 2, isActivator: true, snr: 3 }),
    ];
    expect(selectHuntCandidate(pool, "pota_first", false, null)?.index).toBe(2);
  });

  it("new_grid_first prefers an unworked grid, then higher SNR", () => {
    const pool = [
      candidate({ index: 0, isNewGrid: false, snr: 20 }),
      candidate({ index: 1, isNewGrid: true, snr: -5 }),
    ];
    expect(selectHuntCandidate(pool, "new_grid_first", false, null)?.index).toBe(1);
  });

  it("avoids pileups when a pileup-free candidate exists", () => {
    const pool = [
      candidate({ index: 0, snr: 10, pileupCalls: 3 }),
      candidate({ index: 1, snr: -10, pileupCalls: 0 }),
    ];
    expect(selectHuntCandidate(pool, "strongest", true, null)?.index).toBe(1);
  });

  it("falls back to the full pool when every candidate has a pileup", () => {
    const pool = [
      candidate({ index: 0, snr: -10, pileupCalls: 1 }),
      candidate({ index: 1, snr: 10, pileupCalls: 2 }),
    ];
    expect(selectHuntCandidate(pool, "strongest", true, null)?.index).toBe(1);
  });
});

describe("countRepliesTo", () => {
  it("counts non-CQ messages addressed to the caller", () => {
    const messages = [
      { is_cq: false, call_to: "K1ABC" },
      { is_cq: false, call_to: "k1abc" },
      { is_cq: true, call_to: "K1ABC" }, // a CQ, not a reply -- excluded
      { is_cq: false, call_to: "W1AW" },
    ];
    expect(countRepliesTo("K1ABC", messages)).toBe(2);
  });

  it("returns 0 for a null/empty callsign", () => {
    expect(countRepliesTo(null, [{ is_cq: false, call_to: "K1ABC" }])).toBe(0);
  });
});

describe("isPotaCq", () => {
  it("requires is_cq", () => {
    expect(isPotaCq(false, "CQ POTA K1ABC FN42")).toBe(false);
  });
  it("matches an explicit POTA token", () => {
    expect(isPotaCq(true, "CQ POTA K1ABC FN42")).toBe(true);
  });
  it("matches a garbled CQ POT fragment", () => {
    expect(isPotaCq(true, "CQ POT KX1Y/P")).toBe(true);
  });
  it("does not match a plain CQ", () => {
    expect(isPotaCq(true, "CQ K1ABC FN42")).toBe(false);
  });
});

describe("gridToLatLon", () => {
  it("resolves a known 4-char grid to its exact center", () => {
    // J=9th letter (index 9): lon = 9*20-180+0 = 0, center of a 2°-wide
    // square -> +1 = 1. Same field index for lat: 9*10-90+0 = 0, center of a
    // 1°-tall square -> +0.5 = 0.5.
    expect(gridToLatLon("JJ00")).toEqual({ lon: 1, lat: 0.5 });
  });

  it("narrows to the subsquare center with a 6-char grid", () => {
    // Same field/square as above, subsquare A (index 0) adds 0, then centers
    // within the much smaller subsquare cell.
    const p = gridToLatLon("JJ00AA");
    expect(p?.lon).toBeCloseTo(0 + 2 / 24 / 2, 6);
    expect(p?.lat).toBeCloseTo(0 + 1 / 24 / 2, 6);
  });

  it("rejects a too-short or malformed grid", () => {
    expect(gridToLatLon("JJ0")).toBeNull();
    expect(gridToLatLon("991A")).toBeNull();
  });
});

describe("haversineKm / maidenheadDistanceKm", () => {
  it("is zero for the same point", () => {
    expect(haversineKm(10, 20, 10, 20)).toBeCloseTo(0, 9);
  });

  it("is symmetric", () => {
    const a = haversineKm(10, 20, -30, 100);
    const b = haversineKm(-30, 100, 10, 20);
    expect(a).toBeCloseTo(b, 9);
  });

  it("maidenheadDistanceKm is 0 for the same grid and null for a bad grid", () => {
    expect(maidenheadDistanceKm("FN42", "FN42")).toBeCloseTo(0, 6);
    expect(maidenheadDistanceKm("FN42", "??")).toBeNull();
  });
});
