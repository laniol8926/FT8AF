/**
 * Station-marker SNR tinting for the Map screen. Ported from Android's
 * `MarkerStyle.kt`'s `snrTint`. Scoped to literally this -- the rest of
 * `markerStyleFor`'s shape variety (isCQ/isToMe marker shapes) and
 * zoom-dependent label visibility are a separate, not-yet-requested
 * roadmap item.
 */

/** Extracts the r/g/b channels from a `#rrggbb` hex color string. */
function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const n = parseInt(hex.replace("#", ""), 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

/**
 * Maps a decode's SNR (dB) onto an alpha gradient over the base color,
 * roughly spanning the real FT8 decode range [-24, +10] dB. Floored at
 * 0.78 so even the weakest decode stays visibly opaque, not nearly
 * invisible -- matches Android's `snrTint` exactly.
 */
export function snrTint(baseHex: string, snr: number): string {
  const t = Math.min(1, Math.max(0, (snr + 24) / 34));
  const alpha = 0.78 + 0.22 * t;
  const { r, g, b } = hexToRgb(baseHex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
