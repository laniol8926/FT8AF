import { describe, expect, test } from "vitest";
import { azProject, compassVector, MAP_EDGE_KM, rangeRingRadiusPx, RANGE_RING_KMS } from "./azimuthalProjection";
import { computeBearing } from "./mapProjection";

describe("azProject", () => {
  test("the operator's own position projects to the origin with zero distance", () => {
    const p = azProject(40.7, -74.0, 40.7, -74.0);
    expect(p.x).toBeCloseTo(0, 6);
    expect(p.y).toBeCloseTo(0, 6);
    expect(p.distKm).toBeCloseTo(0, 6);
  });

  test("the antipode projects to the disc edge (normalized radius 1)", () => {
    const p = azProject(0, 0, 0, 180);
    const r = Math.sqrt(p.x ** 2 + p.y ** 2);
    expect(r).toBeCloseTo(1, 3);
    expect(p.distKm).toBeCloseTo(MAP_EDGE_KM, 0);
  });

  // The real rigor the direction arrows need: cross-check azProject's own
  // resulting screen angle against the already-tested computeBearing, for
  // several real city pairs spanning all four quadrants. These are two
  // independently-implemented formulas describing the same real-world
  // geometry -- if either had a sign/axis error, they'd disagree. atan2(x,
  // -y) is the inverse of compassVector's dx=sin(θ), dy=-cos(θ): solving
  // for θ given (dx, dy) = (sin θ, -cos θ) gives θ = atan2(dx, -dy).
  function screenAngleDeg(x: number, y: number): number {
    const deg = (Math.atan2(x, -y) * 180) / Math.PI;
    return ((deg % 360) + 360) % 360;
  }

  const cityPairs: { name: string; op: [number, number]; station: [number, number] }[] = [
    { name: "NY -> London (NE quadrant)", op: [40.7, -74.0], station: [51.5, -0.1] },
    { name: "NY -> Rio (SE quadrant)", op: [40.7, -74.0], station: [-22.9, -43.2] },
    { name: "NY -> Lima (SW-ish)", op: [40.7, -74.0], station: [-12.0, -77.0] },
    { name: "NY -> Anchorage (NW quadrant)", op: [40.7, -74.0], station: [61.2, -149.9] },
    { name: "Sydney -> Tokyo", op: [-33.9, 151.2], station: [35.7, 139.7] },
  ];

  test.each(cityPairs)("$name: azProject's angle matches computeBearing", ({ op, station }) => {
    const [opLat, opLon] = op;
    const [lat, lon] = station;
    const p = azProject(opLat, opLon, lat, lon);
    const bearing = computeBearing(opLat, opLon, lat, lon);
    const fromProjection = screenAngleDeg(p.x, p.y);
    // Allow a small tolerance for floating-point/angle-wraparound noise.
    const diff = Math.min(Math.abs(fromProjection - bearing), 360 - Math.abs(fromProjection - bearing));
    expect(diff).toBeLessThan(1);
  });
});

describe("compassVector -- self-verified against the 4 cardinals before use", () => {
  test("N points straight up on screen", () => {
    const v = compassVector(0);
    expect(v.dx).toBeCloseTo(0, 6);
    expect(v.dy).toBeCloseTo(-1, 6);
  });
  test("E points straight right", () => {
    const v = compassVector(90);
    expect(v.dx).toBeCloseTo(1, 6);
    expect(v.dy).toBeCloseTo(0, 6);
  });
  test("S points straight down", () => {
    const v = compassVector(180);
    expect(v.dx).toBeCloseTo(0, 6);
    expect(v.dy).toBeCloseTo(1, 6);
  });
  test("W points straight left", () => {
    const v = compassVector(270);
    expect(v.dx).toBeCloseTo(-1, 6);
    expect(v.dy).toBeCloseTo(0, 6);
  });
  test("NE points up and to the right", () => {
    const v = compassVector(45);
    expect(v.dx).toBeGreaterThan(0);
    expect(v.dy).toBeLessThan(0);
  });
});

describe("rangeRingRadiusPx", () => {
  test("a ring at the disc edge distance has radius exactly discRadiusPx*scale", () => {
    expect(rangeRingRadiusPx(MAP_EDGE_KM, 200, 1)).toBeCloseTo(200, 6);
    expect(rangeRingRadiusPx(MAP_EDGE_KM, 200, 2)).toBeCloseTo(400, 6);
  });

  test("half the edge distance is half the radius (linear scaling)", () => {
    const full = rangeRingRadiusPx(MAP_EDGE_KM, 200, 1);
    const half = rangeRingRadiusPx(MAP_EDGE_KM / 2, 200, 1);
    expect(half).toBeCloseTo(full / 2, 6);
  });

  test("the five fixed range rings are all within the disc for any real distance", () => {
    for (const km of RANGE_RING_KMS) {
      expect(rangeRingRadiusPx(km, 200, 1)).toBeLessThanOrEqual(200);
    }
  });
});
