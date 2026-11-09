import { describe, expect, it } from "vitest";
import { collapseByStation, nextSortMode, type DecodeSortMode } from "./decodeSort";
import type { UiMessage } from "./ipc";

function msg(over: Partial<UiMessage>): UiMessage {
  return {
    utc_ms: 0,
    call_to: "CQ",
    call_from: "K1ABC",
    extra: "",
    grid: "",
    snr: -10,
    freq_hz: 1500,
    time_sec: 0,
    text: "",
    is_cq: true,
    to_me: false,
    dxcc_prefix: null,
    cq_zone: null,
    state: null,
    ...over,
  };
}

describe("nextSortMode", () => {
  it("cycles time -> call -> snr -> dx -> time", () => {
    const order: DecodeSortMode[] = ["time", "call", "snr", "dx", "time"];
    for (let i = 0; i < order.length - 1; i++) {
      expect(nextSortMode(order[i])).toBe(order[i + 1]);
    }
  });
});

describe("collapseByStation", () => {
  it("keeps only the latest decode per callsign", () => {
    const messages = [
      msg({ call_from: "K1ABC", utc_ms: 1000, snr: -5 }),
      msg({ call_from: "K1ABC", utc_ms: 2000, snr: -10 }),
      msg({ call_from: "W1XYZ", utc_ms: 1500, snr: 0 }),
    ];
    const result = collapseByStation(messages, "time", "");
    expect(result).toHaveLength(2);
    expect(result.find((m) => m.call_from === "K1ABC")?.utc_ms).toBe(2000);
  });

  it("drops messages with no usable callsign", () => {
    const messages = [msg({ call_from: "  " }), msg({ call_from: "K1ABC" })];
    expect(collapseByStation(messages, "time", "")).toHaveLength(1);
  });

  it("sorts by time, newest first", () => {
    const messages = [
      msg({ call_from: "A", utc_ms: 1000 }),
      msg({ call_from: "B", utc_ms: 3000 }),
      msg({ call_from: "C", utc_ms: 2000 }),
    ];
    expect(collapseByStation(messages, "time", "").map((m) => m.call_from)).toEqual(["B", "C", "A"]);
  });

  it("sorts by callsign alphabetically", () => {
    const messages = [msg({ call_from: "W9ABC" }), msg({ call_from: "K1ABC" }), msg({ call_from: "N0XYZ" })];
    expect(collapseByStation(messages, "call", "").map((m) => m.call_from)).toEqual([
      "K1ABC", "N0XYZ", "W9ABC",
    ]);
  });

  it("sorts by SNR, strongest first", () => {
    const messages = [
      msg({ call_from: "A", snr: -15 }),
      msg({ call_from: "B", snr: 2 }),
      msg({ call_from: "C", snr: -5 }),
    ];
    expect(collapseByStation(messages, "snr", "").map((m) => m.call_from)).toEqual(["B", "C", "A"]);
  });

  it("sorts by distance, farthest first, with gridless stations sinking to the bottom", () => {
    const messages = [
      msg({ call_from: "NEAR", grid: "FN42" }), // ~same area as FN43
      msg({ call_from: "FAR", grid: "QF22" }), // Australia-ish
      msg({ call_from: "NOGRID", grid: "" }),
    ];
    const order = collapseByStation(messages, "dx", "FN43").map((m) => m.call_from);
    expect(order).toEqual(["FAR", "NEAR", "NOGRID"]);
  });

  it("falls back to the bottom for every station when the operator has no grid set", () => {
    const messages = [msg({ call_from: "A", grid: "FN42" }), msg({ call_from: "B", grid: "QF22" })];
    // No myGrid -- distance is unknown for everyone, so original relative
    // order among equals (both -Infinity) is preserved by a stable sort.
    expect(collapseByStation(messages, "dx", "").map((m) => m.call_from)).toEqual(["A", "B"]);
  });
});
