// Pure, framework-free logbook helpers. The React `LogScreen` is a thin wrapper
// around these so the decision/normalization logic stays unit-testable (see
// AGENTS.md: Composable/JSX code can't be tested directly — extract the logic).

import type { QsoRecord } from "./ipc";

/** Logbook confirmation filter, mirroring the Android `FilterDialog`. */
export type ConfirmFilter = "all" | "confirmed" | "unconfirmed";

/** Editable fields surfaced by the Edit-QSO dialog (all ADIF strings). */
export interface QsoEditForm {
  call: string;
  gridsquare: string;
  mode: string;
  rst_sent: string;
  rst_rcvd: string;
  qso_date: string;
  time_on: string;
  qso_date_off: string;
  time_off: string;
  band: string;
  freq: string;
  station_callsign: string;
  my_gridsquare: string;
  comment: string;
  state: string;
  country: string;
  confirmed: boolean;
  qsl_sent: boolean;
  lotw_sent: boolean;
  lotw_rcvd: boolean;
  eqsl_sent: boolean;
  eqsl_rcvd: boolean;
  qrz_qsl: boolean;
}

/**
 * Convert an `<input type="date">` value ("YYYY-MM-DD") to an ADIF date
 * ("YYYYMMDD"). Empty input yields an empty string (open bound). Anything that
 * isn't a plain `YYYY-MM-DD` is passed through with hyphens stripped so a
 * hand-typed ADIF date still works.
 */
export function dateInputToAdif(value: string): string {
  const v = (value ?? "").trim();
  if (!v) return "";
  return v.replace(/-/g, "");
}

/** Inverse of {@link dateInputToAdif}: ADIF "YYYYMMDD" -> "YYYY-MM-DD". */
export function adifToDateInput(value: string): string {
  const v = (value ?? "").trim();
  if (/^\d{8}$/.test(v)) return `${v.slice(0, 4)}-${v.slice(4, 6)}-${v.slice(6, 8)}`;
  return v;
}

/** Format a stored `time_on` ("HHMMSS" or "HHMM", UTC) as "HH:MM" for the
 *  table view. Blank or non-numeric input -> "". */
export function formatHhmm(timeOn: string): string {
  const t = (timeOn ?? "").trim();
  if (t.length < 4 || !/^\d+$/.test(t)) return "";
  return `${t.slice(0, 2)}:${t.slice(2, 4)}`;
}

/**
 * Build the `{start, end}` args for the `export_adif` command from the two
 * date pickers. Returns `undefined` bounds (not empty strings) when a picker is
 * blank so the Rust side treats them as fully open.
 */
export function buildExportArgs(
  startInput: string,
  endInput: string,
): { start?: string; end?: string } {
  const start = dateInputToAdif(startInput);
  const end = dateInputToAdif(endInput);
  return {
    start: start || undefined,
    end: end || undefined,
  };
}

/**
 * Normalize an edit-dialog form into a `QsoRecord` ready for `save_qso`.
 * Callsigns and grids are upper-cased and trimmed to match how the app logs
 * QSOs automatically, so an edit doesn't split a station into two dedup keys.
 */
export function buildQsoRecord(
  form: QsoEditForm,
  id: number | null | undefined,
): QsoRecord {
  const up = (s: string) => (s ?? "").trim().toUpperCase();
  const t = (s: string) => (s ?? "").trim();
  return {
    id: id ?? null,
    call: up(form.call),
    gridsquare: up(form.gridsquare),
    mode: up(form.mode) || "FT8",
    rst_sent: t(form.rst_sent),
    rst_rcvd: t(form.rst_rcvd),
    qso_date: t(form.qso_date),
    time_on: t(form.time_on),
    qso_date_off: t(form.qso_date_off),
    time_off: t(form.time_off),
    band: t(form.band),
    freq: t(form.freq),
    station_callsign: up(form.station_callsign),
    my_gridsquare: up(form.my_gridsquare),
    comment: (form.comment ?? "").trim(),
    state: t(form.state),
    country: t(form.country),
    confirmed: !!form.confirmed,
    qsl_sent: !!form.qsl_sent,
    lotw_sent: !!form.lotw_sent,
    lotw_rcvd: !!form.lotw_rcvd,
    eqsl_sent: !!form.eqsl_sent,
    eqsl_rcvd: !!form.eqsl_rcvd,
    qrz_qsl: !!form.qrz_qsl,
  };
}

