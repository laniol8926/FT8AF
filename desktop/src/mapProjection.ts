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
 * comment for the key formula). A tile `(z,x,y)` sits in a **standard Web
 * Mercator / "slippy map" XYZ** grid over the whole world: `2^z x 2^z`
 * tiles, x increasing eastward from lon -180 as a uniform fraction of the
 * full 360 degrees (no trigonometry needed on that axis), y increasing
 * southward from the Mercator-projected top of the world -- NOT a uniform
 * fraction of the 180-degree latitude range, unlike x's longitude.
 *
 * **This was originally (wrongly) assumed to be plain equirectangular, and
 * that bug shipped and was live-tested as correct before being caught.**
 * The mistake survived casual checks (the equator landed correctly --
 * Mercator and equirect agree exactly at the equator -- and low-zoom tiles
 * "looked" undistorted enough at a glance) but produced a growing
 * north-south misplacement of real raster imagery against anything placed
 * by true lon/lat (state borders, markers), worst at high latitude and
 * invisible at the equator -- exactly Mercator's own distortion curve. It
 * was only caught by precisely overlaying real US state border vectors over
 * the raw tiles and finding them systematically off; switching this
 * function's latitude math to the standard inverse Mercator formula below
 * made every one of those overlays land pixel-perfect (reference points
 * exactly on their real-world features, state lines exactly on the real
 * coastline) at z=2, z=3, and z=4 alike. Do not revert this to a linear
 * `90 - y*(180/n)` formula without redoing that same real-border-overlay
 * proof -- a quadtree parent/child consistency check or a "looks about
 * right" visual glance are both insufficient; they can't tell equirect
 * from Mercator apart (both pyramids are internally self-consistent, and
 * both are hard to eyeball-distinguish without a precise ground truth
 * overlay).
 */
export const MAX_TILE_ZOOM = 4;

export interface TileLonLatBounds {
  lonMin: number;
  lonMax: number;
  latMin: number;
  latMax: number;
}

/** Inverse Web Mercator: `f` is the fraction of the way down the full
 *  projected world (0 = top, 1 = bottom), returns the latitude there. Never
 *  reaches the true poles (the projection is undefined there) -- `f=0`/`f=1`
 *  give the standard ~85.0511 degree Mercator cutoff, which is exactly what
 *  every `f` passed in by `tileLonLatBounds` evaluates to, since `y` only
 *  ever ranges over `0..n` inclusive. */
function mercatorLatAtFraction(f: number): number {
  return (Math.atan(Math.sinh(Math.PI * (1 - 2 * f))) * 180) / Math.PI;
}

/** The lon/lat rectangle tile `(z,x,y)` covers. Does not validate that `x`/`y`
 *  are actually in range for `z` -- callers already clamp `z` via
 *  `tileZoomForViewport` and enumerate `x`/`y` from `0` to `2**z - 1`. */
export function tileLonLatBounds(z: number, x: number, y: number): TileLonLatBounds {
  const n = 2 ** z;
  const lonMin = -180 + x * (360 / n);
  return {
    lonMin,
    lonMax: lonMin + 360 / n,
    latMax: mercatorLatAtFraction(y / n),
    latMin: mercatorLatAtFraction((y + 1) / n),
  };
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
