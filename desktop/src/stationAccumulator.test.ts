import { expect, test } from "vitest";
import { buildAccumulatedStations } from "./stationAccumulator";
import type { SlotMessage } from "./gridTrackerSlot";

function msg(overrides: Partial<SlotMessage>): SlotMessage {
  return { callFrom: "", callTo: "", isCq: false, grid: "", utcMs: 0, ...overrides };
}

function resolver(grids: Record<string, string>) {
  return (call: string) => grids[call] ?? null;
}

test("keeps only the first (= latest, since messages are newest-first) occurrence per callsign", () => {
  const latest = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "FN42", utcMs: 2 });
  const older = msg({ callFrom: "K1ABC", callTo: "W1AW", isCq: false, grid: "FN42", utcMs: 1 });
  const stations = buildAccumulatedStations([latest, older], "N0CALL", resolver({}));
  expect(stations).toHaveLength(1);
  expect(stations[0].message).toBe(latest);
});

test("a callsign whose only message is directed (not CQ) still gets a marker", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "W1AW", isCq: false, grid: "FN42" });
  const stations = buildAccumulatedStations([m], "N0CALL", resolver({}));
  expect(stations).toHaveLength(1);
  expect(stations[0].isCq).toBe(false);
  expect(stations[0].callsign).toBe("K1ABC");
});

test("isToMe is true only when callTo matches the operator's own call", () => {
  const toMe = msg({ callFrom: "K1ABC", callTo: "N0CALL", isCq: false, grid: "FN42" });
  const notToMe = msg({ callFrom: "W1AW", callTo: "VE3XY", isCq: false, grid: "FN31" });
  const stations = buildAccumulatedStations([toMe, notToMe], "n0call", resolver({})); // case-insensitive
  expect(stations.find((s) => s.callsign === "K1ABC")?.isToMe).toBe(true);
  expect(stations.find((s) => s.callsign === "W1AW")?.isToMe).toBe(false);
});

test("a callsign with no resolvable grid (own message blank, resolver misses) is dropped", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "" });
  expect(buildAccumulatedStations([m], "N0CALL", resolver({}))).toEqual([]);
});

test("falls back to the resolver when the message itself carries no grid", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "W1AW", isCq: false, grid: "" });
  const stations = buildAccumulatedStations([m], "N0CALL", resolver({ K1ABC: "FN42" }));
  expect(stations[0].grid).toBe("FN42");
});

test("a message with no callFrom is skipped entirely", () => {
  const m = msg({ callFrom: "", callTo: "W1AW", isCq: false, grid: "FN42" });
  expect(buildAccumulatedStations([m], "N0CALL", resolver({}))).toEqual([]);
});
