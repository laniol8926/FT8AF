/**
 * Pure logic for the QSO Sheet -- the "tap a decode row" station-detail
 * dialog with an explicit Call button, ported from Android's `QsoSheet.kt`.
 * Distinct from the docked "Calling CQ" panel (activeQsoPanel.ts): this is
 * the review-before-calling step desktop had no equivalent of at all
 * (tapping a row called `onAnswer` immediately, with no confirmation).
 */

import { formatDistance, formatHeading, normalizeHeadingDeg, greatCircleBearingDeg } from "./decodeRow";
import { gridToLatLon, maidenheadDistanceKm } from "./hunt";
import { grayLineDisplay, solarSnapshot, type GrayLineDisplay } from "./grayLineStatus";
import type { TxStage } from "./ipc";

/**
 * Distance from the operator's grid to the remote station's grid, in the
 * unit Settings -> Decode & Filters' "Distance in Miles" selects. "--"
 * when either grid is unknown or unparseable; a genuine zero-distance
 * (identical grids) still formats as "0 mi"/"0 km", not "--".
 */
export function computeDistanceText(myGrid: string | null, theirGrid: string | null, miles: boolean): string {
  if (!myGrid || !theirGrid) return "--";
  const km = maidenheadDistanceKm(myGrid, theirGrid);
  if (km == null) return "--";
  return formatDistance(km, miles);
}

export interface BeamHeadings {
  shortPathDeg: number;
  longPathDeg: number;
}

/** Short- and long-path beam headings from myGrid to theirGrid, or null
 *  when either grid doesn't resolve or the two points are identical (a
 *  bearing to yourself is undefined). The long path is simply the
 *  reciprocal of the short path from the same point -- point your antenna
 *  the other way to go around the great circle the other direction. */
export function computeBeamHeadings(myGrid: string | null, theirGrid: string | null): BeamHeadings | null {
  if (!myGrid || !theirGrid) return null;
  const me = gridToLatLon(myGrid);
  const them = gridToLatLon(theirGrid);
  if (!me || !them) return null;
  if (me.lat === them.lat && me.lon === them.lon) return null;
  const shortPathDeg = normalizeHeadingDeg(greatCircleBearingDeg(me.lat, me.lon, them.lat, them.lon));
  return { shortPathDeg, longPathDeg: normalizeHeadingDeg(shortPathDeg + 180) };
}

export { formatHeading };

// ---------------------------------------------------------------------------
// Last heard
// ---------------------------------------------------------------------------

export type LastHeard =
  | { kind: "unknown" }
  | { kind: "just_now" }
  | { kind: "seconds"; value: number }
  | { kind: "minutes"; value: number }
  | { kind: "hours"; value: number }
  | { kind: "days"; value: number };

/**
 * Bucket the elapsed time between `utcTimeMillis` (when heard) and `now`
 * into a LastHeard token -- matches Android's `computeLastHeard` exactly:
 * a missing/non-positive timestamp is "unknown"; a future timestamp
 * (clock skew) clamps to "just now", never negative.
 */
export function computeLastHeard(utcTimeMillis: number, nowMillis: number): LastHeard {
  if (utcTimeMillis <= 0) return { kind: "unknown" };
  const elapsedMs = nowMillis - utcTimeMillis;
  const seconds = Math.max(Math.floor(elapsedMs / 1000), 0);
  if (seconds < 5) return { kind: "just_now" };
  if (seconds < 60) return { kind: "seconds", value: seconds };
  if (seconds < 3600) return { kind: "minutes", value: Math.floor(seconds / 60) };
  if (seconds < 86_400) return { kind: "hours", value: Math.floor(seconds / 3600) };
  return { kind: "days", value: Math.floor(seconds / 86_400) };
}

/** "--" / "just now" / "12s ago" / "3m ago" / "1h ago" / "2d ago" --
 *  matches Android's exact copy (`qso_last_heard_*`). */
export function lastHeardText(h: LastHeard): string {
  switch (h.kind) {
    case "unknown": return "--";
    case "just_now": return "just now";
    case "seconds": return `${h.value}s ago`;
    case "minutes": return `${h.value}m ago`;
    case "hours": return `${h.value}h ago`;
    case "days": return `${h.value}d ago`;
  }
}

// ---------------------------------------------------------------------------
// Gray line display text
// ---------------------------------------------------------------------------

