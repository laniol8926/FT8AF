import { describe, expect, test } from "vitest";
import { EquirectViewport, minScaleForLatitude, scalePanForZoom } from "./mapViewport";

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

describe("minScaleForLatitude", () => {
  test("near the equator, only the safety margin pushes scale past 1 -- no real headroom is needed", () => {
    expect(minScaleForLatitude(423, 423, 0)).toBeCloseTo(1.2, 6);
  });

  test("caps at scale 20 for a near-polar latitude", () => {
    expect(minScaleForLatitude(423, 423, 89.9)).toBeCloseTo(20, 6);
  });

  test("floors at scale 1 when the panel is already wide enough at scale 1 to have headroom", () => {
    // A 2:1+ canvas has worldPxH < canvasH is impossible, but worldPxH can
    // already exceed canvasH at scale 1 if baseUniform > canvasH (bound by
    // canvasW/2 instead) -- no zoom-in is needed in that case.
    expect(minScaleForLatitude(300, 600, 31.5)).toBeCloseTo(1, 6);
  });

  test("regression: the returned scale actually gives EquirectViewport enough vertical headroom to reach the target latitude -- not just a plausible-looking number", () => {
    // Reproduces the real bug: a near-square panel (canvasH == baseUniform,
    // i.e. canvasW < 2*canvasH) where scale 1 has exactly zero vertical pan
    // room (maxPanY() === 0), so panToCenter's Y offset used to clamp away
    // to 0 regardless of latitude -- e.g. Alabama (lat ~31.5) stayed pinned
    // to the equator at canvas center.
    const canvasW = 441;
    const canvasH = 423;
    const lat = 31.5;
    const lon = -85;
    const base = new EquirectViewport(canvasW, canvasH, 1, 0, 0).baseUniform;
    expect(base).toBeCloseTo(canvasH, 6); // confirms this canvas hits the zero-headroom case at scale 1

    const scale = minScaleForLatitude(canvasH, base, lat);
    expect(scale).toBeGreaterThan(1);

    const vp = new EquirectViewport(canvasW, canvasH, scale, 0, 0);
    const pan = vp.panToCenter(lat, lon);
    const vpAtPan = new EquirectViewport(canvasW, canvasH, scale, pan.x, pan.y);
    const p = vpAtPan.projectLatLon(lat, lon);
    // The recentered point must land at the canvas center -- not clamped
    // away to some other latitude, as it was before this fix.
    expect(p.x).toBeCloseTo(canvasW / 2, 1);
    expect(p.y).toBeCloseTo(canvasH / 2, 1);
    expect(pan.y).not.toBe(0);
  });
});

describe("scalePanForZoom", () => {
  test("zooming in scales pan up by the same ratio as the scale change", () => {
    const p = scalePanForZoom({ x: 100, y: -40 }, 2, 4);
    expect(p.x).toBeCloseTo(200, 6);
    expect(p.y).toBeCloseTo(-80, 6);
  });

  test("zooming out scales pan down by the same ratio", () => {
    const p = scalePanForZoom({ x: 200, y: -80 }, 4, 2);
    expect(p.x).toBeCloseTo(100, 6);
    expect(p.y).toBeCloseTo(-40, 6);
  });

  test("no scale change leaves pan unchanged", () => {
    const p = scalePanForZoom({ x: 123, y: 456 }, 5, 5);
    expect(p).toEqual({ x: 123, y: 456 });
  });

  test("the point at canvas center stays at canvas center across a zoom step", () => {
    // Regression for the real bug: an unscaled pan made zooming drift back
    // toward lon 0/lat 0 instead of staying on the point you were viewing.
    const oldScale = 2;
    const newScale = 3;
    const oldPan = { x: 37, y: -19 };
    const oldVp = new EquirectViewport(1000, 800, oldScale, oldPan.x, oldPan.y);

    // Whatever lat/lon currently sits at the canvas center under oldVp --
    // derived by inverting `project`, not guessed.
    const nx = -oldPan.x / (oldVp.worldPxW / 2);
    const ny = -oldPan.y / (oldVp.worldPxH / 2);
    const centerLatLon = { lat: -ny * 90, lon: nx * 180 };
    const projectedAtOld = oldVp.projectLatLon(centerLatLon.lat, centerLatLon.lon);
    expect(projectedAtOld.x).toBeCloseTo(500, 6);
    expect(projectedAtOld.y).toBeCloseTo(400, 6);

    const scaledPan = scalePanForZoom(oldPan, oldScale, newScale);
    const newVp = new EquirectViewport(1000, 800, newScale, scaledPan.x, scaledPan.y);
    const projectedAtNew = newVp.projectLatLon(centerLatLon.lat, centerLatLon.lon);
    expect(projectedAtNew.x).toBeCloseTo(500, 6);
    expect(projectedAtNew.y).toBeCloseTo(400, 6);
  });
});
