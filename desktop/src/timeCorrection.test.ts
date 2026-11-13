import { describe, it, expect } from "vitest";
import { clampCorrectionMs, stepCorrectionMs, formatOffsetMs, TIME_CORRECTION_MIN_MS, TIME_CORRECTION_MAX_MS } from "./timeCorrection";

describe("clampCorrectionMs", () => {
  it("passes through an in-range value unchanged", () => {
    expect(clampCorrectionMs(500)).toBe(500);
  });
  it("clamps above the max", () => {
    expect(clampCorrectionMs(9000)).toBe(TIME_CORRECTION_MAX_MS);
  });
  it("clamps below the min", () => {
    expect(clampCorrectionMs(-9000)).toBe(TIME_CORRECTION_MIN_MS);
  });
});

describe("stepCorrectionMs", () => {
  it("nudges by the delta", () => {
    expect(stepCorrectionMs(0, 100)).toBe(100);
    expect(stepCorrectionMs(100, -500)).toBe(-400);
  });
  it("clamps the result", () => {
    expect(stepCorrectionMs(TIME_CORRECTION_MAX_MS, 500)).toBe(TIME_CORRECTION_MAX_MS);
    expect(stepCorrectionMs(TIME_CORRECTION_MIN_MS, -500)).toBe(TIME_CORRECTION_MIN_MS);
  });
});

describe("formatOffsetMs", () => {
  it("formats zero with no sign", () => {
    expect(formatOffsetMs(0)).toBe("0.0s");
  });
  it("formats a positive offset with a leading +", () => {
    expect(formatOffsetMs(600)).toBe("+0.6s");
  });
  it("formats a negative offset with a leading -", () => {
    expect(formatOffsetMs(-300)).toBe("-0.3s");
  });
  it("rounds to one decimal place", () => {
    expect(formatOffsetMs(1234)).toBe("+1.2s");
  });
});
