// Hunt mode: auto-answer the best eligible CQ each decode cycle. The ranking
// core here is a direct port of Android's HuntTargetSelector.kt
// (ft8af/app/src/main/kotlin/radio/ks3ckc/ft8af/hunt/HuntTargetSelector.kt on
// `dev`), which is itself pure/unit-tested Kotlin with no Android dependency
// -- ported rather than re-designed so the two apps rank candidates the same
// way.
//
// Two of Android's priorities/tiers are intentionally left out rather than
// faked, because they need data desktop doesn't have yet:
//   - NEW_DXCC_FIRST needs a DXCC entity-from-callsign database (a large,
//     correctness-critical reference table -- planned as its own piece of
//     work, not something to rush here).
//   - POTA_FIRST's "new park" tier needs a live POTA spot feed
//     (PotaSpotsRepository) -- this is text-only POTA-CQ detection instead
//     (see isPotaCq below), so activatorRank only has two levels, not three.

export type HuntPriority =
  | "latest"
  | "strongest"
  | "weakest"
  | "farthest"
  | "pota_first"
  | "new_grid_first";

export const HUNT_PRIORITIES: HuntPriority[] = [
  "latest",
  "strongest",
  "weakest",
  "farthest",
  "pota_first",
  "new_grid_first",
];

/** Parses a persisted config value back to a priority, falling back to
 *  "latest" for anything unknown/blank -- Hunt should never break on a bad
 *  stored string (fresh install, downgrade, hand-edited config). */
export function huntPriorityFromConfig(value: string | null | undefined): HuntPriority {
  const v = (value ?? "").trim().toLowerCase();
  return (HUNT_PRIORITIES as string[]).includes(v) ? (v as HuntPriority) : "latest";
}

/** One eligible CQ caller, reduced to the facts the ranking needs. Plain data
 *  so selectHuntCandidate is testable with no engine/IPC involved. */
export interface HuntCandidate {
  /** Position in the eligible list (most-recent-first): smaller = fresher. */
  index: number;
  snr: number | null;
  distanceKm: number | null;
  /** The CQ is a POTA activation (text-detected, see isPotaCq). */
  isActivator: boolean;
  /** The caller's grid square hasn't been worked before. */
  isNewGrid: boolean;
  /** How many other stations were heard answering this caller this cycle. */
  pileupCalls: number;
}

/** Whether a candidate clears the minimum-signal floor. A null floor disables
 *  the check; a candidate with no SNR passes (the decoder occasionally omits
 *  SNR, and dropping those stations would make Hunt look randomly deaf). */
export function passesSnrFloor(snr: number | null, minSnrDb: number | null): boolean {
  return minSnrDb == null || snr == null || snr >= minSnrDb;
}

/**
 * Pick the best CQ to answer from this cycle's eligible callers.
 *
 * The SNR floor is a hard filter (a too-weak CQ is never answered). Pileup
 * avoidance is a soft preference: candidates nobody else is answering win,
 * but when every caller has a pileup, one is still picked rather than sitting
 * silent. Ties always go to the freshest decode (smallest `index`).
 */
export function selectHuntCandidate(
  candidates: HuntCandidate[],
  priority: HuntPriority,
  avoidPileups: boolean,
  minSnrDb: number | null,
): HuntCandidate | null {
  const floored = candidates.filter((c) => passesSnrFloor(c.snr, minSnrDb));
  if (floored.length === 0) return null;

  let pool = floored;
  if (avoidPileups) {
    const clear = floored.filter((c) => c.pileupCalls === 0);
    if (clear.length > 0) pool = clear;
  }

  if (priority === "latest") return pool[0];

  // Ascending 3-part sort key; the smallest key wins. Mirrors Kotlin's
  // priority-specific comparator(s) plus the always-final `.thenBy { index }`
  // tie-break.
  const noSnr = Number.NEGATIVE_INFINITY;
  const key = (c: HuntCandidate): [number, number, number] => {
    switch (priority) {
      case "strongest":
        return [-(c.snr ?? noSnr), 0, c.index];
      case "weakest":
        return [c.snr ?? Number.POSITIVE_INFINITY, 0, c.index];
      case "farthest":
        return [-(c.distanceKm ?? -1), 0, c.index];
      case "pota_first":
        return [c.isActivator ? -1 : 0, -(c.snr ?? noSnr), c.index];
      case "new_grid_first":
        return [c.isNewGrid ? -1 : 0, -(c.snr ?? noSnr), c.index];
    }
  };

  let best = pool[0];
  let bestKey = key(best);
  for (const c of pool.slice(1)) {
    const k = key(c);
    const better =
      k[0] < bestKey[0] ||
      (k[0] === bestKey[0] && (k[1] < bestKey[1] || (k[1] === bestKey[1] && k[2] < bestKey[2])));
    if (better) {
      best = c;
      bestKey = k;
    }
  }
  return best;
}

