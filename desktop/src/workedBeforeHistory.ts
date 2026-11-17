/**
 * Pure "Worked before" summarizer for the QSO Sheet, ported from Android's
 * `WorkedBeforeHistory.kt`. Collapses a station's prior QSO rows (from
 * `api.priorQsos`) into the count/last-contact-recap/band-list the sheet's
 * "Worked before" card shows.
 */

export interface PriorQso {
  qsoDate: string;
  timeOn: string;
  band: string | null;
  mode: string | null;
}

export interface WorkedBeforeSummary {
  count: number;
  /** Pre-joined "date · band · mode" recap of the most recent contact (blank parts skipped). */
  lastLine: string;
  /** Distinct bands ever worked, in first-worked order. */
  bands: string[];
}

const PART_SEP = " · "; // " · "

/**
 * Collapse prior QSOs into a WorkedBeforeSummary, or null when there is no
 * prior contact (the caller renders nothing). The most recent QSO is
 * chosen by (qsoDate, timeOn) ascending, independent of input order.
 * Distinct bands are kept in the order they were first worked.
 */
export function summarizeWorkedBefore(qsos: PriorQso[]): WorkedBeforeSummary | null {
  if (qsos.length === 0) return null;

  const sorted = [...qsos].sort((a, b) => {
    const da = a.qsoDate ?? "";
    const db = b.qsoDate ?? "";
    if (da !== db) return da < db ? -1 : 1;
    const ta = a.timeOn ?? "";
    const tb = b.timeOn ?? "";
    if (ta !== tb) return ta < tb ? -1 : 1;
    return 0;
  });
  const last = sorted[sorted.length - 1];

  const lastLine = [formatQsoDate(last.qsoDate), (last.band ?? "").trim(), (last.mode ?? "").trim()]
    .filter((p) => p.length > 0)
    .join(PART_SEP);

  const bands: string[] = [];
  const seen = new Set<string>();
  for (const q of sorted) {
    const b = q.band?.trim();
    if (b && !seen.has(b)) {
      seen.add(b);
      bands.push(b);
    }
  }

  return { count: qsos.length, lastLine, bands };
}

/**
 * Format an ADIF `qso_date` (YYYYMMDD) as YYYY-MM-DD. Anything that isn't
 * exactly 8 digits is returned trimmed and unchanged.
 */
export function formatQsoDate(raw: string | null | undefined): string {
  const d = (raw ?? "").trim();
  if (d.length === 8 && /^\d{8}$/.test(d)) {
    return `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`;
  }
  return d;
}
