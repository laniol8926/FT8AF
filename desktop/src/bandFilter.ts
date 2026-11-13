/**
 * "Enabled bands" filter for the Radio & Audio settings -- hides bands the
 * operator never uses from the band picker. Matches Android's own
 * `excludedBands` config key and value shape exactly (comma-separated band
 * name strings, e.g. `"40m,20m"`, matching `BandInfo.name` -- not `dial_hz`).
 */

export interface NamedBand {
  name: string;
}

/** Parse a persisted `excludedBands` CSV into a set of band names, ignoring
 *  blank entries (tolerant of stray commas/whitespace from manual editing or
 *  a settings import). */
export function parseExcludedBands(csv: string | null | undefined): Set<string> {
  const out = new Set<string>();
  if (!csv) return out;
  for (const name of csv.split(",")) {
    const trimmed = name.trim();
    if (trimmed) out.add(trimmed);
  }
  return out;
}

export function serializeExcludedBands(excluded: Set<string>): string {
  return Array.from(excluded).join(",");
}

/** The subset of `bands` not named in `excludedCsv` -- an unknown/malformed
 *  entry in `excludedCsv` just never matches anything, rather than throwing. */
export function filterEnabledBands<B extends NamedBand>(bands: B[], excludedCsv: string | null | undefined): B[] {
  const excluded = parseExcludedBands(excludedCsv);
  if (excluded.size === 0) return bands;
  return bands.filter((b) => !excluded.has(b.name));
}
