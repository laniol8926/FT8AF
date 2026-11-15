// Per-message status pill, ported from Android's DecodeRow.kt
// (resolveQsoStatus) and StatusPill.kt (QsoStatus enum: label + color). This
// is what the Decode list's compact row trailing pill shows -- replaces a
// plain "is this a CQ" dot with the same priority-ordered, multi-state
// classification Android uses (calling me, POTA, new DXCC/zone/state/grid/
// prefix/band, worked, plain CQ).
//
// SOTA is real: the CQ "modifier" token (e.g. "SOTA" in "CQ SOTA K1ABC
// FN42") decodes into plain text already -- desktop's C decode core is the
// exact same kgoba ft8_lib Android's own cpp/unpack.c compiles from
// (confirmed: `unpack_callsign`'s "CQ_nnn"/"CQ_aaaa" special-token branch
// produces this literal text, no new decode-side work needed), so
// `extractCqModifier` below is pure text parsing on top of what the
// decoder already hands back, matching Android's exact detection regex.
//
// One tier is still left out rather than faked:
//   - NEW_POTA (an unhunted park, vs. plain POTA): the FT8 message itself
//     never carries a park reference -- Android gets it from a *live
//     pota.app spot feed* correlating callsign -> park (`PotaSpotsRepository
//     .parkRefFor`), the same gap hunt.ts already documents for Hunt's
//     pota_first priority. Without that feed there's no way to know *which*
//     park a station is activating, let alone whether it's already hunted,
//     so isPota here (text-only CQ detection, isPotaCq) can only ever
//     produce the plain POTA tier -- a real missing subsystem, not a quick
//     text-parsing addition like SOTA was.
//
// highlightX gates below mirror Android's Settings -> Decode Highlights
// *real, operator-configurable* values now (Settings -> Decode & Filters,
// wired in App.tsx's highlightConfig) -- SOTA itself is never gated by a
// highlight toggle, matching Android's own unconditional `isCQ && modifier
// == "SOTA"` check exactly (unlike POTA, which Android does gate behind
// `highlightPota`).

/**
 * The CQ "modifier" token right after "CQ " (e.g. "SOTA", "DX", "123"), or
 * `null` when the next token doesn't look like one -- matches Android's own
 * `Ft8Message.modifier` detection regex exactly (`"[0-9]{3}|[A-Z]{1,4}"`).
 * A real callsign never matches this: every issued amateur callsign
 * contains at least one digit mixed with letters, which neither pattern
 * allows (3 digits with no letters, or 1-4 letters with no digits).
 */
export function extractCqModifier(isCq: boolean, text: string): string | null {
  if (!isCq) return null;
  const upper = text.trim().toUpperCase();
  if (!upper.startsWith("CQ ")) return null;
  const token = upper.slice(3).trim().split(/\s+/)[0] ?? "";
  if (/^[0-9]{3}$/.test(token) || /^[A-Z]{1,4}$/.test(token)) return token;
  return null;
}

export type QsoStatus =
  | "pending"
  | "pota"
  | "sota"
  | "new_dxcc"
  | "new_zone"
  | "new_state"
  | "new_grid"
  | "new_prefix"
  | "new_band"
  | "worked"
  | "cq";

export interface QsoStatusFlags {
  isCq: boolean;
  isToMe: boolean;
  isPota: boolean;
  /** A SOTA activator's CQ (modifier == "SOTA") -- see `extractCqModifier`. */
  isSota: boolean;
  isNewDxcc: boolean;
  isNewZone: boolean;
  isNewState: boolean;
  isNewGrid: boolean;
  isNewPrefix: boolean;
  /** Worked on some other band, but not the one this message was heard on. */
  isWorkedOtherBand: boolean;
  isWorked: boolean;
}

export interface HighlightConfig {
  dxcc: boolean;
  zone: boolean;
  state: boolean;
  grid: boolean;
  prefix: boolean;
  band: boolean;
  worked: boolean;
  pota: boolean;
}

