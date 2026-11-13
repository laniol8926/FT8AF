// Pure decision/format logic for Settings -> Time Sync's manual correction
// control, ported from the Android fork's own `TimeCorrection.kt` almost
// verbatim. The correction is a millisecond offset applied directly as the
// engine's `clock_offset_ms` (via `api.setClockOffset`) -- positive shifts
// the app's clock forward.
//
// The range covers the *device clock error*, not FT8's much tighter decode
// tolerance -- Android's own comment: a phone offline for a while can drift
// several seconds, and this is what pulls that back toward 0. Kept at
// Android's own ±5000 ms (its field report: a Samsung A50 needing over 3 s
// while offline is why it's ±5 s, not ±2 s).

export const TIME_CORRECTION_MIN_MS = -5000;
export const TIME_CORRECTION_MAX_MS = 5000;

/** Coerce an arbitrary correction into the allowed range. */
export function clampCorrectionMs(ms: number): number {
  return Math.min(TIME_CORRECTION_MAX_MS, Math.max(TIME_CORRECTION_MIN_MS, ms));
}

/** Apply a stepper nudge (e.g. +100 or -500 ms) and clamp to range. */
export function stepCorrectionMs(current: number, deltaMs: number): number {
  return clampCorrectionMs(current + deltaMs);
}

/** Human-readable offset, e.g. "+0.6s", "-0.3s", "0.0s". */
export function formatOffsetMs(ms: number): string {
  const sign = ms > 0 ? "+" : ms < 0 ? "-" : "";
  return `${sign}${(Math.abs(ms) / 1000).toFixed(1)}s`;
}
