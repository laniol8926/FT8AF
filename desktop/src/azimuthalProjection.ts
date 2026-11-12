/**
 * Azimuthal-equidistant ("radar") projection math for the Map screen's
 * Azimuthal view: a great-circle-centered disc, the operator always at its
 * middle, with range rings and compass bearings. Ported from Android's
 * `MapScreen.kt` (`azProject`/`MAP_EDGE_KM`/`rangeRingRadiusPx`/
 * `drawCompassBearings`).
 *
 * The whole globe maps into the unit disc exactly (radius 1 = the
 * antipode) -- unlike the equirectangular tiles (phase 1), there is no
 * valid projection beyond that radius, so a renderer using this must clip
 * to the disc.
 */

export interface AzimuthalPoint {
  /** Normalized [-1,1] on each axis -- x east/west, y north/south (canvas
   *  convention: +y is down, so north is negative y, matching
   *  `equirectProject`'s own sign convention). */
  x: number;
  y: number;
  /** Great-circle distance from the projection's center, in km. */
  distKm: number;
}

const EARTH_RADIUS_KM = 6371;
/** The great-circle distance the disc's edge (normalized radius 1)
 *  represents -- the antipode, half the Earth's circumference. */
export const MAP_EDGE_KM = Math.PI * EARTH_RADIUS_KM;

function toRad(deg: number): number {
  return (deg * Math.PI) / 180;
}

/**
 * Azimuthal-equidistant projection of `(lat, lon)` centered on
 * `(opLat, opLon)`. Standard spherical trig: `c` is the angular great-circle
 * distance, `k = c/sin(c)` the radial scale factor. A point coincident with
 * the center (`c` below float precision) projects to the origin with zero
 * distance rather than dividing by `sin(0) = 0`.
 */
export function azProject(opLat: number, opLon: number, lat: number, lon: number): AzimuthalPoint {
  const phi1 = toRad(opLat);
  const phi2 = toRad(lat);
  const dLam = toRad(lon - opLon);
  const cosC = Math.sin(phi1) * Math.sin(phi2) + Math.cos(phi1) * Math.cos(phi2) * Math.cos(dLam);
  const c = Math.acos(Math.min(1, Math.max(-1, cosC)));
  if (c < 1e-10) return { x: 0, y: 0, distKm: 0 };
  const k = c / Math.sin(c);
  const x = k * Math.cos(phi2) * Math.sin(dLam);
  const y = k * (Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLam));
  return { x: x / Math.PI, y: -y / Math.PI, distKm: c * EARTH_RADIUS_KM };
}

/** Screen-pixel radius of a labeled range ring at `km`, using the same
 *  km-to-normalized-radius scale `azProject` uses, so rings and markers
 *  always agree. */
export function rangeRingRadiusPx(km: number, discRadiusPx: number, scale: number): number {
  return (km / MAP_EDGE_KM) * discRadiusPx * scale;
}

/** The five fixed range rings Android's azimuthal view always draws. */
export const RANGE_RING_KMS = [2500, 5000, 10000, 15000, 20000];

export interface CompassDirection {
  label: string;
  /** Compass bearing in degrees, 0 = north, clockwise. */
  angleDeg: number;
  /** Cardinal (N/E/S/W) directions get a bolder line/label than the
   *  intercardinals (NE/SE/SW/NW). */
  cardinal: boolean;
}

export const COMPASS_DIRECTIONS: CompassDirection[] = [
  { label: "N", angleDeg: 0, cardinal: true },
  { label: "NE", angleDeg: 45, cardinal: false },
  { label: "E", angleDeg: 90, cardinal: true },
  { label: "SE", angleDeg: 135, cardinal: false },
  { label: "S", angleDeg: 180, cardinal: true },
  { label: "SW", angleDeg: 225, cardinal: false },
  { label: "W", angleDeg: 270, cardinal: true },
  { label: "NW", angleDeg: 315, cardinal: false },
];

/**
 * The screen-space unit vector for compass bearing `angleDeg` (0=N,
 * clockwise), on a canvas where +y is down. Derived and self-verified
 * from first principles, not transcribed from Android's own internal
 * rotation: `dx = sin(θ), dy = -cos(θ)` -- checked against all 4 cardinals
 * in `azimuthalProjection.test.ts` (N -> straight up, E -> straight right,
 * S -> straight down, W -> straight left), matching standard north-up map
 * convention, before this is used to draw anything.
 */
export function compassVector(angleDeg: number): { dx: number; dy: number } {
  const rad = toRad(angleDeg);
  return { dx: Math.sin(rad), dy: -Math.cos(rad) };
}
