import { describe, expect, test } from "vitest";
import { forEachRingVertex, parseGeoJsonRings } from "./worldOutlines";

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