/** Seed an edit form from an existing record (or blanks for a new QSO). */
export function formFromQso(r?: QsoRecord | null): QsoEditForm {
  return {
    call: r?.call ?? "",
    gridsquare: r?.gridsquare ?? "",
    mode: r?.mode ?? "FT8",
    rst_sent: r?.rst_sent ?? "",
    rst_rcvd: r?.rst_rcvd ?? "",
    qso_date: r?.qso_date ?? "",
    time_on: r?.time_on ?? "",
    qso_date_off: r?.qso_date_off ?? "",
    time_off: r?.time_off ?? "",
    band: r?.band ?? "",
    freq: r?.freq ?? "",
    station_callsign: r?.station_callsign ?? "",
    my_gridsquare: r?.my_gridsquare ?? "",
    comment: r?.comment ?? "",
    state: r?.state ?? "",
    country: r?.country ?? "",
    confirmed: !!r?.confirmed,
    qsl_sent: !!r?.qsl_sent,
    lotw_sent: !!r?.lotw_sent,
    lotw_rcvd: !!r?.lotw_rcvd,
    eqsl_sent: !!r?.eqsl_sent,
    eqsl_rcvd: !!r?.eqsl_rcvd,
    qrz_qsl: !!r?.qrz_qsl,
  };
}

/** Whether a search string or non-"all" confirmation filter is currently active. */
export function isLogFiltered(search: string, filter: ConfirmFilter): boolean {
  return (search ?? "").trim() !== "" || filter !== "all";
}

/**
 * Header count label for the Logbook bar. With no filter active it's just the
 * total log count; with a search/filter active it shows "shown of total" so the
 * number can't disagree with the rows actually rendered in the table.
 */
export function qsoCountLabel(total: number, shown: number, filtered: boolean): string {
  return filtered ? `${shown} of ${total}` : `${total}`;
}

/**
 * Client-side fallback filter (substring call match + confirmation), matching
 * the Rust `qso_matches` predicate. Used when filtering an already-loaded page
 * without a server round-trip.
 */
export function filterQsos(
  rows: QsoRecord[],
  search: string,
  filter: ConfirmFilter,
): QsoRecord[] {
  const q = (search ?? "").trim().toUpperCase();
  return rows.filter((r) => {
    const callOk = !q || (r.call ?? "").toUpperCase().includes(q);
    const confirmOk =
      filter === "all" ||
      (filter === "confirmed" ? r.confirmed : !r.confirmed);
    return callOk && confirmOk;
  });
}

// ---------------------------------------------------------------------------
// Sortable spreadsheet table (ports the Android Logbook table's format --
// `LogbookScreen.kt`'s QsoTableHeader/QsoTableRow/sortQsoTable/qslMarks).
// ---------------------------------------------------------------------------

/** Column indices of the table view, in display order -- mirrors Android's
 *  QSO_COL_* constants exactly, so the two apps' sort behavior stays in sync. */
export const QSO_COL_DATE = 0;
export const QSO_COL_TIME = 1;
export const QSO_COL_CALL = 2;
export const QSO_COL_GRID = 3;
export const QSO_COL_STATE = 4;
export const QSO_COL_FREQ = 5;
export const QSO_COL_MODE = 6;
export const QSO_COL_RST_SENT = 7;
export const QSO_COL_RST_RCVD = 8;
export const QSO_COL_STATUS = 9;
export const QSO_COL_QSL = 10;

// Labels with a "\n" stack onto two lines (see .qso-table th's
// white-space: pre-line) -- narrower columns for the wider headers, freeing
// up width for the rest of the table, matching Android's header style.
export const QSO_TABLE_COLUMNS = [
  { col: QSO_COL_DATE, label: "QSO\nDate" },
  { col: QSO_COL_TIME, label: "QSO\nTime" },
  { col: QSO_COL_CALL, label: "Callsign" },
  { col: QSO_COL_GRID, label: "Grid" },
  { col: QSO_COL_STATE, label: "State" },
  { col: QSO_COL_FREQ, label: "Freq\nMHz" },
  { col: QSO_COL_MODE, label: "Mode" },
  { col: QSO_COL_RST_SENT, label: "RST\nSent" },
  { col: QSO_COL_RST_RCVD, label: "RST\nRcvd" },
  { col: QSO_COL_STATUS, label: "Status" },
  { col: QSO_COL_QSL, label: "QSL" },
] as const;

