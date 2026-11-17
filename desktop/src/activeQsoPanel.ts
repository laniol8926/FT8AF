/**
 * Pure logic for the docked "Calling CQ" / active-QSO panel, ported from
 * Android's `ActiveQsoPanel.kt` (`buildQsoLog`/`mergeRxLog`/`decideTxLog`).
 * This is the panel the Decode screen was missing entirely -- confirmed
 * live: desktop kept showing the generic "No signals decoded" empty state
 * throughout a real Call CQ transmission, with no visual indication a CQ
 * was even active, let alone what was actually being sent.
 *
 * Desktop uses Android's own "continuous log" shape (its `inLeftColumn`
 * mode, built for Android's landscape split view): ONE running log for the
 * whole TX run -- our CQ overs, then whoever answers and the exchange --
 * never cleared mid-run, only reset when a fresh run starts. That's the
 * natural fit for desktop's single always-visible panel (no separate
 * portrait/landscape mode to choose between).
 *
 * Deliberately not ported (real backend features that don't exist on
 * desktop yet, not a UI oversight): the PAUSE button (Android's hold-TX-
 * to-listen mechanic), the LOG button (force-log-and-move-on), the caller
 * queue bar, and the manual TX1-5 stage selector row (desktop's own
 * `set_stage`/`EngineCommand::SetStage` already covers manual stage
 * selection elsewhere in the UI). Each needs backend work the earlier
 * Call CQ/Answer CQ audit already catalogued as separate, unbuilt pieces
 * (Hunt/caller-queue, the pause mechanic, forceLogAndMoveOn) -- building a
 * button here that calls nothing real would be the opposite of disclosure.
 */

import type { UiMessage } from "./ipc";

export type QsoLogDirection = "tx" | "rx" | "busy";

export interface QsoLogEntry {
  direction: QsoLogDirection;
  utcMs: number;
  text: string;
  /** Only ever set for rx/busy -- desktop's decoder always has a real SNR
   *  (no "unknown" sentinel to port), so this is non-null whenever direction
   *  isn't "tx". */
  snr: number | null;
}

/** Stable identity for de-duplicating across snapshots -- matches
 *  Android's `qsoLogKey`. Time is part of the key on purpose: a station
 *  repeating the same text in a later cycle is a fresh transmission/decode
 *  and gets its own row. */
export function qsoLogKey(entry: QsoLogEntry): string {
  return `${entry.direction}|${entry.utcMs}|${entry.text}`;
}

/**
 * Classify this snapshot's decoded messages from `target` into RX (to us)
 * or BUSY (to someone else) rows. A CQ/DE/QRZ call from the target means
 * they're available, not busy (Android's #254) -- excluded entirely, not
 * classified either way. TX rows are never derived from the decoded list
 * (own-TX loopback, if any ever reached it, must never become a row here)
 * -- they come solely from the synthesized log the caller maintains via
 * `decideTxLog`. Matches Android's `buildQsoLog` exactly, simplified by
 * desktop already carrying `to_me`/`is_cq` per-message (no need to
 * re-derive `checkIsMyCallsign`/`checkIsCQ` here).
 *
 * `sinceMs` excludes anything older than the current run -- real bug,
 * found live: `messages` is the app's whole rolling decode history, so
 * without this floor, answering a station immediately flooded the panel
 * with that station's *older* activity (a QSO he'd worked minutes earlier,
 * before this run even started), not just what happens from here on.
 * Callers pass the moment the run started (or the seed CQ's own
 * timestamp -- see `seedCqEntry`), not a fixed window.
 */
export function buildQsoLog(messages: UiMessage[], target: string, sinceMs: number): QsoLogEntry[] {
  if (!target) return [];
  const upperTarget = target.toUpperCase();
  const entries: QsoLogEntry[] = [];
  for (const m of messages) {
    if (m.call_from.toUpperCase() !== upperTarget) continue;
    if (m.is_cq) continue;
    if (m.utc_ms < sinceMs) continue;
    entries.push({
      direction: m.to_me ? "rx" : "busy",
      utcMs: m.utc_ms,
      text: m.text,
      snr: m.snr,
    });
  }
  return entries;
}

/**
 * The one deliberate exception to `buildQsoLog`'s "no CQs" rule: the exact
 * CQ the operator clicked to answer him in the first place. Without it, the
 * single most relevant line -- *why* the panel is now "Waiting for
 * <target>" -- never appeared, even though it's what the panel header text
 * promises the operator is responding to. Only ever shown once, as the
 * log's opening entry -- a later bare CQ from him while we're still waiting
 * stays excluded, per the existing rule (he's not "busy," and we already
 * know he's there). `null` if `seedCq` is absent or is from someone other
 * than `target` (stale from a previous run the caller forgot to clear).
 */
