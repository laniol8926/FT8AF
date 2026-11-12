import { describe, expect, test } from "vitest";
import { isNewZoneStation, markerColorForWorkedStatus, snrTint, workedBandStatus } from "./markerStyle";

function parseRgba(s: string): { r: number; g: number; b: number; a: number } {
  const m = s.match(/rgba\((\d+), (\d+), (\d+), ([\d.]+)\)/);
  if (!m) throw new Error(`not an rgba() string: ${s}`);
  return { r: Number(m[1]), g: Number(m[2]), b: Number(m[3]), a: Number(m[4]) };
}

describe("snrTint", () => {
  test("floors at alpha 0.78 for a weak decode (-24 dB or below)", () => {
    expect(parseRgba(snrTint("#ffaf5e", -24)).a).toBeCloseTo(0.78, 6);
    expect(parseRgba(snrTint("#ffaf5e", -40)).a).toBeCloseTo(0.78, 6); // clamped, not extrapolated
  });

  test("caps at alpha 1.0 for a strong decode (+10 dB or above)", () => {
    expect(parseRgba(snrTint("#ffaf5e", 10)).a).toBeCloseTo(1.0, 6);
    expect(parseRgba(snrTint("#ffaf5e", 30)).a).toBeCloseTo(1.0, 6); // clamped
  });

  test("midpoint SNR gives a midpoint alpha", () => {
    const midSnr = (-24 + 10) / 2; // -7
    expect(parseRgba(snrTint("#ffaf5e", midSnr)).a).toBeCloseTo(0.78 + 0.22 * 0.5, 2);
  });

  test("preserves the base color's RGB channels regardless of SNR", () => {
    const weak = parseRgba(snrTint("#4ade80", -24));
    const strong = parseRgba(snrTint("#4ade80", 10));
    expect(weak.r).toBe(0x4a);
    expect(weak.g).toBe(0xde);
    expect(weak.b).toBe(0x80);
    expect(strong.r).toBe(weak.r);
    expect(strong.g).toBe(weak.g);
    expect(strong.b).toBe(weak.b);
  });
});

describe("workedBandStatus", () => {
  test("worked this band takes priority over worked another band", () => {
    const status = workedBandStatus("W1AW", new Set(["W1AW"]), new Set(["W1AW"]));
    expect(status).toEqual({ workedThisBand: true, workedOtherBand: false });
  });

  test("worked another band only, when not worked this band", () => {
    const status = workedBandStatus("W1AW", new Set(), new Set(["W1AW"]));
    expect(status).toEqual({ workedThisBand: false, workedOtherBand: true });
  });

  test("neither, for a brand-new callsign", () => {
    const status = workedBandStatus("W1AW", new Set(), new Set());
    expect(status).toEqual({ workedThisBand: false, workedOtherBand: false });
  });
});

describe("markerColorForWorkedStatus", () => {
  test("worked this band is green (--ok)", () => {
    expect(markerColorForWorkedStatus({ workedThisBand: true, workedOtherBand: false })).toBe("#4ade80");
  });

  test("worked another band only is yellow (--yellow)", () => {
    expect(markerColorForWorkedStatus({ workedThisBand: false, workedOtherBand: true })).toBe("#facc15");
  });

  test("brand new is amber (--accent)", () => {
    expect(markerColorForWorkedStatus({ workedThisBand: false, workedOtherBand: false })).toBe("#ffaf5e");
  });
});

describe("isNewZoneStation", () => {
  const unworked = { workedThisBand: false, workedOtherBand: false };

  test("a never-worked DXCC prefix is a new zone", () => {
    expect(isNewZoneStation(unworked, "JA", null, new Set(), new Set())).toBe(true);
  });

  test("a never-worked CQ zone is a new zone", () => {
    expect(isNewZoneStation(unworked, null, 25, new Set(), new Set())).toBe(true);
  });

  test("an already-worked DXCC prefix and CQ zone is not a new zone", () => {
    expect(isNewZoneStation(unworked, "JA", 25, new Set(["JA"]), new Set([25]))).toBe(false);
  });

  test("null dxccPrefix/cqZone (no resolved zone data) is not a new zone", () => {
    expect(isNewZoneStation(unworked, null, null, new Set(), new Set())).toBe(false);
  });

  test("worked this band is never a new zone, even with an unworked DXCC prefix", () => {
    const status = { workedThisBand: true, workedOtherBand: false };
    expect(isNewZoneStation(status, "JA", null, new Set(), new Set())).toBe(false);
  });

  test("worked another band only is never a new zone, even with an unworked CQ zone", () => {
    const status = { workedThisBand: false, workedOtherBand: true };
    expect(isNewZoneStation(status, null, 25, new Set(), new Set())).toBe(false);
  });
});
