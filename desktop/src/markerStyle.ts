/**
 * Station-marker SNR tinting for the Map screen. Ported from Android's
 * `MarkerStyle.kt`'s `snrTint`. Scoped to literally this -- the rest of
 * `markerStyleFor`'s shape variety (isCQ/isToMe marker shapes) and
 * zoom-dependent label visibility are a separate, not-yet-requested
 * roadmap item.
 */

/** Extracts the r/g/b channels from a `#rrggbb` hex color string. */
function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const n = parseInt(hex.replace("#", ""), 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

/**
 * Maps a decode's SNR (dB) onto an alpha gradient over the base color,
 * roughly spanning the real FT8 decode range [-24, +10] dB. Floored at
 * 0.78 so even the weakest decode stays visibly opaque, not nearly
 * invisible -- matches Android's `snrTint` exactly.
 */
export function snrTint(baseHex: string, snr: number): string {
  const t = Math.min(1, Math.max(0, (snr + 24) / 34));
  const alpha = 0.78 + 0.22 * t;
  const { r, g, b } = hexToRgb(baseHex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Three-way worked split for a Map-screen CQ marker, matching
 *  `GridMarker.java`'s own icon-tint priority: worked this band takes
 *  priority over worked-on-another-band-only, and neither over a brand-new
 *  station. */
export interface WorkedBandStatus {
  workedThisBand: boolean;
  workedOtherBand: boolean;
}

export function workedBandStatus(
  callsign: string,
  workedCalls: Set<string>,
  workedCallsAnyBand: Set<string>,
): WorkedBandStatus {
  const workedThisBand = workedCalls.has(callsign);
  const workedOtherBand = !workedThisBand && workedCallsAnyBand.has(callsign);
  return { workedThisBand, workedOtherBand };
}

/** The marker fill color for a worked-band status -- matches
 *  `ic_baseline_cq_qso_24`/`tracker_cq_marker_is_qso_color` (worked this
 *  band), `tracker_cq_marker_other_is_qso_color` (worked another band), and
 *  plain `ic_baseline_cq_24` (brand new) respectively. */
export function markerColorForWorkedStatus(status: WorkedBandStatus): string {
  if (status.workedThisBand) return "#4ade80"; // --ok
  if (status.workedOtherBand) return "#facc15"; // --yellow
  return "#ffaf5e"; // --accent
}

/**
 * Whether a station counts as a "new zone" worth highlighting, matching
 * `GridMarkerInfoWindow`'s `msg.fromCq || msg.fromItu || msg.fromDxcc` (new
 * CQ-zone/ITU-zone/DXCC entity) -- desktop only tracks worked DXCC prefixes
 * and worked CQ zones (no ITU-zone set exists anywhere in this app), so
 * this is DXCC-or-CQ-zone newness, not the real three-way check; a close
 * approximation given what data is actually available, not a 1:1 port.
 * Already-worked (either band) stations are never "new", regardless of
 * zone, matching the Kotlin source's own `!checkQSLCallsign` guard.
 */
export function isNewZoneStation(
  status: WorkedBandStatus,
  dxccPrefix: string | null,
  cqZone: number | null,
  workedDxccPrefixes: Set<string>,
  workedCqZones: Set<number>,
): boolean {
  if (status.workedThisBand || status.workedOtherBand) return false;
  if (dxccPrefix != null && !workedDxccPrefixes.has(dxccPrefix)) return true;
  if (cqZone != null && !workedCqZones.has(cqZone)) return true;
  return false;
}
