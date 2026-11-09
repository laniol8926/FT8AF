// Decode-list sort control, ported from Android's DecodeCollapse.kt
// (DecodeSortMode, nextSortMode, collapseByStation) and the Decode screen's
// top-bar SortModeLabel (TIME/CALL/SNR/DX).
//
// Only applied for the non-default modes (call/snr/dx) -- the default "time"
// mode keeps this app's existing full-history, per-slot-grouped decode list
// exactly as before (already built and approved this round), rather than
// switching to Android's always-collapsed-to-one-row-per-station view, which
// would be a much bigger, unrequested behavior change to the default list.

import type { UiMessage } from "./ipc";
import { maidenheadDistanceKm } from "./hunt";

export type DecodeSortMode = "time" | "call" | "snr" | "dx";

/** The mode selected when the operator taps the sort control while on `current`. */
export function nextSortMode(current: DecodeSortMode): DecodeSortMode {
  switch (current) {
    case "time": return "call";
    case "call": return "snr";
    case "snr": return "dx";
    case "dx": return "time";
  }
}

// Matches DecodeScreen.kt's SortModeLabel exactly (decode_sort_label_*).
export const SORT_MODE_LABEL: Record<DecodeSortMode, string> = {
  time: "TIME",
  call: "CALL",
  snr: "SNR",
  dx: "DX",
};

export const SORT_MODE_TITLE: Record<DecodeSortMode, string> = {
  time: "Sort: last heard",
  call: "Sort: callsign",
  snr: "Sort: SNR",
  dx: "Sort: distance, farthest first",
};

/** Collapse a message list to the latest decode per station (by call_from,
 *  trimmed/upper-cased), then order per `sortMode` -- mirrors
 *  `collapseByStation`. "Latest" is the greatest utc_ms; on a tie, the one
 *  appearing later in `messages` wins. Messages without a usable callsign are
 *  dropped, same as Android (the row UI keys entirely off the callsign). */
export function collapseByStation(
  messages: UiMessage[],
  sortMode: DecodeSortMode,
  myGrid: string,
): UiMessage[] {
  const latest = new Map<string, UiMessage>();
  for (const m of messages) {
    const cs = m.call_from.trim().toUpperCase();
    if (!cs) continue;
    const existing = latest.get(cs);
    if (!existing || m.utc_ms >= existing.utc_ms) {
      latest.set(cs, m);
    }
  }
  const collapsed = Array.from(latest.values());
  switch (sortMode) {
    case "time":
      return collapsed.sort((a, b) => b.utc_ms - a.utc_ms);
    case "call":
      return collapsed.sort((a, b) => a.call_from.toUpperCase().localeCompare(b.call_from.toUpperCase()));
    case "snr":
      return collapsed.sort((a, b) => b.snr - a.snr);
    case "dx": {
      // Farthest first; a station with no parseable grid sinks to the bottom
      // rather than pretending to be near (mirrors Android's
      // Double.NEGATIVE_INFINITY fallback).
      const dist = (m: UiMessage) => (myGrid ? maidenheadDistanceKm(myGrid, m.grid) : null) ?? -Infinity;
      return collapsed
        .map((m) => ({ m, d: dist(m) }))
        .sort((a, b) => b.d - a.d)
        .map((x) => x.m);
    }
  }
}
