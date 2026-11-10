import { describe, expect, test } from "vitest";
import { computeBearing, equirectProject } from "./mapProjection";

// Ported from Android's MapProjectionTest.kt (equirectProject/computeBearing
// cases only -- azProject/rangeRingRadiusPx/greatCircleKm aren't ported, see
// mapProjection.ts's doc comment).

describe("equirectProject", () => {
  test("maps corners to normalized extents", () => {
    const origin = equirectProject(0, 0);
    expect(origin.x).toBeCloseTo(0, 6);
    expect(origin.y).toBeCloseTo(0, 6);

    const corner = equirectProject(90, 180);
    expect(corner.x).toBeCloseTo(1, 6);
    expect(corner.y).toBeCloseTo(-1, 6);
  });
});

describe("computeBearing", () => {
  test("due east is 90", () => {
    expect(computeBearing(0, 0, 0, 90)).toBeCloseTo(90, 6);
  });

  test("due north is 0", () => {
    expect(computeBearing(0, 0, 90, 0)).toBeCloseTo(0, 6);
  });
});
