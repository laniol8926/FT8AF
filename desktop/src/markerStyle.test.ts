import { describe, expect, test } from "vitest";
import { snrTint } from "./markerStyle";

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
