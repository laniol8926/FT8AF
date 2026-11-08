import { describe, expect, it } from "vitest";
import { formatPowerWatts, formatSwrRatio, swrBand } from "./meters";

describe("formatSwrRatio", () => {
  it("shows one decimal", () => {
    expect(formatSwrRatio(1.3)).toBe("1.3");
    expect(formatSwrRatio(1.0)).toBe("1.0");
  });
  it("caps very high readings at 9.9+", () => {
    expect(formatSwrRatio(12)).toBe("9.9+");
    expect(formatSwrRatio(9.95)).toBe("9.9+");
    expect(formatSwrRatio(9.94)).toBe("9.9");
  });
});

describe("formatPowerWatts", () => {
  it("is not rounded away for fractional watts", () => {
    expect(formatPowerWatts(4.5)).toBe("4.5W");
    expect(formatPowerWatts(0.8)).toBe("0.8W");
    expect(formatPowerWatts(12.4)).toBe("12.4W");
  });
  it("trims a whole-number reading to no decimal", () => {
    expect(formatPowerWatts(5)).toBe("5W");
  });
});

describe("swrBand", () => {
  it("splits at 2.0 and 3.0", () => {
    expect(swrBand(1.0)).toBe("good");
    expect(swrBand(2.0)).toBe("good");
    expect(swrBand(2.1)).toBe("caution");
    expect(swrBand(3.0)).toBe("caution");
    expect(swrBand(3.1)).toBe("bad");
  });
});
