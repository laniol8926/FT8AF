import { equirectProject } from "./mapProjection";

/**
 * How the equirectangular world is scaled into the canvas at zoom 1.0.
 * Ported from Android's `MapViewport.kt`.
 *
 * "cover" fills the whole canvas (the map overflows on whichever axis the
 * canvas is "too long" in, so there are never dead bands) -- used for the
 * full-screen map. "fit" letterboxes the map inside the canvas (kept for
 * completeness/tests, not used by MapScreen).
 */
export type FitMode = "cover" | "fit";

export interface Point {
  x: number;
  y: number;
}

/**
 * Maps the pure `equirectProject` output (normalized [-1,1] on each axis)
 * onto a concrete canvas, edge-to-edge, with a user zoom and pan offset.
 *
 * The world's intrinsic aspect is 2:1 (360° lon by 180° lat). `baseUniform`
 * is the world's full height in pixels at zoom 1, so the world is
 * `2*baseUniform` wide by `baseUniform` tall (before `userScale`) -- i.e. x
 * is scaled to twice as many pixels per normalized unit as y, which
 * preserves the 2:1 aspect. "cover" picks `max(canvasW/2, canvasH)` so the
 * world is at least as large as the canvas on both axes (no dead bands).
 */
export class EquirectViewport {
  readonly canvasW: number;
  readonly canvasH: number;
  readonly userScale: number;
  readonly panX: number;
  readonly panY: number;
  readonly fit: FitMode;

  readonly baseUniform: number;
  readonly worldPxW: number;
  readonly worldPxH: number;

  constructor(
    canvasW: number,
    canvasH: number,
    userScale = 1,
    panX = 0,
    panY = 0,
    fit: FitMode = "cover",
  ) {
    this.canvasW = canvasW;
    this.canvasH = canvasH;
    this.userScale = userScale;
    this.panX = panX;
    this.panY = panY;
    this.fit = fit;

    // cover: worldPxW >= canvasW (=> baseUniform >= canvasW/2) AND
    //        worldPxH >= canvasH (=> baseUniform >= canvasH). Take the max so both hold.
    // fit:   the opposite -- the smaller scale, so the whole world fits inside.
    const base =
      fit === "cover" ? Math.max(canvasW / 2, canvasH) : Math.min(canvasW / 2, canvasH);
    this.baseUniform = Math.max(base, 1);
    this.worldPxW = this.baseUniform * 2 * userScale;
    this.worldPxH = this.baseUniform * 1 * userScale;
  }

  /** Longitude pixels per degree at the current zoom -- see Android's doc
   *  comment on the same getter for why this (not raw userScale) drives the
   *  state-border fade. (No state-border layer ported yet -- kept for parity
   *  with the ported math and future use.) */
  get pxPerLonDegree(): number {
    return this.worldPxW / 360;
  }

  /** Project a normalized point (from `equirectProject`) to canvas pixels. */
  project(p: Point): Point {
    const cx = this.canvasW / 2;
    const cy = this.canvasH / 2;
    return {
      x: cx + p.x * (this.worldPxW / 2) + this.panX,
      y: cy + p.y * (this.worldPxH / 2) + this.panY,
    };
  }

  /** Convenience: project geographic lat/lon straight to canvas pixels. */
  projectLatLon(lat: number, lon: number): Point {
    return this.project(equirectProject(lat, lon));
  }

  /** Max pan on each axis = half the overflow of the (zoomed) world past the canvas, floored at 0. */
  maxPanX(): number {
    return Math.max((this.worldPxW - this.canvasW) / 2, 0);
  }
  maxPanY(): number {
    return Math.max((this.worldPxH - this.canvasH) / 2, 0);
  }

  /** Clamp a candidate pan so the map can't be dragged off into empty space.
   *  On an axis where the map is smaller than the canvas the allowed pan is
   *  0 (the map stays centered). */
  clampPan(px: number, py: number): Point {
    const mx = this.maxPanX();
    const my = this.maxPanY();
    return { x: Math.min(Math.max(px, -mx), mx), y: Math.min(Math.max(py, -my), my) };
  }

  /** The clamped pan that puts geographic lat/lon as close to the canvas
   *  centre as the clamp allows -- used by the "Me" recenter button. */
  panToCenter(lat: number, lon: number): Point {
    const p = equirectProject(lat, lon);
    return this.clampPan(-p.x * (this.worldPxW / 2), -p.y * (this.worldPxH / 2));
  }
}
