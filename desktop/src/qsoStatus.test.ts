import { describe, expect, it } from "vitest";
import { resolveQsoStatus, extractCqModifier, type QsoStatusFlags } from "./qsoStatus";

// All-false baseline; each test flips only the flags it cares about, so a
// test reads as "this flag alone resolves to this status" (or doesn't,
// for the off-by-default tiers).
const base: QsoStatusFlags = {
  isCq: false,
  isToMe: false,
  isPota: false,
  isSota: false,
  isNewDxcc: false,
  isNewZone: false,
  isNewState: false,
  isNewGrid: false,
  isNewPrefix: false,
  isWorkedOtherBand: false,
  isWorked: false,
};

describe("resolveQsoStatus", () => {
  it("returns null for an unremarkable message", () => {
    expect(resolveQsoStatus(base)).toBeNull();
  });

  it("calling me outranks everything else", () => {
    expect(
      resolveQsoStatus({ ...base, isToMe: true, isNewDxcc: true, isPota: true, isCq: true })
    ).toBe("pending");
  });

  it("POTA outranks new DXCC/zone", () => {
    expect(resolveQsoStatus({ ...base, isPota: true, isNewDxcc: true, isNewZone: true })).toBe(
      "pota"
    );
  });

  it("SOTA outranks new DXCC/zone and is never gated by a highlight toggle", () => {
    expect(resolveQsoStatus({ ...base, isSota: true, isNewDxcc: true, isNewZone: true })).toBe(
      "sota"
    );
  });

  it("new DXCC outranks new zone", () => {
    expect(resolveQsoStatus({ ...base, isNewDxcc: true, isNewZone: true })).toBe("new_dxcc");
  });

  it("new zone outranks new band and worked", () => {
    expect(
      resolveQsoStatus({ ...base, isNewZone: true, isWorkedOtherBand: true, isWorked: true })
    ).toBe("new_zone");
  });

  it("new band outranks worked", () => {
    expect(resolveQsoStatus({ ...base, isWorkedOtherBand: true, isWorked: true })).toBe(
      "new_band"
    );
  });

  it("worked outranks a plain CQ", () => {
    expect(resolveQsoStatus({ ...base, isWorked: true, isCq: true })).toBe("worked");
  });

  it("falls back to plain CQ", () => {
    expect(resolveQsoStatus({ ...base, isCq: true })).toBe("cq");
  });

  it("new state, new grid, and new prefix are off by default (Android's own default)", () => {
    expect(resolveQsoStatus({ ...base, isNewState: true })).toBeNull();
    expect(resolveQsoStatus({ ...base, isNewGrid: true })).toBeNull();
    expect(resolveQsoStatus({ ...base, isNewPrefix: true })).toBeNull();
    // ... but still fall through correctly to whatever's next/applicable.
    expect(resolveQsoStatus({ ...base, isNewState: true, isCq: true })).toBe("cq");
  });
});

describe("extractCqModifier", () => {
  it("extracts a 1-4 letter modifier right after CQ", () => {
    expect(extractCqModifier(true, "CQ SOTA K1ABC FN42")).toBe("SOTA");
    expect(extractCqModifier(true, "CQ DX K1ABC FN42")).toBe("DX");
    expect(extractCqModifier(true, "CQ POTA K1ABC FN42")).toBe("POTA");
  });

  it("extracts a 3-digit numeric modifier", () => {
    expect(extractCqModifier(true, "CQ 123 K1ABC FN42")).toBe("123");
  });

  it("is case-insensitive", () => {
    expect(extractCqModifier(true, "cq sota k1abc fn42")).toBe("SOTA");
  });

  it("returns null for a plain CQ with a real callsign next", () => {
    // Every real callsign mixes letters and digits, which neither pattern
    // (pure 3-digit, or pure 1-4 letters) accepts.
    expect(extractCqModifier(true, "CQ K1ABC FN42")).toBeNull();
    expect(extractCqModifier(true, "CQ W1AW FN42")).toBeNull();
  });

  it("returns null when not a CQ at all", () => {
    expect(extractCqModifier(false, "CQ SOTA K1ABC FN42")).toBeNull();
    expect(extractCqModifier(true, "K1ABC W9XYZ RR73")).toBeNull();
  });

  it("returns null for a token too long to be a modifier", () => {
    expect(extractCqModifier(true, "CQ ABCDE K1ABC FN42")).toBeNull();
    expect(extractCqModifier(true, "CQ 1234 K1ABC FN42")).toBeNull();
  });
});
