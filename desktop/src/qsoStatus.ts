// Per-message status pill, ported from Android's DecodeRow.kt
// (resolveQsoStatus) and StatusPill.kt (QsoStatus enum: label + color). This
// is what the Decode list's compact row trailing pill shows -- replaces a
// plain "is this a CQ" dot with the same priority-ordered, multi-state
// classification Android uses (calling me, POTA, new DXCC/zone/state/grid/
// prefix/band, worked, plain CQ).
//
// Two tiers are left out rather than faked:
//   - SOTA: needs a parsed CQ "modifier" field (e.g. "CQ SOTA K1ABC FN42")
//     the desktop decoder doesn't extract yet.
//   - NEW_POTA (an unhunted park, vs. plain POTA): needs a live pota.app
//     spot feed for the park ref, same gap hunt.ts already documents for
//     Hunt's pota_first priority. isPota here is text-only CQ detection
//     (isPotaCq), so it can only ever produce the plain POTA tier.
//
// highlightX gates below mirror Android's Settings -> Decode Highlights
// *default* values (GeneralVariables.java): DXCC/Zone/Band/Worked/POTA on,
// State/Grid/Prefix off (too noisy on a fresh install). No Settings screen
// exists yet on desktop to change these -- hardcoded so the toggles have
// matching logic to attach to once that screen is built, rather than
// inventing this priority chain from scratch then.

export type QsoStatus =
  | "pending"
  | "pota"
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
  isNewDxcc: boolean;
  isNewZone: boolean;
  isNewState: boolean;
  isNewGrid: boolean;
  isNewPrefix: boolean;
  /** Worked on some other band, but not the one this message was heard on. */
  isWorkedOtherBand: boolean;
  isWorked: boolean;
}

const HIGHLIGHT_DEFAULTS = {
  dxcc: true,
  zone: true,
  state: false,
  grid: false,
  prefix: false,
  band: true,
  worked: true,
  pota: true,
};

/** Highest-priority applicable status, or null for a plain (unremarkable) message. */
export function resolveQsoStatus(f: QsoStatusFlags): QsoStatus | null {
  if (f.isToMe) return "pending";
  if (HIGHLIGHT_DEFAULTS.pota && f.isPota) return "pota";
  if (HIGHLIGHT_DEFAULTS.dxcc && f.isNewDxcc) return "new_dxcc";
  if (HIGHLIGHT_DEFAULTS.zone && f.isNewZone) return "new_zone";
  if (HIGHLIGHT_DEFAULTS.state && f.isNewState) return "new_state";
  if (HIGHLIGHT_DEFAULTS.grid && f.isNewGrid) return "new_grid";
  if (HIGHLIGHT_DEFAULTS.prefix && f.isNewPrefix) return "new_prefix";
  if (HIGHLIGHT_DEFAULTS.band && f.isWorkedOtherBand) return "new_band";
  if (HIGHLIGHT_DEFAULTS.worked && f.isWorked) return "worked";
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
  new_dxcc: { label: "NEW DXCC", color: "#c084fc", bg: "rgba(192,132,252,0.12)", border: "rgba(192,132,252,0.28)" },
  new_zone: { label: "NEW ZONE", color: "#60a5fa", bg: "rgba(96,165,250,0.12)", border: "rgba(96,165,250,0.28)" },
  new_state: { label: "NEW STATE", color: "#2dd4bf", bg: "rgba(45,212,191,0.12)", border: "rgba(45,212,191,0.28)" },
  new_grid: { label: "NEW GRID", color: "#facc15", bg: "rgba(250,204,21,0.12)", border: "rgba(250,204,21,0.28)" },
  new_prefix: { label: "NEW PFX", color: "#f472b6", bg: "rgba(244,114,182,0.12)", border: "rgba(244,114,182,0.28)" },
  new_band: { label: "NEW BAND", color: "#5cd6e8", bg: "rgba(92,214,232,0.12)", border: "rgba(92,214,232,0.28)" },
  worked: { label: "WORKED", color: "#5cd6e8", bg: "rgba(92,214,232,0.10)", border: "rgba(92,214,232,0.22)" },
  cq: { label: "CQ", color: "#ffaf5e", bg: "rgba(255,175,94,0.12)", border: "rgba(255,175,94,0.28)" },
};
