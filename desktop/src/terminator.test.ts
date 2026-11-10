import { describe, expect, test } from "vitest";
import {
  isNight,
  nightPolygonLatLon,
  normalizeLon,
  solarElevationDeg,
  SubsolarPoint,
  subsolarPoint,
  terminatorCurveLatLon,
  terminatorLatitude,
} from "./terminator";

// Ported from Android's TerminatorTest.kt.

function utc(year: number, month: number, day: number, hour: number, minute = 0): number {
  return Date.UTC(year, month, day, hour, minute, 0);
}

describe("subsolarPoint", () => {
  test("summer solstice is near max northern declination", () => {
    // ~June 21: the Sun is overhead near the Tropic of Cancer (+23.44°). Month is 0-indexed.
    const p = subsolarPoint(utc(2024, 5, 21, 12));
    expect(p.lat).toBeCloseTo(23.44, 0);
    expect(Math.abs(p.lat - 23.44)).toBeLessThan(0.6);
  });

  test("winter solstice is near max southern declination", () => {
    const p = subsolarPoint(utc(2024, 11, 21, 12));
    expect(Math.abs(p.lat - -23.44)).toBeLessThan(0.6);
  });

  test("equinox is near equator", () => {
    const p = subsolarPoint(utc(2024, 2, 20, 12));
    expect(Math.abs(p.lat)).toBeLessThan(1.0);
  });

  test("longitude tracks utc hour", () => {
    expect(Math.abs(subsolarPoint(utc(2024, 2, 20, 12)).lon)).toBeLessThan(2.0);
    expect(Math.abs(subsolarPoint(utc(2024, 2, 20, 6)).lon - 90)).toBeLessThan(2.0);
    expect(Math.abs(subsolarPoint(utc(2024, 2, 20, 18)).lon - -90)).toBeLessThan(2.0);
  });
});

test("normalizeLon wraps into range", () => {
  expect(normalizeLon(190)).toBeCloseTo(-170, 9);
  expect(normalizeLon(-190)).toBeCloseTo(170, 9);
  expect(normalizeLon(45)).toBeCloseTo(45, 9);
  expect(normalizeLon(540)).toBeCloseTo(180, 9);
});

describe("solar elevation / night", () => {
  test("is max at subsolar point, min at antipode", () => {
    const sub: SubsolarPoint = { lat: 10, lon: 40 };
    expect(solarElevationDeg(10, 40, sub)).toBeCloseTo(90, 5);
    expect(solarElevationDeg(-10, normalizeLon(40 + 180), sub)).toBeCloseTo(-90, 5);
  });

  test("subsolar is day, antipode is night", () => {
    const sub: SubsolarPoint = { lat: 0, lon: 0 };
    expect(isNight(0, 0, sub)).toBe(false);
    expect(isNight(0, 180, sub)).toBe(true);
  });
});

describe("terminator latitude", () => {
  test("lies on the zero-elevation boundary", () => {
    const sub = subsolarPoint(utc(2024, 5, 21, 9));
    for (let lon = -180; lon <= 180; lon += 30) {
      const lat = terminatorLatitude(lon, sub);
      expect(solarElevationDeg(lat, lon, sub)).toBeCloseTo(0, 5);
    }
  });

  test("survives equinox without blowing up", () => {
    const sub: SubsolarPoint = { lat: 0, lon: 0 };
    const lat = terminatorLatitude(45, sub);
    expect(Number.isFinite(lat)).toBe(true);
    expect(Math.abs(lat)).toBeLessThanOrEqual(90);
  });
});

describe("night polygon", () => {
  test("terminator curve spans full longitude range", () => {
    const curve = terminatorCurveLatLon({ lat: 20, lon: 0 }, 3.0);
    expect(curve.length % 2).toBe(0);
    expect(curve[0]).toBeCloseTo(-180, 3);
    expect(curve[curve.length - 2]).toBeCloseTo(180, 3);
  });

  test("closes to the dark pole", () => {
    const north = nightPolygonLatLon({ lat: 20, lon: 0 }, 3.0);
    expect(north[north.length - 1]).toBeCloseTo(-90, 3);
    expect(north[north.length - 3]).toBeCloseTo(-90, 3);

    const south = nightPolygonLatLon({ lat: -20, lon: 0 }, 3.0);
    expect(south[south.length - 1]).toBeCloseTo(90, 3);
    expect(south[south.length - 3]).toBeCloseTo(90, 3);
  });

  test("has four more coords than the curve", () => {
    const sub: SubsolarPoint = { lat: 15, lon: -30 };
    const curve = terminatorCurveLatLon(sub, 5.0);
    const ring = nightPolygonLatLon(sub, 5.0);
    expect(ring.length).toBe(curve.length + 4);
  });
});
