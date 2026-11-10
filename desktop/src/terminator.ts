/**
 * Day/night terminator ("gray line") geometry for the world map. Ported
 * from Android's `Terminator.kt` -- the NOAA low-precision solar-position
 * algorithm, accurate to a few tenths of a degree, plenty for a gray-line
 * overlay that only has to look right.
 *
 * All angles are degrees unless noted; longitudes are east-positive in
 * [-180, 180]; times are UTC epoch milliseconds.
 */

/** The point on Earth directly beneath the Sun (Sun at the zenith). */
export interface SubsolarPoint {
  lat: number;
  lon: number;
}

function toRad(deg: number): number {
  return (deg * Math.PI) / 180;
}
function toDeg(rad: number): number {
  return (rad * 180) / Math.PI;
}

/** Wrap a longitude into [-180, 180]. */
export function normalizeLon(lon: number): number {
  let l = lon % 360;
  if (l > 180) l -= 360;
  if (l < -180) l += 360;
  return l;
}

/**
 * Sub-solar point for a UTC instant, via the NOAA low-precision
 * solar-position algorithm.
 */
export function subsolarPoint(utcMillis: number): SubsolarPoint {
  const d = new Date(utcMillis);
  const startOfYearUtc = Date.UTC(d.getUTCFullYear(), 0, 1);
  const dayOfYear = Math.floor((utcMillis - startOfYearUtc) / 86400000) + 1;
  const hoursUtc = d.getUTCHours() + d.getUTCMinutes() / 60 + d.getUTCSeconds() / 3600;

  // Fractional-year angle γ (radians): NOAA uses (day - 1) + (hour - 12) / 24.
  const gamma = ((2 * Math.PI) / 365) * (dayOfYear - 1 + (hoursUtc - 12) / 24);

  // Equation of time (minutes) -- the Sun's longitude wobble vs. clock time.
  const eqTime =
    229.18 *
    (0.000075 +
      0.001868 * Math.cos(gamma) -
      0.032077 * Math.sin(gamma) -
      0.014615 * Math.cos(2 * gamma) -
      0.040849 * Math.sin(2 * gamma));
  // Solar declination (radians) = sub-solar latitude.
  const declRad =
    0.006918 -
    0.399912 * Math.cos(gamma) +
    0.070257 * Math.sin(gamma) -
    0.006758 * Math.cos(2 * gamma) +
    0.000907 * Math.sin(2 * gamma) -
    0.002697 * Math.cos(3 * gamma) +
    0.00148 * Math.sin(3 * gamma);

  const subsolarLat = toDeg(declRad);
  // The Sun is overhead at true solar noon; each UTC hour past noon moves
  // that meridian 15° west. The equation of time nudges it off clock noon.
  const subsolarLon = normalizeLon(-15 * (hoursUtc + eqTime / 60 - 12));
  return { lat: subsolarLat, lon: subsolarLon };
}

/** Solar elevation (degrees) at lat/lon given the subsolar point: positive =
 *  daytime, negative = night, ≈0 = on the terminator (the gray line itself). */
export function solarElevationDeg(lat: number, lon: number, subsolar: SubsolarPoint): number {
  const phi = toRad(lat);
  const decl = toRad(subsolar.lat);
  const hourAngle = toRad(lon - subsolar.lon);
  const sinElev = Math.sin(phi) * Math.sin(decl) + Math.cos(phi) * Math.cos(decl) * Math.cos(hourAngle);
  return toDeg(Math.asin(Math.min(Math.max(sinElev, -1), 1)));
}

/** Whether lat/lon is on the night side (Sun below the horizon). */
export function isNight(lat: number, lon: number, subsolar: SubsolarPoint): boolean {
  return solarElevationDeg(lat, lon, subsolar) < 0;
}

/** Floor on |declination| used by `terminatorLatitude` to survive the equinox. */
const MIN_DECL_DEG = 0.3;

/**
 * The terminator latitude at a given lon -- the latitude where the Sun sits
 * on the horizon. From sin φ·sin δ + cos φ·cos δ·cos H = 0 → tan φ = −cos H / tan δ.
 *
 * Near the equinoxes δ → 0 makes the true terminator two vertical meridians
 * with no single latitude per longitude; |δ| is floored to MIN_DECL_DEG so
 * the curve stays steep-but-finite instead of blowing up.
 */
export function terminatorLatitude(lon: number, subsolar: SubsolarPoint): number {
  let declDeg = subsolar.lat;
  if (declDeg >= 0 && declDeg < MIN_DECL_DEG) declDeg = MIN_DECL_DEG;
  else if (declDeg < 0 && declDeg > -MIN_DECL_DEG) declDeg = -MIN_DECL_DEG;
  const decl = toRad(declDeg);
  const hourAngle = toRad(lon - subsolar.lon);
  return toDeg(Math.atan(-Math.cos(hourAngle) / Math.tan(decl)));
}

/**
 * The terminator as a flat `[lon0, lat0, lon1, lat1, …]` polyline sampled
 * every `stepDeg` degrees of longitude from -180 to +180.
 */
export function terminatorCurveLatLon(subsolar: SubsolarPoint, stepDeg = 3.0): number[] {
  const step = stepDeg <= 0 ? 3.0 : stepDeg;
  const out: number[] = [];
  let lon = -180;
  while (lon <= 180 + 1e-9) {
    out.push(lon, terminatorLatitude(lon, subsolar));
    lon += step;
  }
  return out;
}

/**
 * The night region as a flat `[lon0, lat0, …]` closed ring: the terminator
 * curve plus two vertices along whichever pole is in darkness.
 */
export function nightPolygonLatLon(subsolar: SubsolarPoint, stepDeg = 3.0): number[] {
  const curve = terminatorCurveLatLon(subsolar, stepDeg);
  // Sun north of the equator ⇒ the south pole is dark; close the ring there.
  const poleLat = subsolar.lat >= 0 ? -90 : 90;
  return [...curve, 180, poleLat, -180, poleLat];
}