/** Column width weights, same order as `QSO_TABLE_COLUMNS`, matching
 *  Android's `QsoTableWeights` exactly (`LogbookScreen.kt`) -- the
 *  trailing fixed-width delete-button column isn't weighted, same as
 *  Android's own `QsoTableMenuWidth` (36dp) sits outside this list. */
export const QSO_TABLE_WEIGHTS = [1.02, 0.58, 0.95, 0.62, 0.75, 0.68, 0.75, 0.55, 0.55, 0.62, 0.98] as const;

/**
 * The State column's display value: a non-US logged COUNTRY is shown as is;
 * for the US (or no logged country) it's the logged STATE. `null` means
 * nothing to show.
 *
 * Android's equivalent (`logbookPlace`) also falls back to a grid->state
 * lookup when nothing was explicitly logged; this app has no such lookup yet
 * (see `fork-changes.md`), so this is Android's function with that lookup
 * always empty -- a station worked but never logged/imported with a STATE
 * shows blank here even where the Android app would derive one from the grid.
 */
export function logbookPlace(r: QsoRecord): string | null {
  const country = (r.country ?? "").trim();
  const isUs = country === "" || /^united states( of america)?$|^usa$/i.test(country);
  if (!isUs) return country;
  const state = (r.state ?? "").trim();
  if (state) return state;
  return country || null;
}

/** QSL column marks: LoTW, eQSL, Paper, QRZ.com, each "received" (✓), "sent"
 *  (↑, only when not received) or "neither" (·). QRZ has no "sent" side --
 *  the upload itself is `synced_qrz`, not a QSL confirmation flag. */
export interface QslMark {
  service: "L" | "E" | "P" | "Q";
  mark: "✓" | "↑" | "·";
}

export function qslMarks(r: QsoRecord): QslMark[] {
  const mark = (rcvd: boolean, sent: boolean): QslMark["mark"] =>
    rcvd ? "✓" : sent ? "↑" : "·";
  return [
    { service: "L", mark: mark(!!r.lotw_rcvd, !!r.lotw_sent) },
    { service: "E", mark: mark(!!r.eqsl_rcvd, !!r.eqsl_sent) },
    { service: "P", mark: mark(!!r.confirmed, !!r.qsl_sent) },
    { service: "Q", mark: mark(!!r.qrz_qsl, false) },
  ];
}

/** Sortable QSL score: received counts double, sent single -- an all-received
 *  row (L✓E✓P✓Q✓) sorts above a mostly-sent one (L✓E↑P·Q·). */
function qslScore(r: QsoRecord): number {
  return qslMarks(r).reduce((sum, { mark }) => sum + (mark === "✓" ? 2 : mark === "↑" ? 1 : 0), 0);
}

/**
 * Status pill this row would show: "CONF" when LoTW or QRZ.com confirmed,
 * else none. User's explicit call (2026-11-03, ported from the Android
 * decision): no WORKED tier, and no upload-service chips in this cell --
 * the QRZ tag read as a confirmation when it only meant "uploaded," and the
 * QSL column's L/E/P/Q marks already show which service actually confirmed
 * it. eQSL and paper never set CONF on their own.
 */
export function qsoStatusLabel(r: QsoRecord): "CONF" | null {
  return r.lotw_rcvd || r.qrz_qsl ? "CONF" : null;
}

/**
 * Sync-to-service chip labels shown beside the Status pill -- which online
 * logbooks this row has already been uploaded to ("CL" for Cloudlog/
 * Wavelog/Nextlog, "QRZ" for QRZ.com), ported from Android's
 * `syncChipLabels`/`SyncChips`. Previously omitted here on the theory that
 * the QRZ tag read as a confirmation when it only means "uploaded" -- the
 * user's direct call was to match Android exactly instead, so this is back;
 * `qsoStatusLabel`'s own CONF-vs-not logic is unchanged (it never looked at
 * these flags for confirmation, only `lotw_rcvd`/`qrz_qsl`). Android also
 * has a "WRL" (World Radio League) chip; desktop has no WRL upload tracking
 * to port that from, so only CL/QRZ apply. No distinct Wavelog/Nextlog
 * hostname label either (Android picks "WL"/"NL"/"CL" from the configured
 * server address) -- desktop's `synced_cloudlog` doesn't carry which one it
 * was, so this always shows the generic "CL".
 */
