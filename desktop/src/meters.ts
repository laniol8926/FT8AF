// SWR/power display formatting, ported from the Android app's
// MeterProtectionController.java (formatSwrRatio/formatPowerWatts) and
// ui/components/SwrChipTest.kt (the band thresholds) -- same values, so the
// two apps read the same meter the same way.

/** "1.3" style, one decimal; "9.9+" above that (never shows "infinite"). */
export function formatSwrRatio(ratio: number): string {
  if (ratio >= 9.95) return "9.9+";
  return ratio.toFixed(1);
}

/** "4.5W", "12.4W", "5W" -- one decimal, trimmed to a whole number when the
 *  value lands on one exactly (not rounded to always show ".0"). */
export function formatPowerWatts(watts: number): string {
  const rounded = Math.round(watts * 10) / 10;
  const body = Number.isInteger(rounded) ? rounded.toFixed(0) : rounded.toFixed(1);
  return `${body}W`;
}

export type SwrBand = "good" | "caution" | "bad";

/** Color band for the SWR chip: good up to 2.0:1, caution to 3.0:1, bad above. */
export function swrBand(ratio: number): SwrBand {
  if (ratio <= 2.0) return "good";
  if (ratio <= 3.0) return "caution";
  return "bad";
}
