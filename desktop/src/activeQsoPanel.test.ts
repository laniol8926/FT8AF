import { describe, expect, it } from "vitest";
import {
  buildQsoLog,
  decideTxLog,
  formatTxLogText,
  mergeRxLog,
  qsoHeaderText,
  qsoLogKey,
  type QsoLogEntry,
} from "./activeQsoPanel";
import type { UiMessage } from "./ipc";

function msg(overrides: Partial<UiMessage> = {}): UiMessage {
  return {
    utc_ms: 1_000,
    call_to: "K0XYZ",
    call_from: "RA3XYZ",
    extra: "-10",
    grid: "",
    snr: -10,
    freq_hz: 1500,
    time_sec: 0.5,
    text: "K0XYZ RA3XYZ -10",
    is_cq: false,
    to_me: true,
    dxcc_prefix: null,
    cq_zone: null,
    state: null,
    ...overrides,
  };
}

describe("buildQsoLog", () => {
  it("returns empty for an empty target", () => {
    expect(buildQsoLog([msg()], "")).toEqual([]);
  });

  it("classifies a message from the target to us as rx", () => {
    const result = buildQsoLog([msg({ to_me: true, snr: -7 })], "RA3XYZ");
    expect(result).toHaveLength(1);
    expect(result[0].direction).toBe("rx");
    expect(result[0].snr).toBe(-7);
  });

  it("classifies a message from the target to someone else as busy", () => {
    const result = buildQsoLog([msg({ to_me: false })], "RA3XYZ");
    expect(result).toHaveLength(1);
    expect(result[0].direction).toBe("busy");
  });

  it("ignores unrelated traffic", () => {
    const result = buildQsoLog([msg({ call_from: "JA1XYZ", to_me: false })], "RA3XYZ");
    expect(result).toEqual([]);
  });

  it("target match is case insensitive", () => {
    const result = buildQsoLog([msg({ call_from: "ra3xyz" })], "RA3XYZ");
    expect(result).toHaveLength(1);
  });

  it("excludes a CQ/DE/QRZ call from the target -- not busy, not rx (#254)", () => {
    const result = buildQsoLog([msg({ is_cq: true, to_me: false })], "RA3XYZ");
    expect(result).toEqual([]);
  });

  it("sorts entries ascending by time within one call (caller merges across sources)", () => {
    const result = buildQsoLog(
      [msg({ utc_ms: 2_000 }), msg({ utc_ms: 1_000 })],
      "RA3XYZ",
    );
    expect(result.map((e) => e.utcMs)).toEqual([2_000, 1_000]); // preserves input order; merge sorts
  });
});

describe("mergeRxLog", () => {
  const entry = (overrides: Partial<QsoLogEntry> = {}): QsoLogEntry => ({
    direction: "rx",
    utcMs: 1_000,
    text: "hello",
    snr: -10,
    ...overrides,
  });

  it("returns existing unchanged when incoming is empty", () => {
    const existing = [entry()];
    expect(mergeRxLog(existing, [])).toBe(existing);
  });

  it("adds a genuinely new row", () => {
    const existing = [entry({ utcMs: 1_000 })];
    const merged = mergeRxLog(existing, [entry({ utcMs: 2_000 })]);
    expect(merged).toHaveLength(2);
  });

  it("returns the same instance when the incoming row is an exact repeat", () => {
    const existing = [entry()];
    const merged = mergeRxLog(existing, [entry()]);
    expect(merged).toBe(existing);
  });

  it("replaces a stored row when a repeat arrives with an upgraded snr", () => {
    const existing = [entry({ snr: -20 })];
    const merged = mergeRxLog(existing, [entry({ snr: -5 })]);
    expect(merged).toHaveLength(1);
    expect(merged[0].snr).toBe(-5);
  });

  it("sorts the merged result ascending by time", () => {
    const existing = [entry({ utcMs: 3_000 })];
    const merged = mergeRxLog(existing, [entry({ utcMs: 1_000 }), entry({ utcMs: 2_000 })]);
    expect(merged.map((e) => e.utcMs)).toEqual([1_000, 2_000, 3_000]);
  });

  it("trims to the newest MAX_RX_LOG_ENTRIES", () => {
    const existing: QsoLogEntry[] = [];
    const incoming = Array.from({ length: 150 }, (_, i) => entry({ utcMs: i, text: `m${i}` }));
    const merged = mergeRxLog(existing, incoming);
    expect(merged).toHaveLength(100);
    expect(merged[0].utcMs).toBe(50);
    expect(merged[merged.length - 1].utcMs).toBe(149);
  });
});

describe("decideTxLog", () => {
  it("arms and logs on the rising edge when the message is already known", () => {
    const d = decideTxLog(false, true, false, "CQ K0XYZ EN37", true);
    expect(d.shouldLog).toBe(true);
    expect(d.pendingAfter).toBe(false);
  });

  it("does nothing while already transmitting with no new edge", () => {
    const d = decideTxLog(true, true, false, "CQ K0XYZ EN37", true);
    expect(d.shouldLog).toBe(false);
  });

  it("arms but waits when the message isn't populated yet", () => {
    const d = decideTxLog(false, true, false, "", true);
    expect(d.shouldLog).toBe(false);
    expect(d.pendingAfter).toBe(true);
  });

  it("logs on a later tick once the pending message arrives", () => {
    const d = decideTxLog(true, true, true, "CQ K0XYZ EN37", true);
    expect(d.shouldLog).toBe(true);
    expect(d.pendingAfter).toBe(false);
  });

  it("drops a pending log once transmit ends unfulfilled", () => {
    const d = decideTxLog(true, false, true, "", true);
    expect(d.shouldLog).toBe(false);
    expect(d.pendingAfter).toBe(false);
  });

  it("never logs without an active run (CQ or QSO)", () => {
    const d = decideTxLog(false, true, false, "CQ K0XYZ EN37", false);
    expect(d.shouldLog).toBe(false);
  });

  it("logs a repeated identical message as its own fresh row on a new rising edge", () => {
    const first = decideTxLog(false, true, false, "RR73", true);
    expect(first.shouldLog).toBe(true);
    // Transmit ends, then begins again with the same text -- a fresh edge.
    const ended = decideTxLog(true, false, false, "RR73", true);
    expect(ended.shouldLog).toBe(false);
    const second = decideTxLog(false, true, false, "RR73", true);
    expect(second.shouldLog).toBe(true);
  });
});

describe("formatTxLogText", () => {
  it("prefixes the message with the rounded TX frequency", () => {
    expect(formatTxLogText("CQ AI5II EM71", 1713.4)).toBe("(1713Hz) CQ AI5II EM71");
  });
});

describe("qsoHeaderText", () => {
  it("shows Calling CQ with no target", () => {
    expect(qsoHeaderText(false, null, true)).toBe("Calling CQ");
  });

  it("shows QSOing with <call> while transmitting to a target", () => {
    expect(qsoHeaderText(true, "K1ABC", true)).toBe("QSOing with K1ABC");
  });

  it("shows Waiting for <call> while idle mid-QSO", () => {
    expect(qsoHeaderText(true, "K1ABC", false)).toBe("Waiting for K1ABC");
  });
});

describe("qsoLogKey", () => {
  it("differs for the same text at different times", () => {
    const a: QsoLogEntry = { direction: "tx", utcMs: 1, text: "RR73", snr: null };
    const b: QsoLogEntry = { direction: "tx", utcMs: 2, text: "RR73", snr: null };
    expect(qsoLogKey(a)).not.toBe(qsoLogKey(b));
  });
});