export function qsoSyncLabels(r: QsoRecord): string[] {
  const labels: string[] = [];
  if (r.synced_cloudlog) labels.push("CL");
  if (r.synced_qrz) labels.push("QRZ");
  return labels;
}

/** Sortable status score: confirmed first, then newest (via the stable sort's
 *  newest-first baseline) -- matches Android's sort rule for the same column. */
function qsoStatusScore(r: QsoRecord): number {
  return qsoStatusLabel(r) ? 1 : 0;
}

function dateTimeKey(r: QsoRecord): string {
  // `time_on` is always HHMMSS (6 digits) from this app's own writes, but an
  // import can carry a shorter HHMM. That's still safe to compare unpadded:
  // HHMM is a true string prefix of its own HHMMSS (same leading digits), so
  // plain lexicographic comparison already orders it correctly either way.
  return `${r.qso_date ?? ""}${r.time_on ?? ""}`;
}

function sortByDateTimeDesc(rows: QsoRecord[]): QsoRecord[] {
  return [...rows].sort((a, b) => {
    const ak = dateTimeKey(a);
    const bk = dateTimeKey(b);
    return ak < bk ? 1 : ak > bk ? -1 : 0;
  });
}

function parseNumericField(s: string): number | null {
  const t = (s ?? "").trim().replace(/\+/g, "");
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

/** Android's "no report" sentinels (it stores a real `-120`/`-100` int and
 *  writes it to ADIF literally -- a report that could in principle reach this
 *  app by importing one of its exports), or desktop's own "absent" encoding
 *  (ADIF `adif_record_to_qso` leaves the field empty when the tag is absent). */
function isNoReport(rst: string): boolean {
  const t = (rst ?? "").trim();
  return t === "" || t === "-120" || t === "-100";
}

/** An RST for display: Android's displayReport, ported -- "no report" shows
 *  blank rather than a literal "-120"/"-100". */
export function displayReport(rst: string): string {
  return isNoReport(rst) ? "" : rst;
}

/**
 * Sort the table view by `column` (one of the `QSO_COL_*` constants). Date
 * and time sort together (by actual QSO date+time, not insertion order --
 * an imported historical log is not in id order); frequency and reports
 * sort numerically with blanks last; status/QSL sort by their score. Ties
 * fall back to newest-first, relying on `Array.prototype.sort`'s guaranteed
 * stability (ES2019+), same as the Kotlin original relies on `sortedWith`.
 */
export function sortQsoTable(rows: QsoRecord[], column: number, ascending: boolean): QsoRecord[] {
  const newestFirst = sortByDateTimeDesc(rows);
  if (column === QSO_COL_DATE || column === QSO_COL_TIME) {
    return ascending ? [...newestFirst].reverse() : newestFirst;
  }

  const numKey: ((r: QsoRecord) => number | null) | null = {
    [QSO_COL_FREQ]: (r: QsoRecord) => parseNumericField(r.freq),
    [QSO_COL_RST_SENT]: (r: QsoRecord) => (isNoReport(r.rst_sent) ? null : parseNumericField(r.rst_sent)),
    [QSO_COL_RST_RCVD]: (r: QsoRecord) => (isNoReport(r.rst_rcvd) ? null : parseNumericField(r.rst_rcvd)),
    [QSO_COL_STATUS]: (r: QsoRecord) => qsoStatusScore(r),
    [QSO_COL_QSL]: (r: QsoRecord) => qslScore(r),
  }[column] ?? null;
  if (numKey) {
    return [...newestFirst].sort((a, b) => {
      const x = numKey(a);
      const y = numKey(b);
      if (x == null && y == null) return 0;
      if (x == null) return 1;
      if (y == null) return -1;
      return ascending ? x - y : y - x;
    });
  }

  const textKey: ((r: QsoRecord) => string) | null = {
    [QSO_COL_CALL]: (r: QsoRecord) => (r.call ?? "").toUpperCase(),
    [QSO_COL_GRID]: (r: QsoRecord) => (r.gridsquare ?? "").toUpperCase(),
    [QSO_COL_STATE]: (r: QsoRecord) => (logbookPlace(r) ?? "").toUpperCase(),
    [QSO_COL_MODE]: (r: QsoRecord) => (r.mode ?? "").toUpperCase(),
  }[column] ?? null;
  if (textKey) {
    return [...newestFirst].sort((a, b) => {
      const x = textKey(a);
      const y = textKey(b);
      if (x === "" && y === "") return 0;
      if (x === "") return 1;
      if (y === "") return -1;
      if (x === y) return 0;
      const cmp = x < y ? -1 : 1;
      return ascending ? cmp : -cmp;
    });
  }

  return newestFirst;
}

/** A virtualized table's visible window: which row indices to actually
 *  render, and how tall the two spacer rows before/after them need to be. */
export interface VirtualWindow {
  startIndex: number;
  endIndex: number;
  topSpacerPx: number;
  bottomSpacerPx: number;
}

/**
 * Compute which row slice of a long table to actually mount as DOM nodes.
 *
 * Confirmed necessary on real data: rendering a plain (non-virtualized) table
 * for a ~54,500-row log pinned the webview's renderer at 100% CPU for
 * minutes with nothing on screen -- removing the row-count cap on the fetch
 * (so sorting could see the whole log, not an arbitrary insertion-order
 * window) made the UI itself the new bottleneck. Only `[startIndex, endIndex)`
 * need to exist as real rows; the rest of the scroll range is represented by
 * two spacer rows sized in pixels, so the scrollbar still reflects the true
 * total.
 */
export function computeVirtualWindow(
  totalRows: number,
  rowHeightPx: number,
  scrollTopPx: number,
  viewportHeightPx: number,
  overscan: number,
): VirtualWindow {
  if (totalRows <= 0 || rowHeightPx <= 0) {
    return { startIndex: 0, endIndex: 0, topSpacerPx: 0, bottomSpacerPx: 0 };
  }
  const visibleCount = Math.ceil(viewportHeightPx / rowHeightPx) + overscan * 2;
  const startIndex = Math.max(0, Math.floor(scrollTopPx / rowHeightPx) - overscan);
  const endIndex = Math.min(totalRows, startIndex + visibleCount);
  return {
    startIndex,
    endIndex,
    topSpacerPx: startIndex * rowHeightPx,
    bottomSpacerPx: (totalRows - endIndex) * rowHeightPx,
  };
}

// ---------------------------------------------------------------------------
// Worked All Continents (WAC) -- ported from Android's
// WorkedAllContinents.kt. Antarctica ("AN") is an optional endorsement, not
// one of the six required continents, so it never counts toward the total.
// ---------------------------------------------------------------------------

/** The six WAC continents, in a conventional display order. */
export const WAC_CONTINENTS = ["NA", "SA", "EU", "AF", "AS", "OC"] as const;

export interface ContinentChip {
  code: (typeof WAC_CONTINENTS)[number];
  worked: boolean;
}

export interface WacProgress {
  chips: ContinentChip[];
  workedCount: number;
  total: number;
  isComplete: boolean;
}

/** Reduce raw continent codes (as `db.rs`'s `worked_continents` returns them,
 *  e.g. "NA", "EU", or a stray unmatched entry) into WAC award progress.
 *  Codes are trimmed/upper-cased before matching, duplicates collapse, and
 *  anything outside the six standard continents is ignored so it can never
 *  inflate the total. The chip list is always the six WAC_CONTINENTS in
 *  canonical order, so the UI renders a stable row regardless of input order. */
export function workedAllContinents(rawCodes: (string | null | undefined)[]): WacProgress {
  const worked = new Set(
    rawCodes
      .filter((c): c is string => !!c)
      .map((c) => c.trim().toUpperCase())
      .filter((c): c is (typeof WAC_CONTINENTS)[number] => (WAC_CONTINENTS as readonly string[]).includes(c)),
  );
  const chips = WAC_CONTINENTS.map((code) => ({ code, worked: worked.has(code) }));
  return {
    chips,
    workedCount: worked.size,
    total: WAC_CONTINENTS.length,
    isComplete: worked.size >= WAC_CONTINENTS.length,
  };
}

// ---------------------------------------------------------------------------
// Worked All States (WAS) -- ported from Android's WorkedAllStates.kt.
// Washington DC is intentionally excluded: it is not one of the 50 WAS
// states, even though a grid can resolve to "DC".
// ---------------------------------------------------------------------------

/** The 50 US states (two-letter USPS abbreviations), the ARRL WAS award basis. */
export const WAS_STATES = [
  "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA",
  "HI", "ID", "IL", "IN", "IA", "KS", "KY", "LA", "ME", "MD",
  "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ",
  "NM", "NY", "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC",
  "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV", "WI", "WY",
] as const;

/** The WAS states still needed, in WAS_STATES' canonical order, given the set
 *  already worked (`db.rs`'s `worked_states_all`, already resolved/
 *  upper-cased server-side). Anything in `worked` that isn't a WAS state
 *  (e.g. "DC") is ignored. */
export function neededStates(worked: string[]): string[] {
  const have = new Set(worked.map((s) => s.trim().toUpperCase()));
  return WAS_STATES.filter((s) => !have.has(s));
}

/** Split a remaining-states list into the chips to show (at most `max`) and
 *  the count hidden behind a trailing "+N" overflow chip, so a WAS card can
 *  preview what's left without ever rendering all 50. `max` <= 0 shows
 *  nothing and reports the whole list as overflow. */
export function neededStatesPreview(remaining: string[], max: number): { shown: string[]; overflow: number } {
  if (max <= 0) return { shown: [], overflow: remaining.length };
  if (remaining.length <= max) return { shown: remaining, overflow: 0 };
  return { shown: remaining.slice(0, max), overflow: remaining.length - max };
}

/** How many states the WAS award preview shows before collapsing the rest
 *  into a "+N" chip -- matches Android's NEEDED_STATES_PREVIEW_MAX. */
export const NEEDED_STATES_PREVIEW_MAX = 12;

// ---------------------------------------------------------------------------
// Band color mapping -- ported from Android's LogbookScreen.kt bandColor(),
// used by the Stats tab's band donut chart and legend.
// ---------------------------------------------------------------------------

const BAND_COLOR_MAP: Record<string, string> = {
  "20M": "#5CD6E8",
  "15M": "#4ADE80",
  "40M": "#FFAF5E",
  "10M": "#C084FC",
  "30M": "#F472B6",
  "17M": "#FACC15",
  "12M": "#FB7185",
};

/** The donut/legend color for a band name, or Android's TextMuted fallback
 *  for a band with no assigned color. */
export function bandColor(band: string): string {
  return BAND_COLOR_MAP[band.trim().toUpperCase()] ?? "#8A96B1";
}

// ---------------------------------------------------------------------------
// Grid square coverage heatmap (18x18 field grid: AA..RR at Maidenhead field
// level) -- ported from Android's LogbookScreen.kt. BOTH axes span the full
// A..R range (the latitude field is a letter, not a digit), so the coverage
// grid is 18x18 (324 cells); iterating fewer latitude rows would silently
// drop every field north of +10 degrees latitude -- essentially all of North
// America, Europe, and Japan.
// ---------------------------------------------------------------------------

export const GRID_HEATMAP_COLS = 18;
export const GRID_HEATMAP_ROWS = 18;

export interface GridHeatmapCell {
  field: string;
  isWorked: boolean;
}

/** The full 18x18 coverage grid, row-major (row = latitude field A..R, col =
 *  longitude field A..R), each cell flagged worked against `workedFields`
 *  (`db.rs`'s `worked_grid_fields`, already-uppercased two-letter fields). */
export function buildGridHeatmapCells(workedFields: Set<string>): GridHeatmapCell[][] {
  const rows: GridHeatmapCell[][] = [];
  for (let row = 0; row < GRID_HEATMAP_ROWS; row++) {
    const cols: GridHeatmapCell[] = [];
    for (let col = 0; col < GRID_HEATMAP_COLS; col++) {
      const field = String.fromCharCode(65 + col) + String.fromCharCode(65 + row);
      cols.push({ field, isWorked: workedFields.has(field) });
    }
    rows.push(cols);
  }
  return rows;
}

// ---------------------------------------------------------------------------
// Distance formatting for the Best DX card.
// ---------------------------------------------------------------------------

const KM_TO_MILES = 0.621371;

/** Whole-number distance with a "mi" unit, matching Android's
 *  `MaidenheadGrid.formatDist` ("%.0f %s"). Android lets the operator choose
 *  mi/km (`GeneralVariables.distanceInMiles`); desktop has no such setting
 *  yet, so this always shows miles -- a documented gap, not a guess: every
 *  real sample seen so far (an imported Android QSO's own "Distance: 788 mi"
 *  comment) was already in miles. */
export function formatDistanceMi(km: number): string {
  return `${Math.round(km * KM_TO_MILES)} mi`;
}
