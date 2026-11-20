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

/** How close (Hz) another station's decode has to be to the target's own
 *  last-heard frequency to count as "on his frequency" for `buildQsoLog`
 *  -- roughly one FT8 signal's own bandwidth (8 tones * ~6.25 Hz tone
 *  spacing =~ 50 Hz), so a real collision/adjacent exchange on his channel
 *  is caught without pulling in an unrelated station a bit further up the
 *  passband. A judgment call, not a protocol constant -- adjust if it
 *  turns out too wide or too narrow in practice. */
export const QSO_LOG_FREQ_TOLERANCE_HZ = 50;

/**
 * Classify decoded messages since `sinceMs` into RX (to us) or BUSY (to
 * anyone else) rows: the target's own traffic (any frequency -- he can
 * drift a little between transmissions), plus anyone else decoded within
 * `QSO_LOG_FREQ_TOLERANCE_HZ` of his own last-heard frequency
 * (`targetFreqHz`). TX rows are never derived from the decoded list
 * (own-TX loopback, if any ever reached it, must never become a row here)
 * -- they come solely from the synthesized log the caller maintains via
 * `decideTxLog`.
 *
 * Deliberately NOT matching Android's `buildQsoLog` here, across three
 * rounds of live operator feedback. First: Android's #254 excludes a
 * repeated CQ/DE/QRZ from the target entirely ("he's available, not busy,
 * we already know he's there"), but desktop's operator wanted the
 * opposite -- "I should see all activity on the rx frequency... but see
 * nothing but my calling him in the message panel" while watching the
 * target repeat his CQ on the waterfall with nothing reflected in the
 * docked panel. Second: once repeated CQs from the target were shown, the
 * operator wanted every OTHER station on the frequency too, not just the
 * target's own traffic -- explicit, concrete reasoning: "(1) [the target]
 * could have been answered by someone else. Right now I don't know that
 * and just keep calling him or (2) some other station is transmitting on
 * top of him and I don't know that either." That round removed the
 * `call_from` filter entirely, showing every decoded station on the whole
 * band -- too broad in practice: "it is showing me all activity. Should
 * only show the activity on the frequency the station that is calling CQ
 * [is on]." This third round narrows it to frequency proximity instead of
 * callsign, which still covers both of the operator's own reasons (a
 * collision is a frequency match by definition; an answerer close to the
 * target's own channel is caught too) without flooding the panel with
 * the whole band's traffic.
 *
 * `sinceMs` excludes anything older than the current run -- real bug,
 * found live: `messages` is the app's whole rolling decode history, so
 * without this floor, answering a station immediately flooded the panel
 * with older activity (a QSO worked minutes earlier, before this run even
 * started), not just what happens from here on. Callers pass the moment
 * the run started (or the seed CQ's own timestamp -- see `seedCqEntry`),
 * not a fixed window. `target` is kept only to gate the whole function on
 * there being an active excursion at all (matching the caller's own
 * `if (!logTarget) return;` guard), same as before -- not for row
 * filtering, which is now frequency-based.
 */
export function buildQsoLog(
  messages: UiMessage[],
  target: string,
  sinceMs: number,
  targetFreqHz: number | null,
): QsoLogEntry[] {
  if (!target) return [];
  const upperTarget = target.toUpperCase();
  const entries: QsoLogEntry[] = [];
  for (const m of messages) {
    const isTarget = m.call_from.toUpperCase() === upperTarget;
    const onHisFrequency =
      targetFreqHz != null && Math.abs(m.freq_hz - targetFreqHz) <= QSO_LOG_FREQ_TOLERANCE_HZ;
    if (!isTarget && !onHisFrequency) continue;
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
 * The exact CQ the operator clicked to answer him in the first place --
 * needed separately from `buildQsoLog` because it necessarily predates
 * `sinceMs` (the run only started *because* this CQ was seen and clicked),
 * so `buildQsoLog`'s own floor would otherwise exclude it. Without it, the
 * single most relevant line -- *why* the panel is now "Waiting for
 * <target>" -- never appeared, even though it's what the panel header text
 * promises the operator is responding to. Every later repeat of his CQ is
 * picked up by `buildQsoLog` itself (see its own doc comment) -- this only
 * covers the one that came before the log's own starting point. `null` if
 * `seedCq` is absent or is from someone other than `target` (stale from a
 * previous run the caller forgot to clear).
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
