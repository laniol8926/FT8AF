import { describe, expect, test } from "vitest";
import { AzimuthalViewport } from "./azimuthalViewport";

describe("AzimuthalViewport", () => {
  test("the operator's own position projects to the canvas center (before pan)", () => {
    const vp = new AzimuthalViewport(400, 400, 40.7, -74.0, 1, 0, 0);
    const p = vp.projectLatLon(40.7, -74.0);
    expect(p.x).toBeCloseTo(200, 1);
    expect(p.y).toBeCloseTo(200, 1);
  });

  test("clampPan ignores any requested delta at 1x zoom -- always centered", () => {
    const vp = new AzimuthalViewport(400, 400, 0, 0, 1, 0, 0);
    expect(vp.clampPan(50, -30)).toEqual({ x: 0, y: 0 });
    expect(vp.clampPan(1000, 1000)).toEqual({ x: 0, y: 0 });
  });

  test("clampPan ignores any requested delta at less than 1x zoom too", () => {
    const vp = new AzimuthalViewport(400, 400, 0, 0, 0.5, 0, 0);
    expect(vp.clampPan(50, -30)).toEqual({ x: 0, y: 0 });
  });

  test("clampPan allows panning once zoomed in past 1x, clamped to half the zoomed overflow", () => {
    const vp = new AzimuthalViewport(400, 400, 0, 0, 2, 0, 0);
    // max pan = canvasW*(scale-1)/2 = 400*1/2 = 200 on each axis.
    expect(vp.clampPan(50, -30)).toEqual({ x: 50, y: -30 });
    expect(vp.clampPan(500, -500)).toEqual({ x: 200, y: -200 });
  });

  test("panToCenter always resets to the origin -- centering is baked into the projection itself", () => {
    const vp = new AzimuthalViewport(400, 400, 40.7, -74.0, 3, 120, -80);
    expect(vp.panToCenter(51.5, -0.1)).toEqual({ x: 0, y: 0 });
  });

  test("pxPerLonDegree scales linearly with zoom", () => {
    const at1x = new AzimuthalViewport(400, 400, 0, 0, 1, 0, 0);
    const at2x = new AzimuthalViewport(400, 400, 0, 0, 2, 0, 0);
    expect(at2x.pxPerLonDegree).toBeCloseTo(at1x.pxPerLonDegree * 2, 6);
  });

  test("discRadiusPx reserves margin for the compass labels, not a bare half-canvas", () => {
    const vp = new AzimuthalViewport(400, 400, 0, 0, 1, 0, 0);
    expect(vp.discRadiusPx).toBeLessThan(200);
    expect(vp.discRadiusPx).toBeGreaterThan(150); // sane margin, not something huge
  });
});
