import { describe, expect, it } from "vitest";
import { wpxPrefixOf } from "./wpxPrefix";

// Mirrors WpxPrefixTest.java case-for-case, so the TS and Rust/Java ports
// agree on every example.
describe("wpxPrefixOf", () => {
  it("simple calls", () => {
    expect(wpxPrefixOf("W1AW")).toBe("W1");
    expect(wpxPrefixOf("K5XYZ")).toBe("K5");
    expect(wpxPrefixOf("VE3ABC")).toBe("VE3");
    expect(wpxPrefixOf("EA8XYZ")).toBe("EA8");
    expect(wpxPrefixOf("PY2AA")).toBe("PY2");
    expect(wpxPrefixOf("KH6ABC")).toBe("KH6");
  });

  it("leading digit prefixes", () => {
    expect(wpxPrefixOf("9A1AA")).toBe("9A1");
    expect(wpxPrefixOf("4X4AAA")).toBe("4X4");
    expect(wpxPrefixOf("3DA0RS")).toBe("3DA0");
  });

  it("case and whitespace insensitive", () => {
    expect(wpxPrefixOf("  ve3abc  ")).toBe("VE3");
  });

  it("no-numeral call gets zero", () => {
    expect(wpxPrefixOf("RAEM")).toBe("RA0");
  });

  it("portable number replaces prefix digit", () => {
    expect(wpxPrefixOf("W1AW/4")).toBe("W4");
    expect(wpxPrefixOf("VE3ABC/7")).toBe("VE7");
    expect(wpxPrefixOf("PY2AA/0")).toBe("PY0");
  });

  it("portable number, digit-leading prefix", () => {
    expect(wpxPrefixOf("9A1AA/7")).toBe("9A7");
    expect(wpxPrefixOf("4X4AAA/1")).toBe("4X1");
    expect(wpxPrefixOf("3DA0RS/7")).toBe("3DA7");
  });

  it("portable number, no-numeral call", () => {
    expect(wpxPrefixOf("RAEM/4")).toBe("RA4");
  });

  it("portable number on a non-callsign token stays null", () => {
    expect(wpxPrefixOf("W1/7")).toBeNull();
    expect(wpxPrefixOf("FN42/7")).toBeNull();
  });

  it("portable prefix: designator wins", () => {
    expect(wpxPrefixOf("DL/W1AW")).toBe("DL0");
    expect(wpxPrefixOf("PJ4/K1ABC")).toBe("PJ4");
    expect(wpxPrefixOf("PA3/G4XYZ")).toBe("PA3");
    expect(wpxPrefixOf("W1AW/KH6")).toBe("KH6");
  });

  it("operational suffixes ignored", () => {
    expect(wpxPrefixOf("G4XYZ/P")).toBe("G4");
    expect(wpxPrefixOf("W1AW/M")).toBe("W1");
    expect(wpxPrefixOf("VK2ABC/QRP")).toBe("VK2");
    expect(wpxPrefixOf("K1ABC/MM")).toBe("K1");
    expect(wpxPrefixOf("W1AW/4/QRP")).toBe("W4");
  });

  it("non-callsign tokens return null", () => {
    expect(wpxPrefixOf(null)).toBeNull();
    expect(wpxPrefixOf("")).toBeNull();
    expect(wpxPrefixOf("   ")).toBeNull();
    expect(wpxPrefixOf("73")).toBeNull();
    expect(wpxPrefixOf("RR73")).toBeNull();
    expect(wpxPrefixOf("599")).toBeNull();
    expect(wpxPrefixOf("<...>")).toBeNull();
    expect(wpxPrefixOf("R-12")).toBeNull();
    expect(wpxPrefixOf("FN42")).toBeNull();
    expect(wpxPrefixOf("W1")).toBeNull();
  });
});
