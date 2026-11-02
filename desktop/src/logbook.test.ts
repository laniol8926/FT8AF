import { describe, it, expect } from "vitest";
import {
  adifToDateInput,
  buildExportArgs,
  buildQsoRecord,
  computeVirtualWindow,
  dateInputToAdif,
  filterQsos,
  formatHhmm,
  formatImportSummary,
  formFromQso,
  isLogFiltered,
  logbookPlace,
  qslMarks,
  qsoCountLabel,
  qsoStatusLabel,
  QSO_COL_CALL,
  QSO_COL_DATE,
  QSO_COL_FREQ,
  QSO_COL_MODE,
  QSO_COL_QSL,
  QSO_COL_RST_SENT,
  QSO_COL_STATE,
  QSO_COL_STATUS,
  sortQsoTable,
  syncChipLabels,
  type QsoEditForm,
} from "./logbook";
import type { QsoRecord } from "./ipc";

function qso(over: Partial<QsoRecord>): QsoRecord {
  return {
    id: 1,
    call: "K1ABC",
    gridsquare: "FN42",
    mode: "FT8",
    rst_sent: "-12",
    rst_rcvd: "-08",
    qso_date: "20260604",
    time_on: "120000",
    qso_date_off: "",
    time_off: "",
    band: "20m",
    freq: "14.074",
    station_callsign: "K0XYZ",
    my_gridsquare: "EN37",
    comment: "",
    state: "",
    country: "",
    confirmed: false,
    qsl_sent: false,
    lotw_sent: false,
    lotw_rcvd: false,
    eqsl_sent: false,
    eqsl_rcvd: false,
    qrz_qsl: false,
    synced_cloudlog: false,
    synced_qrz: false,
    ...over,
  };
}

describe("dateInputToAdif / adifToDateInput", () => {
  it("converts date-picker values to ADIF and back", () => {
    expect(dateInputToAdif("2026-06-15")).toBe("20260615");
    expect(dateInputToAdif("")).toBe("");
    expect(dateInputToAdif("  ")).toBe("");
    expect(adifToDateInput("20260615")).toBe("2026-06-15");
    expect(adifToDateInput("")).toBe("");
    // Non-8-digit values pass through.
    expect(adifToDateInput("2026")).toBe("2026");
  });
});

describe("formatHhmm", () => {
  it("formats HHMMSS and HHMM as HH:MM", () => {
    expect(formatHhmm("123045")).toBe("12:30");
    expect(formatHhmm("1230")).toBe("12:30");
  });

  it("blank or non-numeric input is empty", () => {
    expect(formatHhmm("")).toBe("");
    expect(formatHhmm("12:30")).toBe("");
    expect(formatHhmm("ab")).toBe("");
  });
});

describe("buildExportArgs", () => {
  it("omits blank bounds so they are open", () => {
    expect(buildExportArgs("", "")).toEqual({ start: undefined, end: undefined });
    expect(buildExportArgs("2026-06-01", "")).toEqual({
      start: "20260601",
      end: undefined,
    });
    expect(buildExportArgs("2026-06-01", "2026-06-30")).toEqual({
      start: "20260601",
      end: "20260630",
    });
  });
});

describe("buildQsoRecord", () => {
  it("upper-cases call/grid, trims, defaults mode, carries id + confirmed", () => {
    const form: QsoEditForm = {
      call: " k1abc ",
      gridsquare: "fn42",
      mode: "",
      rst_sent: " -12 ",
      rst_rcvd: "-08",
      qso_date: "20260604",
      time_on: "120000",
      qso_date_off: "",
      time_off: "",
      band: "20m",
      freq: "14.074",
      station_callsign: "k0xyz",
      my_gridsquare: "en37",
      comment: " hi ",
      state: "",
      country: "",
      confirmed: true,
      qsl_sent: false,
      lotw_sent: false,
      lotw_rcvd: false,
      eqsl_sent: false,
      eqsl_rcvd: false,
      qrz_qsl: false,
    };
    const rec = buildQsoRecord(form, 7);
    expect(rec.id).toBe(7);
    expect(rec.call).toBe("K1ABC");
    expect(rec.gridsquare).toBe("FN42");
    expect(rec.mode).toBe("FT8"); // blank -> default
    expect(rec.rst_sent).toBe("-12");
    expect(rec.station_callsign).toBe("K0XYZ");
    expect(rec.my_gridsquare).toBe("EN37");
    expect(rec.comment).toBe("hi");
    expect(rec.confirmed).toBe(true);
  });

  it("uses null id for a new QSO", () => {
    const rec = buildQsoRecord(formFromQso(null), undefined);
    expect(rec.id).toBeNull();
    expect(rec.mode).toBe("FT8");
    expect(rec.confirmed).toBe(false);
  });
});

