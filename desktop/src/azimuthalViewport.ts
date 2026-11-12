import { azProject } from "./azimuthalProjection";
import type { Point, Viewport } from "./mapViewport";

/** Space reserved outside the disc for the compass-bearing labels (drawn
 *  `discRadiusPx + 12`px outward from center) so they land inside the
 *  canvas instead of being clipped by the panel's own edge. */
const COMPASS_LABEL_MARGIN_PX = 28;

/**
 * Screen-space viewport for the Azimuthal (radar) projection -- the
 * operator is always at the disc's center before pan, matching Android's
 * own azimuthal pan/zoom rules exactly (ported from `MapScreen.kt`'s
 * inline `clampPan` branch for `MapViewMode.AZIMUTHAL`): panning is only
 * allowed once zoomed in past 1x; at 1x the view is always centered on the
 * operator, and "Me" recentering is simply resetting zoom/pan, since
 * centering is already baked into the projection itself via `opLat`/`opLon`,
 * not a pannable offset.
 */
export class AzimuthalViewport implements Viewport {
  readonly canvasW: number;
  readonly canvasH: number;
  readonly opLat: number;
  readonly opLon: number;
  readonly userScale: number;
  readonly panX: number;
  readonly panY: number;

  /** The disc's radius in pixels at zoom 1 -- half the smaller canvas
   *  dimension, so the whole disc always fits without letterboxing. Reserves
   *  `COMPASS_LABEL_MARGIN_PX` so the 8 compass-bearing labels (drawn
   *  `discRadiusPx + 12`px outward from center, as fixed UI chrome that
   *  doesn't scale with zoom) stay inside the canvas instead of being
   *  clipped by the panel's own edge -- confirmed live this needed fixing:
   *  the N/E/S/W cardinal labels were cut off at the panel boundary before
   *  this margin existed, even though the underlying direction math (and
   *  the always-visible NE/SE/SW/NW intercardinals) was already correct. */
  readonly discRadiusPx: number;

  constructor(
    canvasW: number,
    canvasH: number,
    opLat: number,
    opLon: number,
    userScale = 1,
    panX = 0,
    panY = 0,
  ) {
    this.canvasW = canvasW;
    this.canvasH = canvasH;
    this.opLat = opLat;
    this.opLon = opLon;
    this.userScale = userScale;
    this.panX = panX;
    this.panY = panY;
    this.discRadiusPx = Math.max(Math.min(canvasW, canvasH) / 2 - COMPASS_LABEL_MARGIN_PX, 1);
  }

  get pxPerLonDegree(): number {
    return (this.discRadiusPx * this.userScale) / 180;
  }

  projectLatLon(lat: number, lon: number): Point {
    const p = azProject(this.opLat, this.opLon, lat, lon);
    const r = this.discRadiusPx * this.userScale;
    return {
      x: this.canvasW / 2 + p.x * r + this.panX,
      y: this.canvasH / 2 + p.y * r + this.panY,
    };
  }

  /** Pan is only meaningful once zoomed in -- at 1x (or less) the view is
   *  always centered on the operator, any requested delta is ignored. */
  clampPan(px: number, py: number): Point {
    if (this.userScale <= 1) return { x: 0, y: 0 };
    const mx = (this.canvasW * (this.userScale - 1)) / 2;
    const my = (this.canvasH * (this.userScale - 1)) / 2;
    return { x: Math.min(Math.max(px, -mx), mx), y: Math.min(Math.max(py, -my), my) };
  }

  /** Centering is already baked into the projection via `opLat`/`opLon` --
   *  "recenter" just means resetting pan to zero, same as Android's own
   *  azimuthal "Me" button (`mapScale=1, panX=panY=0`). */
  panToCenter(_lat: number, _lon: number): Point {
    return { x: 0, y: 0 };
  }
}
