/**
 * "Who's hearing me" summary card logic for the Map screen's PSK overlay.
 * Ported from Android's `SignalReach.kt` -- reduces a set of PSK Reporter
 * reception reports into one headline: how many receivers heard us, the
 * furthest one, and the strongest-SNR one. Reuses `hunt.ts`'s existing
 * `haversineKm` for distance (per `mapProjection.ts`'s own doc comment:
 * this already exists, no second great-circle implementation).
 */

import { haversineKm } from "./hunt";

export interface ReachSpot {
  callsign: string;
  lat: number;
  lon: number;
  snr: number;
}

export interface SignalReach {
  receivers: number;
  furthestKm: number;
  furthestCall: string;
  strongestSnr: number;
  strongestCall: string;
}

/** `null` if `spots` is empty (nothing to summarize) or every entry has a
 *  blank callsign. De-dupes by callsign (uppercased), keeping whichever
 *  report has the higher SNR per station, matching Android's
 *  `LinkedHashMap`-based reducer. When `opLat`/`opLon` are unknown,
 *  `furthestKm` stays `0` and `furthestCall` stays `""` -- strongest-SNR
 *  tracking doesn't need the operator's own position, so it's always
 *  computed. */
export function summarizeSignalReach(
  spots: ReachSpot[],
  opLat: number | null,
  opLon: number | null,
): SignalReach | null {
  if (spots.length === 0) return null;

  const byCall = new Map<string, ReachSpot>();
  for (const s of spots) {
    const key = s.callsign.trim().toUpperCase();
    if (!key) continue;
    const existing = byCall.get(key);
    if (!existing || s.snr > existing.snr) byCall.set(key, s);
  }
  if (byCall.size === 0) return null;

  let furthestKm = 0;
  let furthestCall = "";
  let strongestSnr = -Infinity;
  let strongestCall = "";
  for (const s of byCall.values()) {
    if (opLat !== null && opLon !== null) {
      const km = haversineKm(opLat, opLon, s.lat, s.lon);
      if (km > furthestKm) {
        furthestKm = km;
        furthestCall = s.callsign;
      }
    }
    if (s.snr > strongestSnr) {
      strongestSnr = s.snr;
      strongestCall = s.callsign;
    }
  }

  return { receivers: byCall.size, furthestKm, furthestCall, strongestSnr, strongestCall };
}
