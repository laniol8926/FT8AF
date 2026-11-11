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

// Tile-basemap math (see src-tauri/src/map_tiles.rs's doc comment for how
// the key formula and equirectangular-not-Mercator projection were
// empirically verified against the real bundled archive).
describe("tileLonLatBounds", () => {
  test("z=0 is the whole world", () => {
    expect(tileLonLatBounds(0, 0, 0)).toEqual({ lonMin: -180, lonMax: 180, latMin: -90, latMax: 90 });
  });

  test("z=2 quadrant matches the real archive's verified layout", () => {
    // (0,0): Canadian Arctic archipelago -- NW quadrant.
    expect(tileLonLatBounds(2, 0, 0)).toEqual({ lonMin: -180, lonMax: -90, latMin: 45, latMax: 90 });
    // (0,1): North America -- directly south of (0,0), same longitude band.
    expect(tileLonLatBounds(2, 0, 1)).toEqual({ lonMin: -180, lonMax: -90, latMin: 0, latMax: 45 });
    // (1,0): Greenland -- directly east of (0,0), same latitude band.
    expect(tileLonLatBounds(2, 1, 0)).toEqual({ lonMin: -90, lonMax: 0, latMin: 45, latMax: 90 });
    // (3,3): Antarctica -- the far SE corner.
    expect(tileLonLatBounds(2, 3, 3)).toEqual({ lonMin: 90, lonMax: 180, latMin: -90, latMax: -45 });
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
