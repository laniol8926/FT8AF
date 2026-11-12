/**
 * Per-pass CQ-marker + QSO-line computation for the Map screen, ported from
 * the fork's actual Android map -- `ui/map/GridTrackerPanel.kt` wrapping
 * `com.k1af.ft8af.grid_tracker.GridOsmMapView` (osmdroid), **not** FT8AF
 * upstream's own `ui/map/MapScreen.kt`. Confirmed directly:
 * `GridTrackerPanel.kt`'s own doc comment calls this "the grid tracker (the
 * default, fork) rather than the original FT8AF map."
 *
 * Scope per pass: the current decode slot only, not a running history --
 * `GridTrackerPanel.kt` groups the live message list by slot
 * (`utcTime / slotMillis`), keeps only the newest slot, and redraws from
 * scratch (`clearLines(); clearMarkers()`) every pass. This is a deliberate
 * scope cut the fork already made ("no grid shading: with a big log the
 * worked/confirmed/known grids cover the map. Only the current slot's lines
 * and CQs show") -- ported as-is, not reinterpreted.
 *
 * Per message, two different things can be drawn (`Ft8Message.checkIsCQ`/
 * `drawLine`/`addGridMarker` in `GridOsmMapView.java`):
 *  - a CQ message gets a marker at the caller's position -- no line, since
 *    there's no second party yet.
 *  - any directed exchange (grid-reply, report, R+report, RRR, 73, R73 --
 *    i.e. not a CQ) gets a line between the two REAL stations, call_from's
 *    position to call_to's position -- not "me" to them. This is a live
 *    "who's working whom right now" view.
 *
 * Grid resolution (`Ft8Message.getMaidenheadGrid`/`getToMaidenheadGrid`,
 * read directly in the real source):
 *  - call_from's position: the message's own grid when it has one (TX1 CQ,
 *    TX2 grid-reply -- exactly desktop's existing non-empty `UiMessage.grid`
 *    case), otherwise a cache lookup.
 *  - call_to's position: **always** a cache lookup -- the recipient's grid
 *    is never transmitted in any FT8 message type, so this isn't just a
 *    TX3-5 fallback the way call_from's is.
 */

/** The minimal per-message shape this module needs -- decoupled from
 *  `ipc.ts`'s `UiMessage` the same way `hunt.ts` stays decoupled; the
 *  caller maps `UiMessage[]` to this. */
export interface SlotMessage {
  callFrom: string;
  callTo: string;
  isCq: boolean;
  /** The decoder's own grid for this message, or "" -- only ever non-empty
   *  for a CQ or grid-reply decode (see module doc comment). */
  grid: string;
  utcMs: number;
}

export interface CqMarker<M extends SlotMessage = SlotMessage> {
  callsign: string;
  grid: string;
  message: M;
}

export interface QsoLine<M extends SlotMessage = SlotMessage> {
  fromCall: string;
  fromGrid: string;
  toCall: string;
  toGrid: string;
  /** Either end is the operator's own callsign. */
  involvesMyCall: boolean;
  message: M;
}

export interface GridTrackerSlot<M extends SlotMessage = SlotMessage> {
  markers: CqMarker<M>[];
  lines: QsoLine<M>[];
}

/** Whether a station marker should be visible, given the Map screen's
 *  Decodes/Worked layer chips. Ported from `MapRedesign.kt`'s
 *  `isStationVisible` -- the two chips partition every marker by worked
 *  status rather than being independently additive: a worked station's
 *  visibility is controlled only by `workedOn`, an unworked one only by
 *  `decodesOn`. */
export function isStationVisible(isWorked: boolean, decodesOn: boolean, workedOn: boolean): boolean {
  return isWorked ? workedOn : decodesOn;
}

/** The messages belonging to the single newest decode slot in `messages`
 *  (slot = floor(utcMs / cycleMs)). Order is preserved from the input. */
export function currentSlotMessages<M extends SlotMessage>(messages: M[], cycleMs: number): M[] {
  if (messages.length === 0 || cycleMs <= 0) return [];
  let newestSlot = -Infinity;
  for (const m of messages) {
    const slot = Math.floor(m.utcMs / cycleMs);
    if (slot > newestSlot) newestSlot = slot;
  }
  return messages.filter((m) => Math.floor(m.utcMs / cycleMs) === newestSlot);
}

/**
 * Build the current slot's CQ markers + QSO lines. `messages` should
 * already be narrowed to one slot (`currentSlotMessages`). Deduped by
 * `(callFrom, callTo)` pair, first occurrence in `messages`' order wins --
 * matches the Kotlin `HashSet`-guarded `drawn.add(key)` check. `resolveGrid`
 * backs the per-callsign cache lookup (`qth_for_call` on the backend); a
 * callsign it can't resolve (returns `null`/empty) is silently dropped, same
 * as `MaidenheadGrid.gridToLatLng(null)` returning null in the real source.
 */
export function buildGridTrackerSlot<M extends SlotMessage>(
  messages: M[],
  myCall: string,
  resolveGrid: (call: string) => string | null | undefined,
): GridTrackerSlot<M> {
  const myCallUpper = myCall.trim().toUpperCase();
  const drawn = new Set<string>();
  const markers: CqMarker<M>[] = [];
  const lines: QsoLine<M>[] = [];

  for (const m of messages) {
    const callFrom = m.callFrom.trim().toUpperCase();
    if (!callFrom) continue;
    const callTo = m.callTo.trim().toUpperCase();
    const key = `${callFrom}|${callTo}`;
    if (drawn.has(key)) continue;
    drawn.add(key);

    if (m.isCq) {
      const grid = m.grid || resolveGrid(callFrom) || "";
      if (grid) markers.push({ callsign: callFrom, grid, message: m });
      continue;
    }

    if (!callTo) continue;
    const fromGrid = m.grid || resolveGrid(callFrom) || "";
    const toGrid = resolveGrid(callTo) || "";
    if (!fromGrid || !toGrid) continue;
    lines.push({
      fromCall: callFrom,
      fromGrid,
      toCall: callTo,
      toGrid,
      involvesMyCall: callFrom === myCallUpper || callTo === myCallUpper,
      message: m,
    });
  }

  return { markers, lines };
}
