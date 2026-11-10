import { describe, expect, test } from "vitest";
import { EquirectViewport } from "./mapViewport";

// Ported from Android's MapViewportTest.kt.

describe("EquirectViewport", () => {
  test("cover portrait canvas fills height and overflows width", () => {
    const vp = new EquirectViewport(1080, 2000);
    expect(vp.worldPxH).toBeCloseTo(2000, 2);
    expect(vp.worldPxW).toBeGreaterThan(1080);
    expect(vp.worldPxW).toBeCloseTo(4000, 2);
  });

  test("cover landscape canvas fills width", () => {
    const vp = new EquirectViewport(2000, 600);
    expect(vp.worldPxW).toBeCloseTo(2000, 2);
    expect(vp.worldPxH).toBeGreaterThan(600);
    expect(vp.worldPxH).toBeCloseTo(1000, 2);
  });

  test("fit letterboxes instead of filling", () => {
    const vp = new EquirectViewport(1080, 2000, 1, 0, 0, "fit");
    expect(vp.worldPxW).toBeCloseTo(1080, 2);
    expect(vp.worldPxH).toBeLessThan(2000);
  });

  test("project: center of world is canvas center at scale 1, no pan", () => {
    const vp = new EquirectViewport(1000, 800);
    const p = vp.project({ x: 0, y: 0 });
    expect(p.x).toBeCloseTo(500, 3);
    expect(p.y).toBeCloseTo(400, 3);
  });

  test("project: applies pan offset", () => {
    const vp = new EquirectViewport(1000, 800, 1, 50, -30);
    const p = vp.project({ x: 0, y: 0 });
    expect(p.x).toBeCloseTo(550, 3);
    expect(p.y).toBeCloseTo(370, 3);
  });

  test("project: right edge of world is half world width right", () => {
    const vp = new EquirectViewport(1080, 2000);
    const center = vp.project({ x: 0, y: 0 });
    const east = vp.project({ x: 1, y: 0 }); // lon = +180
    expect(east.x - center.x).toBeCloseTo(vp.worldPxW / 2, 2);
  });

  test("clampPan: overflow axis clamps to half overflow", () => {
    const vp = new EquirectViewport(1080, 2000);
    const maxX = (vp.worldPxW - 1080) / 2;
    const clamped = vp.clampPan(99999, 0);
    expect(clamped.x).toBeCloseTo(maxX, 2);
  });

  test("clampPan: underflow axis locks to zero", () => {
    const vp = new EquirectViewport(1080, 2000);
    expect(vp.maxPanY()).toBeCloseTo(0, 2);
    const clamped = vp.clampPan(0, 9999);
    expect(clamped.y).toBeCloseTo(0, 2);
  });

  test("clampPan: scaled up allows more pan", () => {
    const base = new EquirectViewport(1080, 2000);
    const zoomed = new EquirectViewport(1080, 2000, 3);
    expect(zoomed.maxPanX()).toBeGreaterThan(base.maxPanX());
    expect(zoomed.maxPanY()).toBeGreaterThan(0);
  });
});
