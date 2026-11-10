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
