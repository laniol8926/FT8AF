/**
 * Pure geometry for the QSO Sheet's path map, ported from Android's
 * `QsoPathProjection.kt`. Frames the operator's grid and the remote
 * station's grid into a fixed-size canvas using an equirectangular
 * projection with a uniform degrees-per-pixel scale (so the inset isn't
 * stretched), auto-zoomed to the two endpoints with a minimum span so two
 * nearby stations don't zoom in to an uninformative street-level view.
 */

/** Minimum lat/lon span (degrees) the QSO path map will frame. */
export const QSO_MAP_MIN_SPAN_DEG = 25.0;

/** Fractional padding added around the framed endpoints, per side. */
export const QSO_MAP_PAD_FRAC = 0.3;

/** Shift `lon` by +-360 so it lies within 180 degrees of `refLon` -- the
 *  short way around the antemeridian. */
export function normalizeLon(refLon: number, lon: number): number {
  let l = lon;
  if (l - refLon > 180) l -= 360;
  else if (l - refLon < -180) l += 360;
  return l;
}

export class QsoPathProjection {
  readonly normalizedTheirLon: number;
  readonly scale: number;
  private readonly centerLon: number;
  private readonly centerLat: number;
  private readonly width: number;
  private readonly height: number;

  constructor(
    myLat: number,
    myLon: number,
    theirLat: number,
    theirLon: number,
    width: number,
    height: number,
  ) {
    this.width = width;
    this.height = height;
    this.normalizedTheirLon = normalizeLon(myLon, theirLon);
    const tLon = this.normalizedTheirLon;

    this.centerLon = (Math.min(myLon, tLon) + Math.max(myLon, tLon)) / 2;
    this.centerLat = (Math.min(myLat, theirLat) + Math.max(myLat, theirLat)) / 2;

    const padFactor = 1 + 2 * QSO_MAP_PAD_FRAC;
    const spanLon = Math.max(Math.max(myLon, tLon) - Math.min(myLon, tLon), QSO_MAP_MIN_SPAN_DEG) * padFactor;
    const spanLat = Math.max(Math.max(myLat, theirLat) - Math.min(myLat, theirLat), QSO_MAP_MIN_SPAN_DEG) * padFactor;
    this.scale = Math.min(width / spanLon, height / spanLat);
  }

  projectX(lon: number): number {
    return this.width / 2 + (lon - this.centerLon) * this.scale;
  }

  projectY(lat: number): number {
    return this.height / 2 - (lat - this.centerLat) * this.scale;
  }
}
