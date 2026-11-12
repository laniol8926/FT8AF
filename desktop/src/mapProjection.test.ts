import { describe, expect, test } from "vitest";
import { computeBearing, equirectProject, tileLonLatBounds, tileZoomForViewport } from "./mapProjection";

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

// Tile-basemap math. The archive is standard Web Mercator, not plain
// equirectangular -- see tileLonLatBounds's doc comment (mapProjection.ts)
// for how a wrong equirect assumption shipped, live-tested as correct, and
// was only caught by a precise real-border-overlay proof. These expected
// values are the standard inverse-Mercator latitudes at each tile edge
// (atan(sinh(pi*(1-2f)))*180/pi for f = y/n), not a linear 180/n split.
describe("tileLonLatBounds", () => {
  test("z=0 is the whole world (Mercator-cropped at the poles, not +-90)", () => {
    const b = tileLonLatBounds(0, 0, 0);
    expect(b.lonMin).toBe(-180);
    expect(b.lonMax).toBe(180);
    // Standard Web Mercator cutoff -- the projection is undefined at the
    // true poles, so every tile scheme built on it stops here instead.
    expect(b.latMax).toBeCloseTo(85.051129, 5);
    expect(b.latMin).toBeCloseTo(-85.051129, 5);
  });

  test("z=2 quadrant matches the real archive's verified layout", () => {
    // (0,0): Canadian Arctic archipelago -- NW quadrant.
    const nw = tileLonLatBounds(2, 0, 0);
    expect(nw.lonMin).toBe(-180);
    expect(nw.lonMax).toBe(-90);
    expect(nw.latMax).toBeCloseTo(85.051129, 5);
    expect(nw.latMin).toBeCloseTo(66.513260, 5);
    // (0,1): North America -- directly south of (0,0), same longitude band.
    const sw = tileLonLatBounds(2, 0, 1);
    expect(sw.lonMin).toBe(-180);
    expect(sw.lonMax).toBe(-90);
    expect(sw.latMax).toBeCloseTo(66.513260, 5);
    expect(sw.latMin).toBeCloseTo(0, 5);
    // (1,0): Greenland -- directly east of (0,0), same latitude band.
    const ne = tileLonLatBounds(2, 1, 0);
    expect(ne.lonMin).toBe(-90);
    expect(ne.lonMax).toBe(0);
    expect(ne.latMax).toBeCloseTo(85.051129, 5);
    expect(ne.latMin).toBeCloseTo(66.513260, 5);
    // (3,3): Antarctica -- the far SE corner.
    const se = tileLonLatBounds(2, 3, 3);
    expect(se.lonMin).toBe(90);
    expect(se.lonMax).toBe(180);
    expect(se.latMax).toBeCloseTo(-66.513260, 5);
    expect(se.latMin).toBeCloseTo(-85.051129, 5);
  });

  test("tile rows get geographically shorter away from the equator (Mercator, not linear)", () => {
    // A linear split would make every z=4 row exactly 11.25 degrees tall.
    // Mercator rows shrink in degree-span as they approach the poles.
    const equatorRow = tileLonLatBounds(4, 0, 7); // straddles the equator region
    const polarRow = tileLonLatBounds(4, 0, 0); // top row, nearest the pole
    const equatorSpan = equatorRow.latMax - equatorRow.latMin;
    const polarSpan = polarRow.latMax - polarRow.latMin;
    expect(polarSpan).toBeLessThan(equatorSpan);
  });
});

describe("tileZoomForViewport", () => {
  test("picks the nearest zoom to a 1:1 pixel match", () => {
    expect(tileZoomForViewport(256)).toBe(0); // one 256px tile spans the world
    expect(tileZoomForViewport(512)).toBe(1);
    expect(tileZoomForViewport(1024)).toBe(2);
  });

  test("clamps to the archive's stored range", () => {
    expect(tileZoomForViewport(1)).toBe(0); // far too small to clamp negative
    expect(tileZoomForViewport(1_000_000)).toBe(4); // far beyond the stored max
  });
});
