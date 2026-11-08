import { describe, expect, it } from "vitest";
import { resolveQsoStatus, type QsoStatusFlags } from "./qsoStatus";

// All-false baseline; each test flips only the flags it cares about, so a
// test reads as "this flag alone resolves to this status" (or doesn't,
// for the off-by-default tiers).
const base: QsoStatusFlags = {
  isCq: false,
  isToMe: false,
  isPota: false,
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
