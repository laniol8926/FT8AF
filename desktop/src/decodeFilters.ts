// Pure logic for Settings -> Decode & Filters, ported from the Android
// fork's own `DecodeFilterSettings.kt`/`GeneralVariables.java`: the worked-
// station handling (mode/scope), the callsign blocklist, and the Needed-DX
// watchlist. Kept separate from `qsoStatus.ts` (which owns the highlight
// *priority* resolution) since these gate what's even *in* the decode list,
// not just how a shown row is colored.

export type WorkedMode = "highlight" | "ignore" | "hide";
export type WorkedScope = "on_band" | "before" | "today" | "from_list";

/** Lenient parse, defaulting to Android's own default ("highlight"). */
export function parseWorkedMode(s: string | null | undefined): WorkedMode {
  return s === "ignore" || s === "hide" ? s : "highlight";
}

/** Lenient parse, defaulting to Android's own default ("on_band" -- ordinal
 *  0 of its `WorkedStationScope` enum, "preserves legacy behavior" per its
 *  own comment). */
export function parseWorkedScope(s: string | null | undefined): WorkedScope {
  return s === "before" || s === "today" || s === "from_list" ? s : "on_band";
}

/** Which "worked" set applies for the configured scope. */
export function isWorkedByScope(
  scope: WorkedScope,
  f: { onBand: boolean; anyBand: boolean; today: boolean; inList: boolean },
): boolean {
  switch (scope) {
    case "on_band": return f.onBand;
    case "before": return f.anyBand;
    case "today": return f.today;
    case "from_list": return f.inList;
  }
}

/** Whether a message from a worked station should be dropped from the
 *  decode list entirely -- only when the operator picked "Hide". "Highlight"
 *  and "Ignore" both keep the row (resolveQsoStatus's own highlight gate
 *  already handles "ignore" by leaving the worked flag off the priority
 *  chain when `highlightWorked` is false, so this function only needs to
 *  know about the Hide case). */
export function shouldHideWorked(mode: WorkedMode, isWorked: boolean): boolean {
  return mode === "hide" && isWorked;
}

/** Comma/space-separated call/prefix/keyword list -> a clean uppercased array. */
export function parseCallList(s: string): string[] {
  return s
    .split(/[,\s]+/)
    .map((t) => t.trim().toUpperCase())
    .filter((t) => t.length > 0);
}

export interface Blocklist {
  exact: string[];
  prefixes: string[];
  keywords: string[];
}

/** Whether a decode should be dropped by the callsign blocklist -- an exact
 *  whole-call match, a callsign-prefix match, or a substring match against
 *  either the callsign or the full message text (matches Android's own
 *  three independent lists exactly, see `DecodeFilterSettings.kt`). */
export function isBlocked(call: string, text: string, list: Blocklist): boolean {
  const callUpper = call.toUpperCase();
  const textUpper = text.toUpperCase();
  if (list.exact.includes(callUpper)) return true;
  if (list.prefixes.some((p) => callUpper.startsWith(p))) return true;
  if (list.keywords.some((k) => callUpper.includes(k) || textUpper.includes(k))) return true;
  return false;
}

/** Whether a callsign matches the Needed-DX watchlist -- prefix match, so a
 *  DXpedition prefix (e.g. "3Y0") catches every suffix variant, same as
 *  Android's own `settings_watch_dialog_desc`. */
export function matchesWatchlist(call: string, watchPrefixes: string[]): boolean {
  const callUpper = call.toUpperCase();
  return watchPrefixes.some((p) => callUpper.startsWith(p));
}
