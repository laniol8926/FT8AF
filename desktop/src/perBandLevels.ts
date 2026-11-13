/**
 * Per-band RX input gain / TX output level memory for the Radio & Audio
 * settings. Ported from the Android fork's `PerBandOutputLevel.kt` (TX) and
 * `PerBandInputLevel.kt` (RX) -- both reuse the same pure parse/serialize/
 * resolve/record shape, parameterized only by direction's own config keys
 * and max value; this module is that one shared shape, parameterized by
 * desktop's own existing 0-100 gain scale instead of Android's 0-200 (the
 * two apps' slider ranges differ, so a saved percent isn't directly
 * interchangeable between them even though the config keys match).
 *
 * Config keys (exact match with Android, for a future settings import):
 * `perBandInputLevel`/`perBandOutputLevel` ("1"/"0" toggle),
 * `perBandInputLevels`/`perBandOutputLevels` (the serialized map below).
 *
 * Serialized map format: `"40m=80,20m=60"` -- band name (matching
 * `BandInfo.name`) = percent, comma-separated. Malformed entries are
 * skipped rather than thrown on, matching `AudioChannelSelect.parse`'s own
 * resilience philosophy (a corrupted settings import must never crash the
 * load path).
 */

export type PerBandLevels = Record<string, number>;

export function parsePerBandLevels(serialized: string | null | undefined): PerBandLevels {
  const out: PerBandLevels = {};
  if (!serialized) return out;
  for (const pair of serialized.split(",")) {
    const eq = pair.indexOf("=");
    if (eq <= 0) continue;
    const band = pair.slice(0, eq).trim();
    const level = parseInt(pair.slice(eq + 1).trim(), 10);
    if (!band || !Number.isFinite(level)) continue;
    out[band] = level;
  }
  return out;
}

export function serializePerBandLevels(levels: PerBandLevels): string {
  return Object.entries(levels)
    .map(([band, level]) => `${band}=${level}`)
    .join(",");
}

/**
 * Record `level` (0-`max`) as the saved level for `band`, returning the
 * updated map to persist, or `null` if there's nothing to do -- the feature
 * is off, `band` is unknown, or the value is unchanged (matches Android's
 * own `recordPerBandOutputLevel`, which skips a no-op write).
 */
export function recordPerBandLevel(
  enabled: boolean,
  band: string | null,
  level: number,
  current: PerBandLevels,
  max: number,
): PerBandLevels | null {
  if (!enabled || !band) return null;
  const clamped = Math.max(0, Math.min(max, Math.round(level)));
  if (current[band] === clamped) return null;
  return { ...current, [band]: clamped };
}

/**
 * The saved level for `band`, or `null` if there's nothing to restore --
 * the feature is off, no value was ever saved for this band, or the saved
 * value already matches `currentLevel` (pure lookup; the caller applies/
 * persists the result, matching Android's own
 * `restoredInputLevelForBand`/`applyRestoredInputLevelForBand` split).
 */
export function resolvePerBandLevel(
  enabled: boolean,
  band: string | null,
  levels: PerBandLevels,
  currentLevel: number,
): number | null {
  if (!enabled || !band) return null;
  const saved = levels[band];
  if (saved === undefined || saved === currentLevel) return null;
  return saved;
}
