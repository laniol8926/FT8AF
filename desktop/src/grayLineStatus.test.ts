import { describe, expect, it } from "vitest";
import { formatSolarCountdown, grayLineDisplay, solarSnapshot } from "./grayLineStatus";

function utc(year: number, monthIndex: number, day: number, hour: number, minute = 0): number {
  return Date.UTC(year, monthIndex, day, hour, minute, 0);
}

describe("solarSnapshot", () => {
  it("is daytime at local solar noon", () => {
    const s = solarSnapshot(0, 0, utc(2024, 2, 20, 12));
    expect(s.isDay).toBe(true);
    expect(s.elevationDeg).toBeGreaterThan(60);
    expect(s.onGrayLine).toBe(false);
  });

  it("is night at local midnight", () => {
    const s = solarSnapshot(0, 0, utc(2024, 2, 20, 0));
    expect(s.isDay).toBe(false);
    expect(s.elevationDeg).toBeLessThan(-60);
    expect(s.onGrayLine).toBe(false);
  });

  it("the next event in daytime is sunset", () => {
    const s = solarSnapshot(0, 0, utc(2024, 2, 20, 12));
    expect(s.nextKind).toBe("sunset");
    expect(Math.abs(s.minutesToNext - 6 * 60)).toBeLessThanOrEqual(30);
  });

  it("the next event at night is sunrise", () => {
    const s = solarSnapshot(0, 0, utc(2024, 2, 20, 3));
    expect(s.nextKind).toBe("sunrise");
    expect(Math.abs(s.minutesToNext - 3 * 60)).toBeLessThanOrEqual(30);
  });

  it("flips to daytime after a sunrise crossing", () => {
    const beforeDawn = solarSnapshot(0, 0, utc(2024, 2, 20, 5));
    expect(beforeDawn.isDay).toBe(false);
    expect(beforeDawn.nextKind).toBe("sunrise");

    const afterDawn = solarSnapshot(0, 0, utc(2024, 2, 20, 7));
    expect(afterDawn.isDay).toBe(true);
  });

  it("passes through the gray-line band near sunrise", () => {
    let sawGrayLine = false;
    for (let minute = 0; minute < 120; minute += 4) {
      const s = solarSnapshot(0, 0, utc(2024, 2, 20, 5, minute % 60) + Math.floor(minute / 60) * 3600_000);
      if (s.onGrayLine) sawGrayLine = true;
    }
    expect(sawGrayLine).toBe(true);
  });

  it("gray-line width is configurable", () => {
    const base = utc(2024, 2, 20, 5, 30);
    let chosen = -1;
    for (let m = 0; m < 120; m++) {
      const t = base + m * 60_000;
      const elev = solarSnapshot(0, 0, t).elevationDeg;
      if (elev >= -5 && elev <= -3) {
        chosen = t;
        break;
      }
    }
    expect(chosen).toBeGreaterThan(0);
    expect(solarSnapshot(0, 0, chosen, 6).onGrayLine).toBe(true);
    expect(solarSnapshot(0, 0, chosen, 2).onGrayLine).toBe(false);
  });

  it("has no sunset within the scan during arctic summer (midnight sun)", () => {
    const s = solarSnapshot(89, 0, utc(2024, 5, 21, 12));
    expect(s.isDay).toBe(true);
    expect(s.nextKind).toBeNull();
    expect(s.minutesToNext).toBe(0);
  });

  it("has no sunrise within the scan during arctic winter (polar night)", () => {
    const s = solarSnapshot(89, 0, utc(2024, 11, 21, 12));
    expect(s.isDay).toBe(false);
    expect(s.nextKind).toBeNull();
  });
});

describe("formatSolarCountdown", () => {
  it("formats hours and minutes", () => {
    expect(formatSolarCountdown(80)).toBe("1h 20m");
    expect(formatSolarCountdown(45)).toBe("45m");
    expect(formatSolarCountdown(120)).toBe("2h");
    expect(formatSolarCountdown(60)).toBe("1h");
  });

  it("is empty for zero or negative (the 'now' flag)", () => {
    expect(formatSolarCountdown(0)).toBe("");
    expect(formatSolarCountdown(-5)).toBe("");
  });
});

describe("grayLineDisplay", () => {
  it("shows daylight + sunset in the daytime case", () => {
    const d = grayLineDisplay({ elevationDeg: 40, isDay: true, onGrayLine: false, nextKind: "sunset", minutesToNext: 80 });
    expect(d.phase).toBe("daylight");
    expect(d.detail).toBe("sunset");
    expect(d.countdown).toBe("1h 20m");
  });

  it("shows night + sunrise in the nighttime case", () => {
    const d = grayLineDisplay({ elevationDeg: -30, isDay: false, onGrayLine: false, nextKind: "sunrise", minutesToNext: 45 });
    expect(d.phase).toBe("night");
    expect(d.detail).toBe("sunrise");
    expect(d.countdown).toBe("45m");
  });

  it("has an empty countdown when the event is imminent", () => {
    const d = grayLineDisplay({ elevationDeg: 0.5, isDay: true, onGrayLine: true, nextKind: "sunset", minutesToNext: 0 });
    expect(d.detail).toBe("sunset");
    expect(d.countdown).toBe("");
  });

  it("gray-line overrides the day/night phase", () => {
    const d = grayLineDisplay({ elevationDeg: 2, isDay: true, onGrayLine: true, nextKind: "sunset", minutesToNext: 12 });
    expect(d.phase).toBe("gray_line");
    expect(d.detail).toBe("sunset");
  });

  it("shows midnight sun with no countdown", () => {
    const d = grayLineDisplay({ elevationDeg: 15, isDay: true, onGrayLine: false, nextKind: null, minutesToNext: 0 });
    expect(d.phase).toBe("daylight");
    expect(d.detail).toBe("midnight_sun");
    expect(d.countdown).toBe("");
  });

  it("shows polar night with no countdown", () => {
    const d = grayLineDisplay({ elevationDeg: -10, isDay: false, onGrayLine: false, nextKind: null, minutesToNext: 0 });
    expect(d.phase).toBe("night");
    expect(d.detail).toBe("polar_night");
    expect(d.countdown).toBe("");
  });
});