export function seedCqEntry(seedCq: UiMessage | null, target: string): QsoLogEntry | null {
  if (!seedCq || seedCq.call_from.toUpperCase() !== target.toUpperCase()) return null;
  return { direction: "busy", utcMs: seedCq.utc_ms, text: seedCq.text, snr: seedCq.snr };
}

/** Upper bound on accumulated RX/BUSY rows for one target -- matches
 *  Android's `MAX_RX_LOG_ENTRIES`. */
export const MAX_RX_LOG_ENTRIES = 100;

/**
 * Merge freshly-classified rows into what the panel has already shown.
 * Matches Android's `mergeRxLog`: the decode list this is derived from is
 * shared, capped, and periodically emptied (a trim, a band change, the
 * Clear button) -- re-deriving from it alone would retroactively erase
 * conversation the operator already read. A repeat of a known key REPLACES
 * the stored row (desktop's decode list can upgrade a row's SNR in a later
 * pass the same way Android's does), latest-wins. Returns the caller's own
 * `existing` instance when nothing changed, so callers can skip a
 * re-render on a no-op merge (the common case, every cycle).
 */
export function mergeRxLog(existing: QsoLogEntry[], incoming: QsoLogEntry[]): QsoLogEntry[] {
  if (incoming.length === 0) return existing;
  const byKey = new Map<string, QsoLogEntry>();
  for (const e of existing) byKey.set(qsoLogKey(e), e);
  let changed = false;
  for (const entry of incoming) {
    const key = qsoLogKey(entry);
    const prev = byKey.get(key);
    if (!prev || prev.snr !== entry.snr || prev.text !== entry.text) {
      byKey.set(key, entry);
      changed = true;
    }
  }
  if (!changed) return existing;
  const merged = [...byKey.values()].sort((a, b) => a.utcMs - b.utcMs);
  if (merged.length <= MAX_RX_LOG_ENTRIES) return merged;
  return merged.slice(merged.length - MAX_RX_LOG_ENTRIES);
}

export interface TxLogDecision {
  shouldLog: boolean;
  pendingAfter: boolean;
}

/**
 * Decide whether to append one TX row, given the previous and current
 * transmitting state. Matches Android's `decideTxLog` exactly: logs one
 * row **per transmission** (keyed off the rising edge of `isTransmitting`),
 * not de-duplicated by text -- several unanswered repeats of the same
 * message each get their own row. A "pending" flag bridges the case where
 * `isTransmitting` flips true a tick before the message text is populated;
 * it clears once logged or once transmit ends, so it can never leak into
 * the next transmission.
 */
export function decideTxLog(
  wasTransmitting: boolean,
  isTransmitting: boolean,
  pending: boolean,
  message: string,
  active: boolean,
): TxLogDecision {
  const armed = pending || (isTransmitting && !wasTransmitting);
  const shouldLog = armed && isTransmitting && message.length > 0 && active;
  const pendingAfter = !isTransmitting ? false : shouldLog ? false : armed;
  return { shouldLog, pendingAfter };
}

/** `"CQ AI5II EM71"` -> `"(1713Hz) CQ AI5II EM71"`, matching Android's own
 *  `transmittingMessage` format (`FT8TransmitSignal`'s `" (%.0fHz) %s"`,
 *  trimmed of its leading space here since the row's own layout supplies
 *  that spacing). */
export function formatTxLogText(message: string, txFreqHz: number): string {
  return `(${Math.round(txFreqHz)}Hz) ${message}`;
}

/** "Calling CQ" / "QSOing with K1ABC" / "Waiting for K1ABC" -- matches
 *  Android's exact copy (`qsopanel_calling_cq`/`qsopanel_qsoing_with`/
 *  `qsopanel_waiting_for`). Android's "Listening for CQ" (Hunt, silent)
 *  and "Searching..." (no target, not calling CQ -- a brief transitional
 *  state) aren't reachable on desktop: Hunt's auto-answer logic doesn't
 *  exist yet (a separate, already-catalogued gap), and desktop's `qso.rs`
 *  never leaves `active` true with no target outside of actually calling
 *  CQ. */
export function qsoHeaderText(hasTarget: boolean, target: string | null, isTransmitting: boolean): string {
  if (!hasTarget) return "Calling CQ";
  return isTransmitting ? `QSOing with ${target}` : `Waiting for ${target}`;
}
