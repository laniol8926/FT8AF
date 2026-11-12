import { describe, expect, test } from "vitest";
import { summarizeSignalReach } from "./signalReach";

describe("summarizeSignalReach", () => {
  test("empty input summarizes to null", () => {
    expect(summarizeSignalReach([], 0, 0)).toBeNull();
  });

  test("a blank-callsign-only input summarizes to null", () => {
    expect(summarizeSignalReach([{ callsign: "  ", lat: 0, lon: 0, snr: 5 }], 0, 0)).toBeNull();
  });

  test("picks the furthest (from the operator) and strongest-SNR receivers", () => {
    const reach = summarizeSignalReach(
      [
        { callsign: "W1AW", lat: 41.7, lon: -72.7, snr: -5 }, // near
        { callsign: "VK3XYZ", lat: -37.8, lon: 145.0, snr: -15 }, // far
        { callsign: "G4ABC", lat: 51.5, lon: -0.1, snr: 3 }, // strongest SNR
      ],
      40.7,
      -74.0, // operator near New York
    );
    expect(reach).not.toBeNull();
    expect(reach!.receivers).toBe(3);
    expect(reach!.furthestCall).toBe("VK3XYZ");
    expect(reach!.furthestKm).toBeGreaterThan(10_000);
    expect(reach!.strongestCall).toBe("G4ABC");
    expect(reach!.strongestSnr).toBe(3);
  });

  test("dedupes by callsign, keeping the higher-SNR report", () => {
    const reach = summarizeSignalReach(
      [
        { callsign: "w1aw", lat: 41.7, lon: -72.7, snr: -20 },
        { callsign: "W1AW", lat: 41.7, lon: -72.7, snr: -2 },
      ],
      40.7,
      -74.0,
    );
    expect(reach!.receivers).toBe(1);
    expect(reach!.strongestSnr).toBe(-2);
  });

  test("unknown operator position leaves furthest unset but still tracks strongest SNR", () => {
    const reach = summarizeSignalReach([{ callsign: "W1AW", lat: 41.7, lon: -72.7, snr: 7 }], null, null);
    expect(reach!.furthestKm).toBe(0);
    expect(reach!.furthestCall).toBe("");
    expect(reach!.strongestSnr).toBe(7);
    expect(reach!.strongestCall).toBe("W1AW");
  });
});
