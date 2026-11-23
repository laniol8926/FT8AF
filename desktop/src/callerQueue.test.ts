import { describe, expect, it } from "vitest";
import { callerSnrLabel, orderCallerQueue } from "./callerQueue";
import type { QueuedCaller } from "./ipc";

function caller(callsign: string, snr: number): QueuedCaller {
  return { callsign, grid: "FN42", snr, freq_hz: 1500, extra: "", queued_at_ms: 0, last_heard_ms: 0 };
}

describe("orderCallerQueue", () => {
  it("keeps first-heard order when strongestFirst is off", () => {
    const queue = [caller("WEAK", -10), caller("STRONG", 10)];
    expect(orderCallerQueue(queue, false).map((c) => c.callsign)).toEqual(["WEAK", "STRONG"]);
  });

  it("sorts by SNR descending when strongestFirst is on", () => {
    const queue = [caller("WEAK", -10), caller("STRONG", 10), caller("MID", 0)];
    expect(orderCallerQueue(queue, true).map((c) => c.callsign)).toEqual(["STRONG", "MID", "WEAK"]);
  });

  it("keeps first-heard order as a stable tie-break for equal SNR", () => {
    const queue = [caller("FIRST", 5), caller("SECOND", 5)];
    expect(orderCallerQueue(queue, true).map((c) => c.callsign)).toEqual(["FIRST", "SECOND"]);
  });

  it("does not mutate the input array", () => {
    const queue = [caller("WEAK", -10), caller("STRONG", 10)];
    orderCallerQueue(queue, true);
    expect(queue.map((c) => c.callsign)).toEqual(["WEAK", "STRONG"]);
  });
});

describe("callerSnrLabel", () => {
  it("signs a positive SNR", () => {
    expect(callerSnrLabel(5)).toBe("+5");
  });

  it("renders zero as +0", () => {
    expect(callerSnrLabel(0)).toBe("+0");
  });

  it("signs a negative SNR without a double minus", () => {
    expect(callerSnrLabel(-12)).toBe("-12");
  });
});