/** Count the other stations heard answering `cqCallsign` in the same decode
 *  window: any non-CQ message directed at the caller counts as a competitor. */
export function countRepliesTo(
  cqCallsign: string | null,
  messages: { is_cq: boolean; call_to: string }[],
): number {
  if (!cqCallsign) return 0;
  const target = cqCallsign.toUpperCase();
  return messages.filter((m) => !m.is_cq && m.call_to.toUpperCase() === target).length;
}

/**
 * Ported from Android's PotaCqClassifier.isPotaCq, reduced to the signals
 * available from decoded text alone: an explicit "POTA" token on a CQ, or a
 * garbled callsignTo-style fragment starting "CQ POT" (decoders mangle long
 * activator calls). Android's third signal -- a live pota.app spot lookup --
 * needs a new network integration and is out of scope here (see the POTA
 * screen phase of the UI-parity plan).
 */
export function isPotaCq(isCq: boolean, text: string): boolean {
  if (!isCq) return false;
  const upper = text.toUpperCase();
  return /\bPOTA\b/.test(upper) || upper.startsWith("CQ POT");
}

// --- Maidenhead grid-square math, for the "farthest" priority --------------

/** Center lat/lon of a 4- or 6-character Maidenhead grid square. Standard
 *  locator math: field = 20°lon x 10°lat cells, square = 2°lon x 1°lat,
 *  optional subsquare = 5'lon x 2.5'lat. Returns null for an unparseable
 *  grid rather than guessing. */
export function gridToLatLon(grid: string): { lat: number; lon: number } | null {
  const g = grid.trim().toUpperCase();
  if (g.length < 4) return null;
  const A = "A".charCodeAt(0);
  const lonField = g.charCodeAt(0) - A;
  const latField = g.charCodeAt(1) - A;
  if (lonField < 0 || lonField > 17 || latField < 0 || latField > 17) return null;
  const lonSq = g.charCodeAt(2) - "0".charCodeAt(0);
  const latSq = g.charCodeAt(3) - "0".charCodeAt(0);
  if (lonSq < 0 || lonSq > 9 || latSq < 0 || latSq > 9) return null;

  let lon = lonField * 20 - 180 + lonSq * 2;
  let lat = latField * 10 - 90 + latSq * 1;
  let lonSize = 2;
  let latSize = 1;

  if (g.length >= 6) {
    const lonSub = g.charCodeAt(4) - A;
    const latSub = g.charCodeAt(5) - A;
    if (lonSub >= 0 && lonSub <= 23 && latSub >= 0 && latSub <= 23) {
      lon += lonSub * (2 / 24);
      lat += latSub * (1 / 24);
      lonSize = 2 / 24;
      latSize = 1 / 24;
    }
  }
  return { lon: lon + lonSize / 2, lat: lat + latSize / 2 };
}

const EARTH_RADIUS_KM = 6371;

function toRad(deg: number): number {
  return (deg * Math.PI) / 180;
}

/** Great-circle distance between two lat/lon points (haversine). */
export function haversineKm(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const dLat = toRad(lat2 - lat1);
  const dLon = toRad(lon2 - lon1);
  const a =
    Math.sin(dLat / 2) ** 2 + Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLon / 2) ** 2;
  return EARTH_RADIUS_KM * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

/** Great-circle distance in km between two grid squares' centers, or null if
 *  either grid doesn't parse. */
export function maidenheadDistanceKm(gridA: string, gridB: string): number | null {
  const a = gridToLatLon(gridA);
  const b = gridToLatLon(gridB);
  if (!a || !b) return null;
  return haversineKm(a.lat, a.lon, b.lat, b.lon);
}
