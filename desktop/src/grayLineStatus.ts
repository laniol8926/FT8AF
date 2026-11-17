/**
 * Per-station solar / gray-line status for the QSO Sheet's "Gray line"
 * row, ported from Android's `ui/map/SolarStatus.kt`. HF propagation is
 * briefly enhanced along the gray line (the moving sunrise/sunset band),
 * so a DX operator who knows a distant station is at (or near) their
 * sunrise/sunset can time a call for the best shot.
 *
 * Built on the same `subsolarPoint`/`solarElevationDeg` math the map's own
 * terminator overlay uses (terminator.ts), so this is a thin, pure,
 * tested wrapper rather than a second solar-position implementation.
 */

import { solarElevationDeg, subsolarPoint } from "./terminator";

export type SolarEventKind = "sunrise" | "sunset";

export interface SolarSnapshot {
  elevationDeg: number;
  isDay: boolean;
  onGrayLine: boolean;
  nextKind: SolarEventKind | null;
  minutesToNext: number;
}

/** Half-width (degrees) of the gray-line band on either side of the horizon. */
export const GRAY_LINE_HALF_WIDTH_DEG = 6.0;

const EVENT_SCAN_STEP_MS = 2 * 60 * 1000;
/** A little over a day, so at least one crossing is found outside the polar regions. */
const EVENT_SCAN_SPAN_MS = 26 * 60 * 60 * 1000;

/**
 * The solar situation at lat/lon for the UTC instant `utcMillis`. "Day" is
 * elevation >= 0; the gray line is |elevation| <= grayLineHalfWidthDeg.
 * The next sunrise/sunset is found by scanning forward in fixed steps and
 * linearly interpolating the exact horizon crossing between the two
 * bracketing samples -- accurate to well under a minute. Near the poles a
 * full day can pass with no crossing (midnight sun / polar night);
 * `nextKind` is null there.
 */
export function solarSnapshot(
  lat: number,
  lon: number,
  utcMillis: number,
  grayLineHalfWidthDeg = GRAY_LINE_HALF_WIDTH_DEG,
): SolarSnapshot {
  const elevNow = solarElevationDeg(lat, lon, subsolarPoint(utcMillis));

  let prevElev = elevNow;
  let t = utcMillis + EVENT_SCAN_STEP_MS;
  const end = utcMillis + EVENT_SCAN_SPAN_MS;
  let nextKind: SolarEventKind | null = null;
  let crossingMs = 0;
  while (t <= end) {
    const elev = solarElevationDeg(lat, lon, subsolarPoint(t));
    if (prevElev < 0 && elev >= 0) nextKind = "sunrise";
    else if (prevElev >= 0 && elev < 0) nextKind = "sunset";
    if (nextKind != null) {
      const span = elev - prevElev;
      const frac = span === 0 ? 0 : (0 - prevElev) / span;
      const prevT = t - EVENT_SCAN_STEP_MS;
      crossingMs = prevT + Math.round(frac * EVENT_SCAN_STEP_MS);
      break;
    }
    prevElev = elev;
    t += EVENT_SCAN_STEP_MS;
  }

  const minutesToNext = nextKind == null ? 0 : Math.floor(Math.max(crossingMs - utcMillis, 0) / 60000);

  return {
    elevationDeg: elevNow,
    isDay: elevNow >= 0,
    onGrayLine: Math.abs(elevNow) <= grayLineHalfWidthDeg,
    nextKind,
    minutesToNext,
  };
}

export type GrayLinePhase = "gray_line" | "daylight" | "night";
export type GrayLineDetail = "sunrise" | "sunset" | "midnight_sun" | "polar_night";

export interface GrayLineDisplay {
  phase: GrayLinePhase;
  detail: GrayLineDetail;
  /** Preformatted "1h 20m" / "45m" countdown; empty for the polar no-event details. */
  countdown: string;
}

/** Resolve a SolarSnapshot into the tokens the gray-line row should display. */
export function grayLineDisplay(s: SolarSnapshot): GrayLineDisplay {
  const phase: GrayLinePhase = s.onGrayLine ? "gray_line" : s.isDay ? "daylight" : "night";
  if (s.nextKind === "sunrise") return { phase, detail: "sunrise", countdown: formatSolarCountdown(s.minutesToNext) };
  if (s.nextKind === "sunset") return { phase, detail: "sunset", countdown: formatSolarCountdown(s.minutesToNext) };
  return { phase, detail: s.isDay ? "midnight_sun" : "polar_night", countdown: "" };
}

/**
 * Format a whole-minute countdown as a compact "1h 20m" / "45m" string, or
 * "" when the event is happening now (< 1 minute away) -- the empty return
 * flags the caller to use a grammatical "at sunrise/sunset" string instead
 * of "sunrise in now".
 */
export function formatSolarCountdown(minutes: number): string {
  const m = Math.max(minutes, 0);
  if (m < 1) return "";
  const hours = Math.floor(m / 60);
  const mins = m % 60;
  if (hours <= 0) return `${mins}m`;
  if (mins === 0) return `${hours}h`;
  return `${hours}h ${mins}m`;
}