describe("formFromQso round-trip", () => {
  it("seeds a form from a record and rebuilds an equivalent record", () => {
    const original = qso({ confirmed: true, comment: "gm" });
    const rebuilt = buildQsoRecord(formFromQso(original), original.id);
    // synced_cloudlog/synced_qrz aren't in the edit form (they're read-only,
    // set by syncing, not editing) -- the round trip legitimately drops them.
    const { synced_cloudlog, synced_qrz, ...editable } = original;
    void synced_cloudlog;
    void synced_qrz;
    expect(rebuilt).toEqual(editable);
  });
});

describe("filterQsos", () => {
  const rows = [
    qso({ id: 1, call: "K1ABC", confirmed: true }),
    qso({ id: 2, call: "K1XYZ", confirmed: false }),
    qso({ id: 3, call: "W1AW", confirmed: true }),
  ];

  it("matches callsign substrings case-insensitively", () => {
    expect(filterQsos(rows, "k1", "all").map((r) => r.id)).toEqual([1, 2]);
    expect(filterQsos(rows, "  ", "all").length).toBe(3);
    expect(filterQsos(rows, "zzz", "all").length).toBe(0);
  });

  it("filters by confirmation status", () => {
    expect(filterQsos(rows, "", "confirmed").map((r) => r.id)).toEqual([1, 3]);
    expect(filterQsos(rows, "", "unconfirmed").map((r) => r.id)).toEqual([2]);
  });

  it("combines search and confirmation", () => {
    expect(filterQsos(rows, "k1", "confirmed").map((r) => r.id)).toEqual([1]);
  });
});

describe("isLogFiltered / qsoCountLabel", () => {
  it("detects an active search or non-all filter", () => {
    expect(isLogFiltered("", "all")).toBe(false);
    expect(isLogFiltered("   ", "all")).toBe(false);
    expect(isLogFiltered("k1", "all")).toBe(true);
    expect(isLogFiltered("", "confirmed")).toBe(true);
    expect(isLogFiltered("", "unconfirmed")).toBe(true);
  });

  it("shows the plain total when unfiltered and 'shown of total' when filtered", () => {
    expect(qsoCountLabel(42, 42, false)).toBe("42");
    // Filtered: the label reflects the rows actually shown, not the full total.
    expect(qsoCountLabel(42, 3, true)).toBe("3 of 42");
  });
});

describe("formatImportSummary", () => {
  it("shows just the imported count when nothing else happened", () => {
    expect(formatImportSummary({ imported: 5, duplicates: 0, skipped: 0 })).toBe("5 imported");
  });

  it("adds duplicates and skipped only when present", () => {
    expect(formatImportSummary({ imported: 3, duplicates: 2, skipped: 0 })).toBe(
      "3 imported, 2 already logged",
    );
    expect(formatImportSummary({ imported: 0, duplicates: 0, skipped: 1 })).toBe(
      "0 imported, 1 skipped",
    );
    expect(formatImportSummary({ imported: 1, duplicates: 1, skipped: 1 })).toBe(
      "1 imported, 1 already logged, 1 skipped",
    );
  });
});

describe("logbookPlace", () => {
  it("shows the logged state for a US QSO", () => {
    expect(logbookPlace(qso({ state: "CA", country: "" }))).toBe("CA");
    expect(logbookPlace(qso({ state: "CA", country: "United States" }))).toBe("CA");
  });

  it("falls back to country when no state is logged", () => {
    expect(logbookPlace(qso({ state: "", country: "United States" }))).toBe("United States");
    expect(logbookPlace(qso({ state: "", country: "" }))).toBeNull();
  });

  it("shows the country as-is for a non-US QSO, ignoring any logged state", () => {
    expect(logbookPlace(qso({ state: "WA", country: "Australia" }))).toBe("Australia");
  });
});

