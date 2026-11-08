// Extracts the CQ WPX prefix from a callsign, ported from the Android app's
// WpxPrefix.java (com.k1af.ft8af.callsign.WpxPrefix.of). Backs the live
// Decode list's "New Prefix" filter -- the any-band worked-prefix set itself
// comes from the backend (src-tauri/src/wpx_prefix.rs, a parallel port of
// the same algorithm applied over the SQLite log), so a decoded message's
// prefix and the worked set it's compared against always agree.
//
// Rules: a simple call keeps everything up to and including the last prefix
// numeral; a call with no numeral at all gets its first two letters plus a
// trailing "0"; CALL/n swaps the base call's own prefix numeral for n;
// pfx/CALL takes the shorter token as the designator; /P /M /MM /AM /QRP
// /QRPP are ignored. Anything that isn't a plausible callsign returns null
// rather than inventing a prefix.

export function wpxPrefixOf(raw: string | null | undefined): string | null {
  if (raw == null) return null;
  const call = raw.trim().toUpperCase();
  if (call.length === 0) return null;
  return call.includes("/") ? compound(call) : simple(call);
}

function simple(call: string): string | null {
  if (!isAlnum(call)) return null;
  let lastDigit = -1;
  let hasLetter = false;
  for (let i = 0; i < call.length; i++) {
    const c = call[i];
    if (c >= "0" && c <= "9") lastDigit = i;
    else hasLetter = true;
  }
  if (!hasLetter) return null; // all-digit token (signal report, "73", ...) -- not a call
  if (lastDigit < 0) {
    // No numeral at all (historic call like RAEM): first two letters + 0.
    return call.length >= 2 ? call.substring(0, 2) + "0" : null;
  }
  // A full callsign carries a letter suffix after its prefix numeral.
  if (lastDigit === call.length - 1) return null;
  const prefix = call.substring(0, lastDigit + 1);
  return containsLetter(prefix) ? prefix : null;
}

function compound(call: string): string | null {
  const parts = call.split("/").filter((p) => p.length > 0 && !isIgnoredSuffix(p));
  if (parts.length === 0) return null;
  if (parts.length === 1) return simple(parts[0]);

  // Portable number: CALL/n -> the base call's own prefix with its numeral
  // swapped for the new one.
  const numeric = parts.find((p) => isAllDigits(p) && p.length <= 2);
  if (numeric != null) {
    for (const p of parts) {
      if (!isAllDigits(p)) {
        const base = simple(p);
        return base == null ? null : withPortableNumber(base, numeric);
      }
    }
    return null;
  }

  // Portable prefix: pfx/CALL. The designator is conventionally the shorter token.
  const [a, b] = parts;
  const designator = a.length <= b.length ? a : b;
  if (!isAlnum(designator) || !containsLetter(designator)) return null;
  let lastDigit = -1;
  for (let i = 0; i < designator.length; i++) {
    if (designator[i] >= "0" && designator[i] <= "9") lastDigit = i;
  }
  if (lastDigit >= 0) return designator.substring(0, lastDigit + 1);
  // Designator without a numeral (bare prefix like "DL"): first two letters + 0.
  return designator.length >= 2 ? designator.substring(0, 2) + "0" : designator + "0";
}

function isIgnoredSuffix(s: string): boolean {
  return s === "P" || s === "M" || s === "MM" || s === "AM" || s === "QRP" || s === "QRPP";
}

function isAlnum(s: string): boolean {
  return /^[A-Z0-9]*$/.test(s);
}

function isAllDigits(s: string): boolean {
  return s.length > 0 && /^[0-9]+$/.test(s);
}

function containsLetter(s: string): boolean {
  return /[A-Z]/.test(s);
}

/** Swap the trailing numerals of an already-derived prefix for the portable
 *  number, e.g. ("9A1","7") -> "9A7", ("W1","4") -> "W4". */
function withPortableNumber(base: string, number: string): string | null {
  let end = base.length;
  while (end > 0 && base[end - 1] >= "0" && base[end - 1] <= "9") end--;
  const letters = base.substring(0, end);
  return letters.length === 0 ? null : letters + number;
}
