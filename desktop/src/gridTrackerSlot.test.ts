import { expect, test } from "vitest";
import { buildGridTrackerSlot, currentSlotMessages, isStationVisible, type SlotMessage } from "./gridTrackerSlot";

function msg(over: Partial<SlotMessage>): SlotMessage {
  return { callFrom: "K1ABC", callTo: "", isCq: false, grid: "", utcMs: 0, ...over };
}

const CYCLE_MS = 15_000;

// --- currentSlotMessages ---------------------------------------------------

test("currentSlotMessages keeps only the newest slot", () => {
  const older = msg({ callFrom: "A", utcMs: 0 });
  const newer1 = msg({ callFrom: "B", utcMs: 20_000 });
  const newer2 = msg({ callFrom: "C", utcMs: 29_000 }); // same slot as newer1 (floor/15000 == 1)
  const result = currentSlotMessages([older, newer1, newer2], CYCLE_MS);
  expect(result).toEqual([newer1, newer2]);
});

test("currentSlotMessages preserves input order within the slot", () => {
  const a = msg({ callFrom: "A", utcMs: 15_500 });
  const b = msg({ callFrom: "B", utcMs: 15_100 });
  expect(currentSlotMessages([a, b], CYCLE_MS)).toEqual([a, b]);
});

test("currentSlotMessages returns empty for an empty list", () => {
  expect(currentSlotMessages([], CYCLE_MS)).toEqual([]);
});

// --- buildGridTrackerSlot ---------------------------------------------------

function resolver(table: Record<string, string>) {
  return (call: string): string | null => table[call] ?? null;
}

test("a CQ message gets a marker, no line", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "W9ZZZ", resolver({}));
  expect(slot.markers).toEqual([{ callsign: "K1ABC", grid: "FN42", message: m }]);
  expect(slot.lines).toEqual([]);
});

test("CQ marker falls back to the resolver when the message has no grid", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "" });
  const slot = buildGridTrackerSlot([m], "W9ZZZ", resolver({ K1ABC: "FN42" }));
  expect(slot.markers).toEqual([{ callsign: "K1ABC", grid: "FN42", message: m }]);
});

test("a CQ message with no resolvable grid is dropped entirely", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "" });
  const slot = buildGridTrackerSlot([m], "W9ZZZ", resolver({}));
  expect(slot.markers).toEqual([]);
});

test("a directed exchange gets a line between the two real stations", () => {
  // TX3-style report, no grid in the message itself -- both ends via the resolver.
  const m = msg({ callFrom: "K1ABC", callTo: "W9XYZ", isCq: false, grid: "" });
  const slot = buildGridTrackerSlot(
    [m], "N0CALL", resolver({ K1ABC: "FN42", W9XYZ: "EN52" }),
  );
  expect(slot.markers).toEqual([]);
  expect(slot.lines).toEqual([
    { fromCall: "K1ABC", fromGrid: "FN42", toCall: "W9XYZ", toGrid: "EN52", involvesMyCall: false, message: m },
  ]);
});

test("call_to's grid always comes from the resolver, never the message", () => {
  // A grid-reply message: the decoder's grid field belongs to call_from only.
  const m = msg({ callFrom: "K1ABC", callTo: "W9XYZ", isCq: false, grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "N0CALL", resolver({ W9XYZ: "EN52" }));
  expect(slot.lines).toEqual([
    { fromCall: "K1ABC", fromGrid: "FN42", toCall: "W9XYZ", toGrid: "EN52", involvesMyCall: false, message: m },
  ]);
});

test("a directed exchange with an unresolvable end draws nothing", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "W9XYZ", isCq: false, grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "N0CALL", resolver({})); // W9XYZ unresolvable
  expect(slot.lines).toEqual([]);
});

test("involvesMyCall is true when either end is my callsign", () => {
  const toMe = msg({ callFrom: "K1ABC", callTo: "N0CALL", grid: "FN42" });
  const fromMe = msg({ callFrom: "N0CALL", callTo: "K1ABC", grid: "FN42" });
  const table = resolver({ K1ABC: "FN42", N0CALL: "EN52" });
  expect(buildGridTrackerSlot([toMe], "N0CALL", table).lines[0].involvesMyCall).toBe(true);
  expect(buildGridTrackerSlot([fromMe], "N0CALL", table).lines[0].involvesMyCall).toBe(true);
});

test("involvesMyCall is false for an overheard third-party QSO", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "W9XYZ", grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "N0CALL", resolver({ W9XYZ: "EN52" }));
  expect(slot.lines[0].involvesMyCall).toBe(false);
});

test("my callsign comparison is case-insensitive", () => {
  const m = msg({ callFrom: "k1abc", callTo: "n0call", grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "N0CALL", resolver({ N0CALL: "EN52" }));
  expect(slot.lines[0].involvesMyCall).toBe(true);
  expect(slot.lines[0].toCall).toBe("N0CALL");
});

test("duplicate (callFrom, callTo) pairs are deduped, first occurrence wins", () => {
  const first = msg({ callFrom: "K1ABC", callTo: "W9XYZ", grid: "FN42" });
  const second = msg({ callFrom: "K1ABC", callTo: "W9XYZ", grid: "EM10" }); // same pair, different grid
  const slot = buildGridTrackerSlot([first, second], "N0CALL", resolver({ W9XYZ: "EN52" }));
  expect(slot.lines).toHaveLength(1);
  expect(slot.lines[0].message).toBe(first);
});

test("two CQ decodes from the same station dedupe to one marker", () => {
  const first = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "FN42" });
  const second = msg({ callFrom: "K1ABC", callTo: "CQ", isCq: true, grid: "FN42" });
  const slot = buildGridTrackerSlot([first, second], "N0CALL", resolver({}));
  expect(slot.markers).toHaveLength(1);
});

test("a directed message with no callTo (unresolved) is dropped", () => {
  const m = msg({ callFrom: "K1ABC", callTo: "", isCq: false, grid: "FN42" });
  const slot = buildGridTrackerSlot([m], "N0CALL", resolver({}));
  expect(slot.lines).toEqual([]);
  expect(slot.markers).toEqual([]);
});

test("isStationVisible partitions by worked status, not additively", () => {
  // Unworked: controlled only by decodesOn.
  expect(isStationVisible(false, true, false)).toBe(true);
  expect(isStationVisible(false, false, true)).toBe(false);
  // Worked: controlled only by workedOn.
  expect(isStationVisible(true, true, false)).toBe(false);
  expect(isStationVisible(true, false, true)).toBe(true);
});
