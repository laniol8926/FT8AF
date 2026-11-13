import { describe, expect, test } from "vitest";
import { filterEnabledBands, parseExcludedBands, serializeExcludedBands } from "./bandFilter";

const BANDS = [{ name: "160m" }, { name: "40m" }, { name: "20m" }, { name: "10m" }];

describe("parseExcludedBands", () => {
  test("empty/missing input yields an empty set", () => {
    expect(parseExcludedBands("")).toEqual(new Set());
    expect(parseExcludedBands(null)).toEqual(new Set());
    expect(parseExcludedBands(undefined)).toEqual(new Set());
  });

  test("parses a comma-separated list, trimming stray whitespace", () => {
    expect(parseExcludedBands("40m, 20m ,10m")).toEqual(new Set(["40m", "20m", "10m"]));
  });

  test("ignores blank entries from stray commas", () => {
    expect(parseExcludedBands("40m,,20m,")).toEqual(new Set(["40m", "20m"]));
  });
});

describe("serializeExcludedBands", () => {
  test("round-trips through parse", () => {
    const set = new Set(["40m", "20m"]);
    expect(parseExcludedBands(serializeExcludedBands(set))).toEqual(set);
  });

  test("empty set serializes to an empty string", () => {
    expect(serializeExcludedBands(new Set())).toBe("");
  });
});

describe("filterEnabledBands", () => {
  test("empty exclusion list passes everything through", () => {
    expect(filterEnabledBands(BANDS, "")).toEqual(BANDS);
    expect(filterEnabledBands(BANDS, null)).toEqual(BANDS);
  });

  test("removes an excluded band by name", () => {
    expect(filterEnabledBands(BANDS, "40m")).toEqual([
      { name: "160m" },
      { name: "20m" },
      { name: "10m" },
    ]);
  });

  test("removes multiple excluded bands", () => {
    expect(filterEnabledBands(BANDS, "160m,10m")).toEqual([{ name: "40m" }, { name: "20m" }]);
  });

  test("an unknown band name in the exclusion list matches nothing extra", () => {
    expect(filterEnabledBands(BANDS, "6m,40m")).toEqual([
      { name: "160m" },
      { name: "20m" },
      { name: "10m" },
    ]);
  });
});
