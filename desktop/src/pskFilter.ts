/**
 * PSK Reporter overlay filter state for the Map screen. Ported from
 * Android's `PskFilter.kt`, simplified where this codebase already has an
 * equivalent: band classification happens server-side now
 * (`psk_reporter.rs` resolves each spot's `band` via the existing
 * `bands::band_for_freq_hz`, the same table `db.rs`'s `worked_calls`/
 * `worked_states` already use), so there's no second `Band`
 * enum/range-table or `freqHzToBand` here -- filtering by band is just a
 * string compare against the band name the backend already resolved.
 * There's also no "excluded bands" concept on desktop (Android's
 * `GeneralVariables.isBandExcluded`), so the band filter simply offers
 * whatever bands are already known to the app (`api.listBands()`).
 */

import type { OperatingMode } from "./ipc";

export type PskMode = "CURRENT" | "FT8" | "FT4" | "FT2";
export type PskDirection = "WHO_HEARD_ME" | "WHO_I_HEARD";

/** Seconds-back presets for the PSK filter sheet's time-window chips. */
export const PSK_TIME_PRESETS = [900, 3600, 10800, 21600, 86400];

export function pskTimeLabel(sec: number): string {
  if (sec < 3600) return `${Math.round(sec / 60)}m`;
  return `${Math.round(sec / 3600)}h`;
}

export interface PskFilter {
  timeWindowSec: number;
  /** Band name (e.g. "20m"), or null for "All". */
  band: string | null;
  mode: PskMode;
  direction: PskDirection;
}

/** `CURRENT` resolves to whatever the rig is actually doing right now --
 *  the caller passes the live `OperatingMode` in, since there's no
 *  synchronous "ask the engine" read the way Android's
 *  `GeneralVariables.currentMode()` singleton allows (see
 *  `psk_reporter.rs`'s doc comment). */
export function resolvePskModeString(filter: PskMode, operatingMode: OperatingMode): string {
  if (filter !== "CURRENT") return filter;
  return operatingMode.toUpperCase();
}

/** Opens the filter sheet pre-filtered to the operator's current band,
 *  matching Android's own `initialPskFilter(dialHz)` -- `bandForDialHz`
 *  is injected so this stays decoupled from any particular band-lookup
 *  source (the frontend has no local band-range table of its own; the
 *  caller resolves this via whatever's already on hand, e.g. the current
 *  band list from `api.listBands()`). */
export function initialPskFilter(dialHz: number, bandForDialHz: (hz: number) => string | null): PskFilter {
  return { timeWindowSec: 3600, band: bandForDialHz(dialHz), mode: "CURRENT", direction: "WHO_HEARD_ME" };
}

export interface HasBand {
  band: string;
}

/** `null` band means "All" -- no filtering. */
export function applyPskBandFilter<T extends HasBand>(spots: T[], band: string | null): T[] {
  if (band === null) return spots;
  return spots.filter((s) => s.band === band);
}
