import { describe, expect, test } from "vitest";
import {
  parsePerBandLevels,
  recordPerBandLevel,
  resolvePerBandLevel,
  serializePerBandLevels,
} from "./perBandLevels";

describe("parsePerBandLevels", () => {
  test("parses a well-formed map", () => {
    expect(parsePerBandLevels("40m=80,20m=60")).toEqual({ "40m": 80, "20m": 60 });
  });

  test("empty/missing input yields an empty map", () => {
    expect(parsePerBandLevels("")).toEqual({});
    expect(parsePerBandLevels(null)).toEqual({});
    expect(parsePerBandLevels(undefined)).toEqual({});
  });

  test("skips malformed entries rather than throwing", () => {
    expect(parsePerBandLevels("40m=80,garbage,20m=not_a_number,=60,10m=50")).toEqual({
      "40m": 80,
      "10m": 50,
    });
  });
});

describe("serializePerBandLevels", () => {
  test("round-trips through parse", () => {
    const levels = { "40m": 80, "20m": 60 };
    expect(parsePerBandLevels(serializePerBandLevels(levels))).toEqual(levels);
  });

  test("empty map serializes to an empty string", () => {
    expect(serializePerBandLevels({})).toBe("");
  });
});

describe("recordPerBandLevel", () => {
  test("disabled feature records nothing", () => {
    expect(recordPerBandLevel(false, "40m", 80, {}, 100)).toBeNull();
  });

  test("no band (unresolved) records nothing", () => {
    expect(recordPerBandLevel(true, null, 80, {}, 100)).toBeNull();
  });

  test("records a new band", () => {
    expect(recordPerBandLevel(true, "40m", 80, {}, 100)).toEqual({ "40m": 80 });
  });

  test("unchanged value records nothing", () => {
    expect(recordPerBandLevel(true, "40m", 80, { "40m": 80 }, 100)).toBeNull();
  });

  test("clamps to the given max and to zero", () => {
    expect(recordPerBandLevel(true, "40m", 999, {}, 100)).toEqual({ "40m": 100 });
    expect(recordPerBandLevel(true, "40m", -5, {}, 100)).toEqual({ "40m": 0 });
  });

  test("preserves other bands already in the map", () => {
    expect(recordPerBandLevel(true, "20m", 60, { "40m": 80 }, 100)).toEqual({
      "40m": 80,
      "20m": 60,
    });
  });
});

describe("resolvePerBandLevel", () => {
  test("disabled feature restores nothing", () => {
    expect(resolvePerBandLevel(false, "40m", { "40m": 80 }, 50)).toBeNull();
  });

  test("no band (unresolved) restores nothing", () => {
    expect(resolvePerBandLevel(true, null, { "40m": 80 }, 50)).toBeNull();
  });

  test("no saved value for this band restores nothing (keeps current)", () => {
    expect(resolvePerBandLevel(true, "20m", { "40m": 80 }, 50)).toBeNull();
  });

  test("saved value matching current level restores nothing", () => {
    expect(resolvePerBandLevel(true, "40m", { "40m": 80 }, 80)).toBeNull();
  });

  test("restores a saved value that differs from current", () => {
    expect(resolvePerBandLevel(true, "40m", { "40m": 80 }, 50)).toBe(80);
  });
});
