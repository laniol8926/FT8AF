/**
 * Parse a GeoJSON FeatureCollection of Polygon/MultiPolygon features into a
 * flat list of polygon outer-boundary rings. Each ring is a `number[]` of
 * interleaved `[lon, lat, lon, lat, ...]`. Ported from Android's
 * `WorldOutlines.kt` (`parseGeoJsonRings`); backs the bundled land
 * (`world_land.json`) and country-border (`world_countries.json`) basemap
 * layers, copied verbatim from `ft8af/app/src/main/res/raw/`.
 *
 * Holes (inner rings) are ignored -- negligible for a low-resolution
 * basemap. GeoJSON coordinate order is [lon, lat]; preserved here.
 */
export function parseGeoJsonRings(geojson: unknown): number[][] {
  const rings: number[][] = [];
  const features = (geojson as { features?: unknown[] })?.features ?? [];
  for (const feature of features) {
    const geom = (feature as { geometry?: { type?: string; coordinates?: unknown } })?.geometry;
    if (!geom) continue;
    if (geom.type === "Polygon") {
      const coords = geom.coordinates as number[][][];
      if (coords?.[0]) rings.push(ringToFlat(coords[0]));
    } else if (geom.type === "MultiPolygon") {
      const polys = geom.coordinates as number[][][][];
      for (const poly of polys ?? []) {
        if (poly?.[0]) rings.push(ringToFlat(poly[0]));
      }
    }
  }
  return rings;
}

function ringToFlat(ring: number[][]): number[] {
  const out: number[] = [];
  for (const pt of ring) {
    out.push(pt[0], pt[1]); // lon, lat
  }
  return out;
}

/**
 * Invokes `plot` once per complete (lon, lat) vertex of a flattened polygon
 * `ring` (`[lon0, lat0, lon1, lat1, ...]`), passing `first === true` on the
 * opening vertex. Returns `true` when at least one vertex was plotted (the
 * caller should close the path), or `false` when the ring has fewer than
 * `minVertices` complete vertices.
 */
export function forEachRingVertex(
  ring: number[],
  minVertices: number,
  plot: (lon: number, lat: number, first: boolean) => void,
): boolean {
  const vertices = Math.floor(ring.length / 2);
  if (vertices === 0 || vertices < minVertices) return false;
  for (let v = 0; v < vertices; v++) {
    plot(ring[v * 2], ring[v * 2 + 1], v === 0);
  }
  return true;
}
