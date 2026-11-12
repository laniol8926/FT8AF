import { describe, expect, test } from "vitest";
import {
  forEachRingVertex,
  parseGeoJsonRings,
  parseStateLabels,
  ringCentroid,
  ringSignedArea,
  stateBorderAlpha,
} from "./worldOutlines";

// Ported from Android's GeoOutlinesParseTest.kt.

test("polygon yields one ring with interleaved lon lat in order", () => {
  const geojson = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: {},
        geometry: { type: "Polygon", coordinates: [[[-87.0, 35.0], [-85.0, 34.0], [-86.0, 33.0]]] },
      },
    ],
  };
  const rings = parseGeoJsonRings(geojson);
  expect(rings).toHaveLength(1);
  expect(rings[0]).toEqual([-87.0, 35.0, -85.0, 34.0, -86.0, 33.0]);
});

test("multipolygon yields one outer ring per sub-polygon", () => {
  const geojson = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: {},
        geometry: {
          type: "MultiPolygon",
          coordinates: [
            [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
            [[[10.0, 10.0], [11.0, 10.0], [11.0, 11.0]]],
          ],
        },
      },
    ],
  };
  const rings = parseGeoJsonRings(geojson);
  expect(rings).toHaveLength(2);
  expect(rings[0][0]).toBeCloseTo(0.0, 4);
  expect(rings[1][0]).toBeCloseTo(10.0, 4);
});

test("inner rings (holes) are ignored", () => {
  const geojson = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: {},
        geometry: {
          type: "Polygon",
          coordinates: [
            [[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
            [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0]],
          ],
        },
      },
    ],
  };
  const rings = parseGeoJsonRings(geojson);
  expect(rings).toHaveLength(1);
  expect(rings[0]).toHaveLength(8);
});

test("non-polygon geometries are skipped", () => {
  const geojson = {
    type: "FeatureCollection",
    features: [
      { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: [[0.0, 0.0], [1.0, 1.0]] } },
      { type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0]]] } },
    ],
  };
  const rings = parseGeoJsonRings(geojson);
  expect(rings).toHaveLength(1);
  expect(rings[0][0]).toBeCloseTo(2.0, 4);
});

test("empty feature collection yields no rings", () => {
  expect(parseGeoJsonRings({ type: "FeatureCollection", features: [] })).toEqual([]);
});

describe("forEachRingVertex", () => {
  test("plots each vertex, marks the first, and returns true", () => {
    const seen: Array<[number, number, boolean]> = [];
    const plotted = forEachRingVertex([1, 2, 3, 4, 5, 6], 3, (lon, lat, first) => {
      seen.push([lon, lat, first]);
    });
    expect(plotted).toBe(true);
    expect(seen).toEqual([[1, 2, true], [3, 4, false], [5, 6, false]]);
  });

  test("returns false and skips when below minVertices", () => {
    const seen: number[] = [];
    const plotted = forEachRingVertex([1, 2, 3, 4], 3, (lon) => seen.push(lon));
    expect(plotted).toBe(false);
    expect(seen).toEqual([]);
  });

  test("returns false for an empty ring", () => {
    expect(forEachRingVertex([], 3, () => {})).toBe(false);
  });
});

// Unit square ring, counter-clockwise: (0,0) (1,0) (1,1) (0,1).
const UNIT_SQUARE = [0, 0, 1, 0, 1, 1, 0, 1];

describe("ringSignedArea", () => {
  test("a unit square has area magnitude 1", () => {
    expect(Math.abs(ringSignedArea(UNIT_SQUARE))).toBeCloseTo(1, 6);
  });

  test("winding direction flips the sign", () => {
    const reversed = [0, 0, 0, 1, 1, 1, 1, 0];
    expect(Math.sign(ringSignedArea(UNIT_SQUARE))).toBe(-Math.sign(ringSignedArea(reversed)));
  });
});

describe("ringCentroid", () => {
  test("a unit square's centroid is its exact geometric center", () => {
    const c = ringCentroid(UNIT_SQUARE);
    expect(c).not.toBeNull();
    expect(c!.lon).toBeCloseTo(0.5, 6);
    expect(c!.lat).toBeCloseTo(0.5, 6);
  });

  test("falls back to the plain vertex mean for a degenerate (zero-area) ring", () => {
    // All three points colinear -- zero area.
    const degenerate = [0, 0, 1, 0, 2, 0];
    const c = ringCentroid(degenerate);
    expect(c).not.toBeNull();
    expect(c!.lon).toBeCloseTo(1, 6); // mean of 0,1,2
    expect(c!.lat).toBeCloseTo(0, 6);
  });

  test("returns null for an empty ring", () => {
    expect(ringCentroid([])).toBeNull();
  });
});

describe("parseStateLabels", () => {
  function stateFeature(name: string, ring: number[][]) {
    return { type: "Feature", properties: { name }, geometry: { type: "Polygon", coordinates: [ring] } };
  }

  test("a recognized state gets a label at its ring centroid", () => {
    const geojson = {
      type: "FeatureCollection",
      features: [stateFeature("Colorado", [[-106, 37], [-102, 37], [-102, 41], [-106, 41]])],
    };
    const labels = parseStateLabels(geojson);
    expect(labels).toHaveLength(1);
    expect(labels[0].abbr).toBe("CO");
    expect(labels[0].lon).toBeCloseTo(-104, 1);
    expect(labels[0].lat).toBeCloseTo(39, 1);
  });

  test("an unrecognized name is skipped, not crashed on", () => {
    const geojson = {
      type: "FeatureCollection",
      features: [stateFeature("Narnia", [[0, 0], [1, 0], [1, 1]])],
    };
    expect(parseStateLabels(geojson)).toEqual([]);
  });

  test("a MultiPolygon state picks its largest outer ring for the label", () => {
    const geojson = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { name: "Hawaii" },
          geometry: {
            type: "MultiPolygon",
            coordinates: [
              [[[-156, 20.5], [-155.9, 20.5], [-155.9, 20.6], [-156, 20.6]]], // tiny island
              [[[-160, 21], [-154, 21], [-154, 23], [-160, 23]]], // the big island, much larger area
            ],
          },
        },
      ],
    };
    const labels = parseStateLabels(geojson);
    expect(labels).toHaveLength(1);
    expect(labels[0].abbr).toBe("HI");
    // Centroid should land near the big polygon, not the tiny island.
    expect(labels[0].lon).toBeCloseTo(-157, 0);
  });
});

test("the real bundled us_states.json resolves to exactly the 50 states + DC", async () => {
  const usStates = (await import("./map-data/us_states.json")).default;
  const labels = parseStateLabels(usStates);
  expect(labels).toHaveLength(51);
  const abbrs = new Set(labels.map((l) => l.abbr));
  expect(abbrs.has("CA")).toBe(true);
  expect(abbrs.has("DC")).toBe(true);
  expect(abbrs.has("TX")).toBe(true);
  // Every abbreviation should be unique (no duplicate labels for one state).
  expect(abbrs.size).toBe(51);
});

describe("stateBorderAlpha", () => {
  test("fully transparent at/below the minimum density", () => {
    expect(stateBorderAlpha(3)).toBe(0);
    expect(stateBorderAlpha(1)).toBe(0);
  });

  test("fully opaque at/above the full density", () => {
    expect(stateBorderAlpha(6)).toBe(1);
    expect(stateBorderAlpha(20)).toBe(1);
  });

  test("linear fade between the two thresholds", () => {
    expect(stateBorderAlpha(4.5)).toBeCloseTo(0.5, 6);
  });
});
