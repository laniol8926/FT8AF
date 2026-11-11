import { describe, it, expect } from "vitest";
import { formatDecodeDt, isDecodeDtNotable, formatDecodeFreq } from "./decodeRow";

describe("formatDecodeDt", () => {
  it("formats a positive value signed, one decimal", () => {
    expect(formatDecodeDt(0.23)).toBe("+0.2");
    expect(formatDecodeDt(1.0)).toBe("+1.0");
  });

  it("formats a negative value signed, one decimal", () => {
    expect(formatDecodeDt(-1.34)).toBe("-1.3");
  });

  it("a value that rounds to zero renders unsigned, never +0.0 or -0.0", () => {
    expect(formatDecodeDt(0)).toBe("0.0");
    expect(formatDecodeDt(0.02)).toBe("0.0");
    expect(formatDecodeDt(-0.02)).toBe("0.0");
    expect(formatDecodeDt(-0.001)).toBe("0.0");
  });

  it("a value just past the rounding floor keeps its sign", () => {
    expect(formatDecodeDt(0.06)).toBe("+0.1");
    expect(formatDecodeDt(-0.06)).toBe("-0.1");
  });

  it("exactly +/-0.05 rounds like Math.round (half toward +Infinity), same as Android's Math.round(Float)", () => {
    // Both Java's Math.round(float) and JS's Math.round round a tie up
    // toward +Infinity, not away from zero -- so -0.05*10 = -0.5 rounds to
    // 0, not -1, and renders unsigned, same asymmetry Android itself has.
    expect(formatDecodeDt(0.05)).toBe("+0.1");
    expect(formatDecodeDt(-0.05)).toBe("0.0");
  });

  it("NaN/Infinity render as --", () => {
    expect(formatDecodeDt(NaN)).toBe("--");
    expect(formatDecodeDt(Infinity)).toBe("--");
    expect(formatDecodeDt(-Infinity)).toBe("--");
  });
});

describe("isDecodeDtNotable", () => {
  it("is notable past the 1.0s CLOCK_SYNC_FAIR_SEC threshold", () => {
    expect(isDecodeDtNotable(1.01)).toBe(true);
    expect(isDecodeDtNotable(-1.01)).toBe(true);
  });

  it("is not notable at or under the threshold, or for non-finite input", () => {
    expect(isDecodeDtNotable(1.0)).toBe(false);
    expect(isDecodeDtNotable(0.5)).toBe(false);
    expect(isDecodeDtNotable(NaN)).toBe(false);
  });
});

describe("formatDecodeFreq", () => {
  it("zero-pads to 4 digits so columns line up regardless of digit count", () => {
    expect(formatDecodeFreq(523)).toBe("0523");
    expect(formatDecodeFreq(50)).toBe("0050");
    expect(formatDecodeFreq(1864)).toBe("1864");
    expect(formatDecodeFreq(7)).toBe("0007");
  });

  it("rounds to the nearest Hz before padding", () => {
    expect(formatDecodeFreq(523.6)).toBe("0524");
  });

  it("a 5-digit frequency is not truncated, just left un-padded", () => {
    expect(formatDecodeFreq(12345)).toBe("12345");
  });

  it("clamps a negative input to 0 (never occurs in a real decode)", () => {
    expect(formatDecodeFreq(-5)).toBe("0000");
  });
});
