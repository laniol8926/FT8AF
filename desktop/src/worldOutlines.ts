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

/**
 * US state border/label support for the Map screen's Azimuthal and
 * equirectangular views alike. Ported from Android's `WorldOutlines.kt`
 * (`US_STATE_ABBR`/`parseStateLabels`/`largestOuterRing`/`ringSignedArea`/
 * `ringCentroid`/`stateBorderAlpha`). Scoped to borders + labels only --
 * `WorldOutlines.kt`'s `regionNameAt`/`UsStateLocator` point-in-polygon
 * "which state is this point in" query is deliberately not ported here:
 * desktop already answers that differently, via a backend grid-square to
 * state lookup (`us_states.rs`/`us_grid_states.json`) for the Decode
 * list's New-State filter -- unrelated to map rendering, left untouched.
 */
const US_STATE_ABBR: Record<string, string> = {
  Alabama: "AL",
  Alaska: "AK",
  Arizona: "AZ",
  Arkansas: "AR",
  California: "CA",
  Colorado: "CO",
  Connecticut: "CT",
  Delaware: "DE",
  "District of Columbia": "DC",
  Florida: "FL",
  Georgia: "GA",
  Hawaii: "HI",
  Idaho: "ID",
  Illinois: "IL",
  Indiana: "IN",
  Iowa: "IA",
  Kansas: "KS",
  Kentucky: "KY",
  Louisiana: "LA",
  Maine: "ME",
  Maryland: "MD",
  Massachusetts: "MA",
  Michigan: "MI",
  Minnesota: "MN",
  Mississippi: "MS",
  Missouri: "MO",
  Montana: "MT",
  Nebraska: "NE",
  Nevada: "NV",
  "New Hampshire": "NH",
  "New Jersey": "NJ",
  "New Mexico": "NM",
  "New York": "NY",
  "North Carolina": "NC",
  "North Dakota": "ND",
  Ohio: "OH",
  Oklahoma: "OK",
  Oregon: "OR",
  Pennsylvania: "PA",
  "Rhode Island": "RI",
  "South Carolina": "SC",
  "South Dakota": "SD",
  Tennessee: "TN",
  Texas: "TX",
  Utah: "UT",
  Vermont: "VT",
  Virginia: "VA",
  Washington: "WA",
  "West Virginia": "WV",
  Wisconsin: "WI",
  Wyoming: "WY",
};

export interface StateLabel {
  abbr: string;
  lon: number;
  lat: number;
}

/** Shoelace-formula signed area of a flat `[lon,lat,...]` ring -- sign
 *  encodes winding direction, magnitude is twice the real area (the `/2`
 *  and absolute value are left to callers that need the real area, same
 *  as `ringCentroid` below). */
export function ringSignedArea(ring: number[]): number {
  const vertices = Math.floor(ring.length / 2);
  let sum = 0;
  for (let i = 0; i < vertices; i++) {
    const j = (i + 1) % vertices;
    sum += ring[i * 2] * ring[j * 2 + 1] - ring[j * 2] * ring[i * 2 + 1];
  }
  return sum / 2;
}

/** Area-weighted centroid of a flat `[lon,lat,...]` ring (shoelace-style),
 *  falling back to the plain vertex mean on a degenerate (near-zero-area)
 *  ring. Returns `null` for an empty ring. */
export function ringCentroid(ring: number[]): { lon: number; lat: number } | null {
  const vertices = Math.floor(ring.length / 2);
  if (vertices === 0) return null;
  let doubleArea = 0;
  let cx = 0;
  let cy = 0;
  for (let i = 0; i < vertices; i++) {
    const j = (i + 1) % vertices;
    const x0 = ring[i * 2];
    const y0 = ring[i * 2 + 1];
    const x1 = ring[j * 2];
    const y1 = ring[j * 2 + 1];
    const cross = x0 * y1 - x1 * y0;
    doubleArea += cross;
    cx += (x0 + x1) * cross;
    cy += (y0 + y1) * cross;
  }
  if (Math.abs(doubleArea) < 1e-9) {
    let sx = 0;
    let sy = 0;
    for (let i = 0; i < vertices; i++) {
      sx += ring[i * 2];
      sy += ring[i * 2 + 1];
    }
    return { lon: sx / vertices, lat: sy / vertices };
  }
  return { lon: cx / (3 * doubleArea), lat: cy / (3 * doubleArea) };
}

/** The outer ring with the largest absolute area among a GeoJSON feature's
 *  geometry (its one ring for a Polygon, or whichever polygon has the
 *  largest outer ring for a MultiPolygon) -- the ring worth placing a
 *  single label on. */
function largestOuterRing(geom: { type?: string; coordinates?: unknown }): number[] | null {
  if (geom.type === "Polygon") {
    const coords = geom.coordinates as number[][][];
    return coords?.[0] ? ringToFlat(coords[0]) : null;
  }
  if (geom.type === "MultiPolygon") {
    const polys = geom.coordinates as number[][][][];
    let best: number[] | null = null;
    let bestArea = -1;
    for (const poly of polys ?? []) {
      if (!poly?.[0]) continue;
      const ring = ringToFlat(poly[0]);
      const area = Math.abs(ringSignedArea(ring));
      if (area > bestArea) {
        bestArea = area;
        best = ring;
      }
    }
    return best;
  }
  return null;
}

/** Parses `us_states.json`-shaped GeoJSON into one label per recognized
 *  state (skipping any feature whose `properties.name` isn't in
 *  `US_STATE_ABBR` -- there shouldn't be any in the real bundled file,
 *  but a stray/renamed feature should be dropped, not crash). */
export function parseStateLabels(geojson: unknown): StateLabel[] {
  const labels: StateLabel[] = [];
  const features = (geojson as { features?: unknown[] })?.features ?? [];
  for (const feature of features) {
    const name = (feature as { properties?: { name?: string } })?.properties?.name;
    if (!name) continue;
    const abbr = US_STATE_ABBR[name];
    if (!abbr) continue;
    const geom = (feature as { geometry?: { type?: string; coordinates?: unknown } })?.geometry;
    if (!geom) continue;
    const ring = largestOuterRing(geom);
    if (!ring) continue;
    const centroid = ringCentroid(ring);
    if (!centroid) continue;
    labels.push({ abbr, lon: centroid.lon, lat: centroid.lat });
  }
  return labels;
}

const STATE_BORDER_MIN_PX_PER_DEG = 3;
const STATE_BORDER_FULL_PX_PER_DEG = 6;

/** Linear fade for the state-border layer, driven by basemap pixel
 *  density rather than a hard zoom-level gate (a hard gate under-fires
 *  for a US operator's default auto-framed view, per Android's own design
 *  note) -- 0 at/below `STATE_BORDER_MIN_PX_PER_DEG`, 1 at/above
 *  `STATE_BORDER_FULL_PX_PER_DEG`, linear between. */
export function stateBorderAlpha(pxPerLonDegree: number): number {
  if (pxPerLonDegree <= STATE_BORDER_MIN_PX_PER_DEG) return 0;
  if (pxPerLonDegree >= STATE_BORDER_FULL_PX_PER_DEG) return 1;
  return (
    (pxPerLonDegree - STATE_BORDER_MIN_PX_PER_DEG) /
    (STATE_BORDER_FULL_PX_PER_DEG - STATE_BORDER_MIN_PX_PER_DEG)
  );
}