describe("qslMarks", () => {
  it("marks received as ✓, sent-only as ↑, neither as ·", () => {
    const r = qso({
      lotw_rcvd: true,
      eqsl_sent: true,
      confirmed: false,
      qsl_sent: false,
      qrz_qsl: false,
    });
    expect(qslMarks(r)).toEqual([
      { service: "L", mark: "✓" },
      { service: "E", mark: "↑" },
      { service: "P", mark: "·" },
      { service: "Q", mark: "·" },
    ]);
  });

  it("QRZ has no sent state -- it's either received or neither", () => {
    expect(qslMarks(qso({ qrz_qsl: true }))[3]).toEqual({ service: "Q", mark: "✓" });
    expect(qslMarks(qso({ qrz_qsl: false }))[3]).toEqual({ service: "Q", mark: "·" });
  });
});

describe("qsoStatusLabel", () => {
  it("LoTW confirmation wins as CONF over any other signal", () => {
    expect(qsoStatusLabel(qso({ lotw_rcvd: true, confirmed: true }))).toBe("CONF");
  });

  it("any non-LoTW confirmation is WORKED", () => {
    expect(qsoStatusLabel(qso({ confirmed: true }))).toBe("WORKED");
    expect(qsoStatusLabel(qso({ eqsl_rcvd: true }))).toBe("WORKED");
    expect(qsoStatusLabel(qso({ qrz_qsl: true }))).toBe("WORKED");
  });

  it("no confirmation at all is no pill", () => {
    expect(qsoStatusLabel(qso({}))).toBeNull();
  });
});

describe("syncChipLabels", () => {
  it("lists only the services actually synced, in a fixed order", () => {
    expect(syncChipLabels(qso({}))).toEqual([]);
    expect(syncChipLabels(qso({ synced_cloudlog: true }))).toEqual(["CL"]);
    expect(syncChipLabels(qso({ synced_qrz: true }))).toEqual(["QRZ"]);
    expect(syncChipLabels(qso({ synced_cloudlog: true, synced_qrz: true }))).toEqual([
      "CL",
      "QRZ",
    ]);
  });
});

describe("sortQsoTable", () => {
  const rows = [
    qso({ id: 1, call: "W1AW", qso_date: "20260601", time_on: "100000", freq: "14.074", rst_sent: "-12" }),
    qso({ id: 2, call: "K1ABC", qso_date: "20260603", time_on: "120000", freq: "7.074", rst_sent: "+05" }),
    qso({ id: 3, call: "N0CALL", qso_date: "20260602", time_on: "150000", freq: "", rst_sent: "" }),
  ];

  it("date/time sort by actual QSO date+time, not insertion order", () => {
    const desc = sortQsoTable(rows, QSO_COL_DATE, false);
    expect(desc.map((r) => r.call)).toEqual(["K1ABC", "N0CALL", "W1AW"]);
    const asc = sortQsoTable(rows, QSO_COL_DATE, true);
    expect(asc.map((r) => r.call)).toEqual(["W1AW", "N0CALL", "K1ABC"]);
  });

  it("sorts a text column case-insensitively, blanks last either direction", () => {
    const withBlank = [...rows, qso({ id: 4, call: "", qso_date: "20260604", time_on: "090000" })];
    const asc = sortQsoTable(withBlank, QSO_COL_CALL, true);
    expect(asc.map((r) => r.call)).toEqual(["K1ABC", "N0CALL", "W1AW", ""]);
    const desc = sortQsoTable(withBlank, QSO_COL_CALL, false);
    expect(desc.map((r) => r.call)).toEqual(["W1AW", "N0CALL", "K1ABC", ""]);
  });

  it("sorts a numeric column with blanks last, ascending and descending", () => {
    const asc = sortQsoTable(rows, QSO_COL_FREQ, true);
    expect(asc.map((r) => r.call)).toEqual(["K1ABC", "W1AW", "N0CALL"]);
    const desc = sortQsoTable(rows, QSO_COL_FREQ, false);
    expect(desc.map((r) => r.call)).toEqual(["W1AW", "K1ABC", "N0CALL"]);
  });

  it("strips a leading + from a report before parsing it numerically", () => {
    const asc = sortQsoTable(rows, QSO_COL_RST_SENT, true);
    // -12 < +05 numerically; the blank report sorts last.
    expect(asc.map((r) => r.call)).toEqual(["W1AW", "K1ABC", "N0CALL"]);
  });

  it("sorts by status score: CONF above WORKED above none", () => {
    const statusRows = [
      qso({ id: 1, call: "A", qso_date: "20260601", time_on: "100000" }),
      qso({ id: 2, call: "B", qso_date: "20260601", time_on: "110000", confirmed: true }),
      qso({ id: 3, call: "C", qso_date: "20260601", time_on: "120000", lotw_rcvd: true }),
    ];
    const desc = sortQsoTable(statusRows, QSO_COL_STATUS, false);
    expect(desc.map((r) => r.call)).toEqual(["C", "B", "A"]);
  });

  it("sorts by QSL score: more ✓ marks above more ↑ marks above none", () => {
    const qslRows = [
      qso({ id: 1, call: "A", qso_date: "20260601", time_on: "100000" }),
      qso({ id: 2, call: "B", qso_date: "20260601", time_on: "110000", lotw_sent: true }),
      qso({ id: 3, call: "C", qso_date: "20260601", time_on: "120000", lotw_rcvd: true, eqsl_rcvd: true }),
    ];
    const desc = sortQsoTable(qslRows, QSO_COL_QSL, false);
    expect(desc.map((r) => r.call)).toEqual(["C", "B", "A"]);
  });

  it("sorts the State column using logbookPlace's display value", () => {
    const stateRows = [
      qso({ id: 1, call: "A", qso_date: "20260601", time_on: "100000", state: "WA" }),
      qso({ id: 2, call: "B", qso_date: "20260601", time_on: "110000", state: "CA" }),
      qso({ id: 3, call: "C", qso_date: "20260601", time_on: "120000", state: "", country: "" }),
    ];
    const asc = sortQsoTable(stateRows, QSO_COL_STATE, true);
    expect(asc.map((r) => r.call)).toEqual(["B", "A", "C"]);
  });

  it("ties fall back to newest-first (stable sort)", () => {
    // Both rows have the same MODE ("FT8"); a tied text column must keep the
    // date/time-desc baseline order rather than reshuffling arbitrarily.
    const tied = [
      qso({ id: 1, call: "A", qso_date: "20260603", time_on: "090000" }),
      qso({ id: 2, call: "B", qso_date: "20260602", time_on: "090000" }),
    ];
    const byMode = sortQsoTable(tied, QSO_COL_MODE, true);
    expect(byMode.map((r) => r.call)).toEqual(["A", "B"]); // unchanged: newest-first
  });
});

