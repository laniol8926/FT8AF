/**
 * Map coordinate math: the equirectangular (Plate Carrée) projection and
 * great-circle bearing. Ported from the Android app's `MapScreen.kt`
 * (`equirectProject`/`computeBearing`) — see `MapProjectionTest.kt` for the
 * reference test cases this mirrors.
 *
 * The azimuthal-equidistant projection, range rings, and `greatCircleKm`
 * are deliberately not ported here: v1 of the desktop map is
 * equirectangular-only (see docs/desktop-decode-android-parity.md), and
 * `hunt.ts` already has an equivalent great-circle distance
 * (`haversineKm`) used elsewhere in this app -- no need for a second one.
 */

export interface ProjectedPoint {
  x: number;
  y: number;
}

/** lat/lon (degrees) -> normalized [-1, 1] on each axis. lon=±180 -> x=±1;
 *  lat=90 (north) -> y=-1 (top of screen), lat=-90 -> y=1 (bottom). */
export function equirectProject(lat: number, lon: number): ProjectedPoint {
  return { x: lon / 180, y: -lat / 90 };
}

function toRad(deg: number): number {
  return (deg * Math.PI) / 180;
}
function toDeg(rad: number): number {
  return (rad * 180) / Math.PI;
}

/** Initial great-circle bearing (degrees, 0-360, 0=north) from point 1 to point 2. */
export function computeBearing(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const phi1 = toRad(lat1);
  const phi2 = toRad(lat2);
  const dLam = toRad(lon2 - lon1);
  const y = Math.sin(dLam) * Math.cos(phi2);
  const x = Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLam);
  const bearing = toDeg(Math.atan2(y, x));
  return (bearing + 360) % 360;
}

/**
 * Offline satellite tile basemap (see `src-tauri/src/map_tiles.rs`'s doc
 * comment for how the archive's key formula and equirectangular-not-Mercator
 * projection were empirically verified). A tile `(z,x,y)` sits in a plain
 * `2^z x 2^z` grid over the whole equirectangular world: x increases
 * eastward from lon -180, y increases *southward* from lat +90 -- so its
 * bounds are a uniform fraction of the full lon/lat range, no trigonometry
 * involved (unlike a Mercator tile scheme).
 */
export const MAX_TILE_ZOOM = 4;

export interface TileLonLatBounds {
  lonMin: number;
  lonMax: number;
  latMin: number;
  latMax: number;
}

/** The lon/lat rectangle tile `(z,x,y)` covers. Does not validate that `x`/`y`
 *  are actually in range for `z` -- callers already clamp `z` via
 *  `tileZoomForViewport` and enumerate `x`/`y` from `0` to `2**z - 1`. */
export function tileLonLatBounds(z: number, x: number, y: number): TileLonLatBounds {
  const n = 2 ** z;
  const lonMin = -180 + x * (360 / n);
  const latMax = 90 - y * (180 / n);
  return { lonMin, lonMax: lonMin + 360 / n, latMin: latMax - 180 / n, latMax };
}

/** Which stored zoom level (0-4) best matches the viewport's current world
 *  pixel width (`EquirectViewport.worldPxW`) -- each tile is 256px square at
 *  its native resolution, so `worldPxW / 256` tiles span the whole world
 *  horizontally at a 1:1 pixel match; round to the nearest integer zoom and
 *  clamp to the archive's stored range (deeper zoom just upsamples z4,
 *  matching Android's own `setMaxZoomLevel(6.0)` over the same z0-4 data). */
export function tileZoomForViewport(worldPxW: number): number {
  const z = Math.round(Math.log2(worldPxW / 256));
  return Math.min(MAX_TILE_ZOOM, Math.max(0, z));
}
