/**
 * Pure display helpers for the caller-queue bar inside the active-QSO
 * panel, ported from Android's `ActiveQsoPanel.kt`'s own
 * `orderCallerQueue`/`callerSnrLabel`. The queue itself (enqueue/refresh/
 * prune/dequeue) lives entirely in the Rust backend (`qso.rs`'s
 * `QueuedCaller`/`QsoEngine::offer_callers`) and arrives here as a plain
 * snapshot (`QsoStatus.caller_queue`) -- this module only orders it for
 * display, mirroring the auto-dequeue policy so the chip the operator is
 * nudged to tap next is the same station the engine would pick if/when
 * auto-dequeue is built (see this feature's plan doc's "Not in this pass").
 */

import type { QueuedCaller } from "./ipc";

/** Display order for the queue's chips. `strongestFirst` sorts by SNR
 *  descending (best copy first); otherwise first-heard (queue) order is
 *  kept as-is. Stable under equal SNR (ties keep first-heard order) since
 *  `Array.prototype.toSorted`/`sort` in modern JS engines is a stable sort. */
export function orderCallerQueue(queue: QueuedCaller[], strongestFirst: boolean): QueuedCaller[] {
  if (!strongestFirst) return queue;
  return [...queue].sort((a, b) => b.snr - a.snr);
}

/** Signed SNR label for a queued-caller chip (e.g. "+5", "-12"). */
export function callerSnrLabel(snr: number): string {
  return snr >= 0 ? `+${snr}` : `${snr}`;
}
