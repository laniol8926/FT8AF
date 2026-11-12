/**
 * Cross-session "one marker per callsign" station accumulator for the Map
 * screen's Tracker-off mode. Ported from Android's `MapScreen.kt` own
 * `stations` builder -- deliberately distinct from `gridTrackerSlot.ts`'s
 * `buildGridTrackerSlot`, whose CQ-gets-a-marker/directed-gets-a-line split
 * doesn't match this: Android's `stations` always makes one marker per
 * unique callsign regardless of whether their latest message was a CQ or a
 * directed reply, using `isCq`/`isToMe` purely for marker styling.
 */

import type { SlotMessage } from "./gridTrackerSlot";

export interface AccumulatedStation<M extends SlotMessage = SlotMessage> {
  callsign: string;
  grid: string;
  isCq: boolean;
  isToMe: boolean;
  message: M;
}

/**
 * Walks `messages` (assumed newest-first, as `App.tsx`'s decode handler
 * always prepends fresh decodes -- see its `[...fresh, ...prev]`) and keeps
 * the first occurrence per `callFrom`, which is therefore already the
 * latest message from that station -- no reversal or timestamp comparison
 * needed. A callsign that never resolves a grid (own message carries none,
 * and `resolveGrid` can't find one either) is silently dropped, matching
 * `buildGridTrackerSlot`'s existing convention for the same situation.
 */
export function buildAccumulatedStations<M extends SlotMessage>(
  messages: M[],
  myCall: string,
  resolveGrid: (call: string) => string | null | undefined,
): AccumulatedStation<M>[] {
  const myCallUpper = myCall.trim().toUpperCase();
  const seen = new Set<string>();
  const out: AccumulatedStation<M>[] = [];

  for (const m of messages) {
    const callFrom = m.callFrom.trim().toUpperCase();
    if (!callFrom || seen.has(callFrom)) continue;
    seen.add(callFrom);

    const grid = m.grid || resolveGrid(callFrom) || "";
    if (!grid) continue;

    const callTo = m.callTo.trim().toUpperCase();
    out.push({
      callsign: callFrom,
      grid,
      isCq: m.isCq,
      isToMe: callTo === myCallUpper,
      message: m,
    });
  }

  return out;
}
