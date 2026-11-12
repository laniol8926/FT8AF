/**
 * Connection-line logic for the Map screen's Tracker-off mode. Ported from
 * Android's `ConnectionLines.kt`: a solid line to whoever's actively being
 * called (the live TX target), plus dashed lines from any station
 * currently calling the operator. Not general QSO-exchange lines between
 * arbitrary pairs -- that visual language belongs to Tracker-on mode
 * (`gridTrackerSlot.ts`'s per-slot `QsoLine`s) only.
 */

export type LineStyle = "SOLID_TX" | "DASHED_CALLING_ME";

export interface StationLine {
  callsign: string;
  grid: string;
  isToMe: boolean;
}

export interface ConnectionLine {
  toGrid: string;
  style: LineStyle;
  callsign: string;
}

/**
 * `txTargetCallsign` is normalized here: blank or case-insensitive "CQ"
 * both mean "no target" (a bare CQ with no reply locked in yet). If a
 * target is set but isn't among `stations` (not decoded this session, or
 * already scrolled out), no solid line is drawn -- mirrors Android's "no
 * line if the target isn't among decoded stations." The target's own
 * dashed "calling me" line (if it also happens to be calling the operator)
 * is suppressed so it doesn't get drawn twice.
 */
export function buildConnectionLines(stations: StationLine[], txTargetCallsign: string | null): ConnectionLine[] {
  const target = txTargetCallsign?.trim().toUpperCase() || null;
  const normalizedTarget = target === "CQ" ? null : target;

  const lines: ConnectionLine[] = [];
  let targetStation: StationLine | undefined;
  if (normalizedTarget) {
    targetStation = stations.find((s) => s.callsign.toUpperCase() === normalizedTarget);
    if (targetStation) {
      lines.push({ toGrid: targetStation.grid, style: "SOLID_TX", callsign: targetStation.callsign });
    }
  }

  for (const s of stations) {
    if (!s.isToMe) continue;
    if (targetStation && s.callsign === targetStation.callsign) continue;
    lines.push({ toGrid: s.grid, style: "DASHED_CALLING_ME", callsign: s.callsign });
  }

  return lines;
}