export const HIGHLIGHT_DEFAULTS: HighlightConfig = {
  dxcc: true,
  zone: true,
  state: false,
  grid: false,
  prefix: false,
  band: true,
  worked: true,
  pota: true,
};

/** Highest-priority applicable status, or null for a plain (unremarkable)
 *  message. `highlights` defaults to Android's own defaults (matching every
 *  existing call site's prior unconfigurable behavior exactly); Settings ->
 *  Decode & Filters -> Decode Highlights passes the operator's real,
 *  persisted choice once loaded (see `App.tsx`'s `highlightConfig` state). */
export function resolveQsoStatus(f: QsoStatusFlags, highlights: HighlightConfig = HIGHLIGHT_DEFAULTS): QsoStatus | null {
  if (f.isToMe) return "pending";
  if (highlights.pota && f.isPota) return "pota";
  // Unconditional -- matches Android's own `isCQ && modifier == "SOTA"`,
  // which has no highlight-toggle gate at all (unlike POTA just above).
  if (f.isSota) return "sota";
  if (highlights.dxcc && f.isNewDxcc) return "new_dxcc";
  if (highlights.zone && f.isNewZone) return "new_zone";
  if (highlights.state && f.isNewState) return "new_state";
  if (highlights.grid && f.isNewGrid) return "new_grid";
  if (highlights.prefix && f.isNewPrefix) return "new_prefix";
  if (highlights.band && f.isWorkedOtherBand) return "new_band";
  if (highlights.worked && f.isWorked) return "worked";
  if (f.isCq) return "cq";
  return null;
}

/** Label + colors per status, matching StatusPill.kt's QsoStatus enum
 *  exactly (color/bgColor/borderColor, alpha-blended backgrounds included)
 *  rather than reusing the app's existing CSS tokens, since several of
 *  those bg/border alpha values don't correspond to any token this palette
 *  already had. */
export const QSO_STATUS_META: Record<
  QsoStatus,
  { label: string; color: string; bg: string; border: string }
> = {
  pending: { label: "PENDING", color: "#8a96b1", bg: "rgba(138,150,177,0.10)", border: "rgba(138,150,177,0.22)" },
  pota: { label: "POTA", color: "#4ade80", bg: "rgba(74,222,128,0.12)", border: "rgba(74,222,128,0.28)" },
  sota: { label: "SOTA", color: "#4ade80", bg: "rgba(74,222,128,0.12)", border: "rgba(74,222,128,0.28)" },
  new_dxcc: { label: "NEW DXCC", color: "#c084fc", bg: "rgba(192,132,252,0.12)", border: "rgba(192,132,252,0.28)" },
  new_zone: { label: "NEW ZONE", color: "#60a5fa", bg: "rgba(96,165,250,0.12)", border: "rgba(96,165,250,0.28)" },
  new_state: { label: "NEW STATE", color: "#2dd4bf", bg: "rgba(45,212,191,0.12)", border: "rgba(45,212,191,0.28)" },
  new_grid: { label: "NEW GRID", color: "#facc15", bg: "rgba(250,204,21,0.12)", border: "rgba(250,204,21,0.28)" },
  new_prefix: { label: "NEW PFX", color: "#f472b6", bg: "rgba(244,114,182,0.12)", border: "rgba(244,114,182,0.28)" },
  new_band: { label: "NEW BAND", color: "#5cd6e8", bg: "rgba(92,214,232,0.12)", border: "rgba(92,214,232,0.28)" },
  worked: { label: "WORKED", color: "#5cd6e8", bg: "rgba(92,214,232,0.10)", border: "rgba(92,214,232,0.22)" },
  cq: { label: "CQ", color: "#ffaf5e", bg: "rgba(255,175,94,0.12)", border: "rgba(255,175,94,0.28)" },
};
