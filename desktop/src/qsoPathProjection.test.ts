import { describe, expect, it } from "vitest";
import { normalizeLon, QSO_MAP_MIN_SPAN_DEG, QSO_MAP_PAD_FRAC, QsoPathProjection } from "./qsoPathProjection";

const w = 400;
const h = 160;
const tol = 0.5;

describe("normalizeLon", () => {
  it("shifts an eastern lon west when that is the short way", () => {
    expect(normalizeLon(-98, 138)).toBeCloseTo(-222, 9);
  });

  it("shifts a western lon east across the antemeridian", () => {
    expect(normalizeLon(170, -170)).toBeCloseTo(190, 9);
  });

  it("leaves nearby longitudes unchanged", () => {
    expect(normalizeLon(-71, 0)).toBeCloseTo(0, 9);
    expect(normalizeLon(10, -10)).toBeCloseTo(-10, 9);
  });
});

describe("QsoPathProjection", () => {
  it("exposes the normalized remote longitude", () => {
    const p = new QsoPathProjection(40, -98, 35, 138, w, h);
    expect(p.normalizedTheirLon).toBeCloseTo(-222, 9);
  });

  it("projects the endpoint midpoint to the canvas center", () => {
    const p = new QsoPathProjection(42.5, -71, 51.5, 0, w, h);
    const midLon = (-71 + 0) / 2;
    const midLat = (42.5 + 51.5) / 2;
    expect(p.projectX(midLon)).toBeCloseTo(w / 2, 0);
    expect(Math.abs(p.projectX(midLon) - w / 2)).toBeLessThan(tol);
    expect(p.projectY(midLat)).toBeCloseTo(h / 2, 0);
    expect(Math.abs(p.projectY(midLat) - h / 2)).toBeLessThan(tol);
  });

  it("projects an eastern station to the right of a western one", () => {
    const p = new QsoPathProjection(42.5, -71, 51.5, 0, w, h);
    expect(p.projectX(p.normalizedTheirLon)).toBeGreaterThan(p.projectX(-71));
  });

  it("projects a higher latitude higher on screen (smaller y)", () => {
    const p = new QsoPathProjection(42.5, -71, 51.5, 0, w, h);
    expect(p.projectY(51.5)).toBeLessThan(p.projectY(42.5));
  });

  it("clamps nearby stations to the minimum span", () => {
    const p = new QsoPathProjection(40, -75, 40.1, -75.1, w, h);
    const padFactor = 1 + 2 * QSO_MAP_PAD_FRAC;
    const expectedSpan = QSO_MAP_MIN_SPAN_DEG * padFactor;
    const expectedScale = h / expectedSpan;
    expect(p.scale).toBeCloseTo(expectedScale, 6);
  });

  it("keeps both endpoints inside the canvas for a trans-Pacific path", () => {
    const p = new QsoPathProjection(38, -98, 35, 140, w, h);
    const myX = p.projectX(-98);
    const themX = p.projectX(p.normalizedTheirLon);
    expect(myX).toBeGreaterThanOrEqual(0);
    expect(myX).toBeLessThanOrEqual(w);
    expect(themX).toBeGreaterThanOrEqual(0);
    expect(themX).toBeLessThanOrEqual(w);
  });

  it("uses a uniform scale that fits both axes for a wide span", () => {
    const p = new QsoPathProjection(42.5, -71, 51.5, 0, w, h);
    const padFactor = 1 + 2 * QSO_MAP_PAD_FRAC;
    const spanLon = (0 - -71) * padFactor;
    const spanLat = (51.5 - 42.5) * padFactor;
    const maxAllowed = Math.min(w / spanLon, h / spanLat);
    expect(p.scale).toBeCloseTo(maxAllowed, 6);
  });
});
