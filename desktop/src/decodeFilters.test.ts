import { describe, it, expect } from "vitest";
import {
  parseWorkedMode,
  parseWorkedScope,
  isWorkedByScope,
  shouldHideWorked,
  parseCallList,
  isBlocked,
  matchesWatchlist,
} from "./decodeFilters";

describe("parseWorkedMode", () => {
  it("accepts the three known values", () => {
    expect(parseWorkedMode("highlight")).toBe("highlight");
    expect(parseWorkedMode("ignore")).toBe("ignore");
    expect(parseWorkedMode("hide")).toBe("hide");
  });
  it("defaults to highlight for anything else", () => {
    expect(parseWorkedMode(null)).toBe("highlight");
    expect(parseWorkedMode(undefined)).toBe("highlight");
    expect(parseWorkedMode("garbage")).toBe("highlight");
  });
});

describe("parseWorkedScope", () => {
  it("accepts the four known values", () => {
    expect(parseWorkedScope("on_band")).toBe("on_band");
    expect(parseWorkedScope("before")).toBe("before");
    expect(parseWorkedScope("today")).toBe("today");
    expect(parseWorkedScope("from_list")).toBe("from_list");
  });
  it("defaults to on_band for anything else", () => {
    expect(parseWorkedScope(null)).toBe("on_band");
    expect(parseWorkedScope("garbage")).toBe("on_band");
  });
});

describe("isWorkedByScope", () => {
  const flags = { onBand: false, anyBand: true, today: false, inList: false };
  it("picks the field matching the scope", () => {
    expect(isWorkedByScope("on_band", flags)).toBe(false);
    expect(isWorkedByScope("before", flags)).toBe(true);
    expect(isWorkedByScope("today", flags)).toBe(false);
    expect(isWorkedByScope("from_list", flags)).toBe(false);
  });
});

describe("shouldHideWorked", () => {
  it("hides only in hide mode for a worked station", () => {
    expect(shouldHideWorked("hide", true)).toBe(true);
    expect(shouldHideWorked("hide", false)).toBe(false);
    expect(shouldHideWorked("highlight", true)).toBe(false);
    expect(shouldHideWorked("ignore", true)).toBe(false);
  });
});

describe("parseCallList", () => {
  it("splits on commas and whitespace, uppercases, and drops blanks", () => {
    expect(parseCallList("w1aw, k1abc   3y0,  ")).toEqual(["W1AW", "K1ABC", "3Y0"]);
  });
  it("returns an empty array for a blank string", () => {
    expect(parseCallList("   ")).toEqual([]);
  });
});

describe("isBlocked", () => {
  const list = { exact: ["W1AW"], prefixes: ["RA"], keywords: ["POTA", "/P"] };
  it("matches an exact whole-call", () => {
    expect(isBlocked("W1AW", "CQ W1AW FN42", list)).toBe(true);
    expect(isBlocked("W1AWX", "CQ W1AWX FN42", list)).toBe(false);
  });
  it("matches a callsign prefix", () => {
    expect(isBlocked("RA3ABC", "CQ RA3ABC KO85", list)).toBe(true);
  });
  it("matches a keyword in the callsign", () => {
    expect(isBlocked("K1ABC/P", "CQ K1ABC/P FN42", list)).toBe(true);
  });
  it("matches a keyword in the message text", () => {
    expect(isBlocked("K1ABC", "CQ POTA K1ABC FN42", list)).toBe(true);
  });
  it("lets an unmatched call through", () => {
    expect(isBlocked("N0CALL", "CQ N0CALL EN00", list)).toBe(false);
  });
});

describe("matchesWatchlist", () => {
  it("matches by prefix so a DXpedition prefix catches every suffix", () => {
    const watch = ["3Y0", "W1AW"];
    expect(matchesWatchlist("3Y0J", watch)).toBe(true);
    expect(matchesWatchlist("W1AW", watch)).toBe(true);
    expect(matchesWatchlist("K1ABC", watch)).toBe(false);
  });
});
