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