/** "Daylight · sunset in 1h 20m" -- matches Android's `grayLineStatusText`. */
export function grayLineStatusText(display: GrayLineDisplay): string {
  const phaseText =
    display.phase === "gray_line" ? "On the gray line now" : display.phase === "daylight" ? "Daylight" : "Night";
  const detailText =
    display.detail === "sunrise"
      ? display.countdown === "" ? "at sunrise" : `sunrise in ${display.countdown}`
      : display.detail === "sunset"
      ? display.countdown === "" ? "at sunset" : `sunset in ${display.countdown}`
      : display.detail === "midnight_sun"
      ? "midnight sun"
      : "polar night";
  return `${phaseText} · ${detailText}`;
}

/** Gray-line status text for a station's grid at the given instant, or
 *  null when the grid is unknown (nothing to place on the globe) --
 *  matches Android's `GrayLineRow`'s render-nothing guard. */
export function computeGrayLineStatusText(grid: string | null, nowMillis: number): string | null {
  const latLon = gridToLatLon(grid ?? "");
  if (!latLon) return null;
  return grayLineStatusText(grayLineDisplay(solarSnapshot(latLon.lat, latLon.lon, nowMillis)));
}

// ---------------------------------------------------------------------------
// Location line
// ---------------------------------------------------------------------------

/** Combine state/country/grid into a single location line, or null when
 *  none of the inputs are usable -- matches Android's `formatLocationLine`. */
export function formatLocationLine(state: string | null, country: string | null, grid: string | null): string | null {
  const parts: string[] = [];
  const place = state ? (country ? `${state}, ${country}` : state) : country;
  if (place) parts.push(place);
  if (grid) parts.push(grid);
  return parts.length === 0 ? null : parts.join(" • ");
}

// ---------------------------------------------------------------------------
// QSO sequence visualizer
// ---------------------------------------------------------------------------

export const QSO_STEP_LABELS = ["Send call", "Report sent", "Roger", "Confirm", "Logged"];

/**
 * Which step (0..4) the QSO is on, or -1 for "not started". Just-viewing
 * (no live QSO with this station) always returns -1 so the sequence stays
 * grayed out until the operator commits via the Call button; a fully
 * worked-and-confirmed call lights all 5 as complete (returns 5).
 * `liveStage` is desktop's `QsoStatus.stage` (TxStage), mapped the same
 * way Android maps its own functionOrder 1..5.
 */
export function computeCurrentStepIndex(
  isFullyComplete: boolean,
  isLiveQso: boolean,
  liveStage: TxStage | null,
): number {
  if (isFullyComplete && !isLiveQso) return 5;
  if (!isLiveQso) return -1;
  switch (liveStage) {
    case "grid": return 0;
    case "report": return 1;
    case "r_report": return 2;
    case "rr73": return 3;
    case "bye73": return 4;
    default: return -1;
  }
}

export interface QsoStep {
  label: string;
  txRxLabel: "TX" | "RX" | "--";
  messagePreview: string;
}

/** Build the 5-step preview messages for this exchange -- matches
 *  Android's `QsoSequenceVisualizer`'s `steps` list exactly. */
export function buildQsoSteps(callsign: string, myCall: string, myGrid: string, snrDisplay: string): QsoStep[] {
  return [
    { label: QSO_STEP_LABELS[0], txRxLabel: "TX", messagePreview: `${callsign} ${myCall} ${myGrid}` },
    { label: QSO_STEP_LABELS[1], txRxLabel: "RX", messagePreview: `${myCall} ${callsign} ${snrDisplay}` },
    { label: QSO_STEP_LABELS[2], txRxLabel: "TX", messagePreview: `${callsign} ${myCall} R${snrDisplay}` },
    { label: QSO_STEP_LABELS[3], txRxLabel: "RX", messagePreview: `${myCall} ${callsign} RR73` },
    { label: QSO_STEP_LABELS[4], txRxLabel: "--", messagePreview: `${callsign} ${myCall} 73` },
  ];
}

/** "QSOing with K1ABC — Sending report sent" / "Waiting for K1ABC to
 *  reply..." / "Transmitting..." / null when nothing live applies --
 *  matches Android's `QsoLiveStatusRow` text selection. */
export function qsoLiveStatusText(
  callsign: string,
  isLiveQso: boolean,
  isTransmitting: boolean,
  currentStepIndex: number,
): string | null {
  if (isLiveQso && isTransmitting) {
    const stepLabel = QSO_STEP_LABELS[currentStepIndex]?.toLowerCase() ?? "message";
    return `QSOing with ${callsign} — Sending ${stepLabel}`;
  }
  if (isLiveQso && !isTransmitting) return `Waiting for ${callsign} to reply...`;
  if (!isLiveQso && isTransmitting) return "Transmitting...";
  return null;
}
