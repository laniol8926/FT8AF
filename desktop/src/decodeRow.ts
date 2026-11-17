/**
 * Decode-row metadata formatting, ported from Android's `DecodeDt.kt`
 * (DT) and `Ft8Message.java`'s `getFreq_hz()` (frequency). Extracted as
 * pure, testable functions per this repo's own rule (`App.tsx`'s decode row
 * stays a thin wrapper) after the user flagged that the desktop Decode
 * list's per-row numbers didn't quite match the real app's formatting.
 */

/**
 * A decode's DT, WSJT-X style: signed, one decimal, no unit -- "-1.3",
 * "+0.2", "0.0". A value that rounds to zero renders unsigned ("-0.0" is
 * noise, and an operator scanning a column of numbers for a sign shouldn't
 * have to discount it) -- this is the one rule a naive `toFixed(1)` +
 * `sign >= 0 ? "+" : ""` misses, confirmed as a real mismatch against
 * Android's own `formatDecodeDt`.
 */
export function formatDecodeDt(seconds: number): string {
  if (!Number.isFinite(seconds)) return "--";
  const rounded = Math.round(seconds * 10) / 10;
  if (Math.abs(rounded) < 0.05) return "0.0";
  const sign = rounded > 0 ? "+" : "-";
  return `${sign}${Math.abs(rounded).toFixed(1)}`;
}

/**
 * True when a decode's DT is far enough out to be worth the operator's eye
 * (colors the label) -- matches Android's `isDecodeDtNotable`, which shares
 * the same 1.0s threshold as the slot bar's clock-sync "fair" pill
 * (`CLOCK_SYNC_FAIR_SEC`) so a row can never call a reading alarming while
 * the averaged indicator still calls it fair.
 */
export function isDecodeDtNotable(seconds: number): boolean {
  return Number.isFinite(seconds) && Math.abs(seconds) > 1.0;
}

/**
 * A decode's frequency offset, Android's `getFreq_hz()` format
 * (`String.format("%04.0f", freq_hz)`): rounded to the nearest Hz,
 * zero-padded to 4 digits ("0523", not "523") so every row's numeric
 * columns line up in the monospace font regardless of digit count --
 * confirmed as a real mismatch, desktop was displaying the unpadded number.
 * `freqHz` is always a non-negative passband offset in real decodes;
 * clamped to 0 as a floor (not a port of Java's own, different, formatting
 * of a negative `%04.0f` -- that case can't occur here).
 */
export function formatDecodeFreq(freqHz: number): string {
  const rounded = Math.max(0, Math.round(freqHz));
  return String(rounded).padStart(4, "0");
}

/**
 * Distance text for a decode row, ported from Android's
 * `MaidenheadGrid.formatDist`/`convertDist`/`getDistUnitLabel` -- "%.0f %s"
 * rounded to the nearest whole unit. `distKm` is always in kilometers
 * (Android's own `getDist` return unit); `miles` selects the "Distance in
 * Miles" Settings toggle's unit.
 */
const KM_TO_MILES = 0.621371;

export function formatDistance(distKm: number, miles: boolean): string {
  const v = miles ? distKm * KM_TO_MILES : distKm;
  return `${Math.round(v)} ${miles ? "mi" : "km"}`;
}

/**
 * Initial great-circle bearing (degrees, `[0, 360)`) from point 1 to point 2
 * -- ported from Android's `BeamHeading.kt#greatCircleBearing` (standard
 * forward-azimuth formula). 360 is never returned (due north is 0).
 */
export function greatCircleBearingDeg(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const toRad = (d: number) => (d * Math.PI) / 180;
  const phi1 = toRad(lat1);
  const phi2 = toRad(lat2);
  const dLon = toRad(lon2 - lon1);
  const y = Math.sin(dLon) * Math.cos(phi2);
  const x = Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLon);
  return ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360;
}

/** Round a bearing to whole degrees in `0..359` (360 wraps back to 0) --
 *  matches Android's `normalizeHeadingDeg`. */
export function normalizeHeadingDeg(deg: number): number {
  return ((Math.round(deg) % 360) + 360) % 360;
}

/** "47°" -- matches Android's `formatHeading`. */
export function formatHeading(deg: number): string {
  return `${deg}°`;
}

/**
 * Short-path beam heading (whole degrees) from `myGrid` to `theirGrid`, or
 * `null` when either grid doesn't resolve to a position or the two points
 * are identical (a bearing to yourself is undefined) -- matches Android's
 * `computeBeamHeadings`'s short-path case exactly; desktop has no use for
 * the long-path reciprocal Android also computes (its QSO sheet shows
 * both, desktop's decode row -- the only place this is used -- shows only
 * the short path, same as Android's own row).
 */
export function beamHeadingDeg(
  myGrid: { lat: number; lon: number } | null,
  theirGrid: { lat: number; lon: number } | null,
): number | null {
  if (!myGrid || !theirGrid) return null;
  if (myGrid.lat === theirGrid.lat && myGrid.lon === theirGrid.lon) return null;
  return normalizeHeadingDeg(greatCircleBearingDeg(myGrid.lat, myGrid.lon, theirGrid.lat, theirGrid.lon));
}