describe("computeVirtualWindow", () => {
  it("renders only the visible slice plus overscan, with correctly sized spacers", () => {
    // 10,000 rows, 26px each, viewport 260px (10 rows visible), scrolled to row 1000.
    const w = computeVirtualWindow(10_000, 26, 1000 * 26, 260, 5);
    expect(w.startIndex).toBe(995); // 1000 - overscan
    expect(w.endIndex).toBe(995 + Math.ceil(260 / 26) + 10);
    expect(w.topSpacerPx).toBe(995 * 26);
    expect(w.bottomSpacerPx).toBe((10_000 - w.endIndex) * 26);
  });

  it("clamps the start to 0 near the top and the end to the row count near the bottom", () => {
    const top = computeVirtualWindow(100, 26, 0, 260, 5);
    expect(top.startIndex).toBe(0);
    expect(top.topSpacerPx).toBe(0);

    const bottom = computeVirtualWindow(100, 26, 100 * 26, 260, 5);
    expect(bottom.endIndex).toBe(100);
    expect(bottom.bottomSpacerPx).toBe(0);
  });

  it("an empty or degenerate table renders nothing and needs no spacers", () => {
    expect(computeVirtualWindow(0, 26, 0, 260, 5)).toEqual({
      startIndex: 0,
      endIndex: 0,
      topSpacerPx: 0,
      bottomSpacerPx: 0,
    });
    expect(computeVirtualWindow(100, 0, 0, 260, 5).endIndex).toBe(0);
  });

  it("the rendered slice always covers the actual viewport (no blank gap while scrolling)", () => {
    const w = computeVirtualWindow(10_000, 26, 5000, 400, 3);
    const firstVisibleRow = Math.floor(5000 / 26);
    const lastVisibleRow = Math.floor((5000 + 400) / 26);
    expect(w.startIndex).toBeLessThanOrEqual(firstVisibleRow);
    expect(w.endIndex).toBeGreaterThanOrEqual(lastVisibleRow);
  });
});
