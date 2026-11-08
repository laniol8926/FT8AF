//! Extracts the CQ WPX prefix from a callsign, ported from the Android app's
//! `WpxPrefix.java` (`com.k1af.ft8af.callsign.WpxPrefix`, `of(String)`).
//! Backs the "New Prefix" filter's any-band worked-prefix aggregation
//! ([`crate::db::Db::worked_prefixes`]); the per-message "is this a new
//! prefix" check lives in the frontend's `src/wpxPrefix.ts`, a parallel port
//! of the same algorithm (same reason the grid/DXCC logic is split: the
//! worked-set aggregation runs over the SQLite log in Rust, the live decode
//! list runs in the browser).
//!
//! Rules (see the Java doc comment for the full WPX spec this follows):
//! simple call keeps everything up to and including the last prefix numeral;
//! a call with no numeral at all gets its first two letters plus a trailing
//! `0`; `CALL/n` swaps the base call's own prefix numeral for `n`; `pfx/CALL`
//! takes the shorter token as the designator; `/P /M /MM /AM /QRP /QRPP` are
//! ignored. Anything that isn't a plausible callsign returns `None` rather
//! than inventing a prefix.

/// The WPX prefix in upper case, or `None` if none can be determined.
pub fn of(raw: &str) -> Option<String> {
    let call = raw.trim().to_uppercase();
    if call.is_empty() {
        return None;
    }
    if call.contains('/') {
        compound(&call)
    } else {
        simple(&call)
    }
}

fn simple(call: &str) -> Option<String> {
    if !is_alnum(call) {
        return None;
    }
    let chars: Vec<char> = call.chars().collect();
    let mut last_digit: Option<usize> = None;
    let mut has_letter = false;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            last_digit = Some(i);
        } else {
            has_letter = true;
        }
    }
    if !has_letter {
        return None; // all-digit token (signal report, "73", ...) -- not a call
    }
    let last_digit = match last_digit {
        None => {
            // No numeral at all (historic call like RAEM): first two letters + 0.
            return if chars.len() >= 2 {
                Some(format!("{}{}0", chars[0], chars[1]))
            } else {
                None
            };
        }
        Some(d) => d,
    };
    // A full callsign carries a letter suffix after its prefix numeral.
    if last_digit == chars.len() - 1 {
        return None;
    }
    let prefix: String = chars[..=last_digit].iter().collect();
    if contains_letter(&prefix) {
        Some(prefix)
    } else {
        None
    }
}

fn compound(call: &str) -> Option<String> {
    let parts: Vec<&str> = call
        .split('/')
        .filter(|p| !p.is_empty() && !is_ignored_suffix(p))
        .collect();
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        return simple(parts[0]);
    }

    // Portable number: CALL/n -> the base call's own prefix with its numeral
    // swapped for the new one.
    let numeric = parts
        .iter()
        .find(|p| is_all_digits(p) && p.len() <= 2)
        .copied();
    if let Some(numeric) = numeric {
        for p in &parts {
            if !is_all_digits(p) {
                let base = simple(p)?;
                return with_portable_number(&base, numeric);
            }
        }
        return None;
    }

    // Portable prefix: pfx/CALL. The designator is conventionally the
    // shorter token.
    let (a, b) = (parts[0], parts[1]);
    let designator = if a.len() <= b.len() { a } else { b };
    if !is_alnum(designator) || !contains_letter(designator) {
        return None;
    }
    let chars: Vec<char> = designator.chars().collect();
    let last_digit = chars.iter().rposition(|c| c.is_ascii_digit());
    match last_digit {
        Some(d) => Some(chars[..=d].iter().collect()),
        None => {
            // Designator without a numeral (bare prefix like "DL"): first two letters + 0.
            if chars.len() >= 2 {
                Some(format!("{}{}0", chars[0], chars[1]))
            } else {
                Some(format!("{designator}0"))
            }
        }
    }
}

fn is_ignored_suffix(s: &str) -> bool {
    matches!(s, "P" | "M" | "MM" | "AM" | "QRP" | "QRPP")
}

fn is_alnum(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn contains_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_uppercase())
}

/// Swap the trailing numerals of an already-derived prefix for the portable
/// number, e.g. `("9A1","7")` -> `"9A7"`, `("W1","4")` -> `"W4"`.
fn with_portable_number(base: &str, number: &str) -> Option<String> {
    let end = base
        .chars()
        .rev()
        .position(|c| !c.is_ascii_digit())
        .map(|trailing_digits| base.len() - trailing_digits)
        .unwrap_or(0);
    let letters = &base[..end];
    if letters.is_empty() {
        None
    } else {
        Some(format!("{letters}{number}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(call: &str) -> Option<String> {
        of(call)
    }

    #[test]
    fn simple_calls() {
        assert_eq!(p("W1AW").as_deref(), Some("W1"));
        assert_eq!(p("K5XYZ").as_deref(), Some("K5"));
        assert_eq!(p("VE3ABC").as_deref(), Some("VE3"));
        assert_eq!(p("EA8XYZ").as_deref(), Some("EA8"));
        assert_eq!(p("PY2AA").as_deref(), Some("PY2"));
        assert_eq!(p("KH6ABC").as_deref(), Some("KH6"));
    }

    #[test]
    fn leading_digit_prefixes() {
        assert_eq!(p("9A1AA").as_deref(), Some("9A1"));
        assert_eq!(p("4X4AAA").as_deref(), Some("4X4"));
        assert_eq!(p("3DA0RS").as_deref(), Some("3DA0"));
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        assert_eq!(p("  ve3abc  ").as_deref(), Some("VE3"));
    }

    #[test]
    fn no_numeral_call_gets_zero() {
        assert_eq!(p("RAEM").as_deref(), Some("RA0"));
    }

    #[test]
    fn portable_number_replaces_prefix_digit() {
        assert_eq!(p("W1AW/4").as_deref(), Some("W4"));
        assert_eq!(p("VE3ABC/7").as_deref(), Some("VE7"));
        assert_eq!(p("PY2AA/0").as_deref(), Some("PY0"));
    }

    #[test]
    fn portable_number_digit_leading_prefix() {
        assert_eq!(p("9A1AA/7").as_deref(), Some("9A7"));
        assert_eq!(p("4X4AAA/1").as_deref(), Some("4X1"));
        assert_eq!(p("3DA0RS/7").as_deref(), Some("3DA7"));
    }

    #[test]
    fn portable_number_no_numeral_call() {
        assert_eq!(p("RAEM/4").as_deref(), Some("RA4"));
    }

    #[test]
    fn portable_number_on_non_callsign_token_stays_none() {
        assert_eq!(p("W1/7"), None);
        assert_eq!(p("FN42/7"), None);
    }

    #[test]
    fn portable_prefix_designator_wins() {
        assert_eq!(p("DL/W1AW").as_deref(), Some("DL0"));
        assert_eq!(p("PJ4/K1ABC").as_deref(), Some("PJ4"));
        assert_eq!(p("PA3/G4XYZ").as_deref(), Some("PA3"));
        assert_eq!(p("W1AW/KH6").as_deref(), Some("KH6"));
    }

    #[test]
    fn operational_suffixes_ignored() {
        assert_eq!(p("G4XYZ/P").as_deref(), Some("G4"));
        assert_eq!(p("W1AW/M").as_deref(), Some("W1"));
        assert_eq!(p("VK2ABC/QRP").as_deref(), Some("VK2"));
        assert_eq!(p("K1ABC/MM").as_deref(), Some("K1"));
        assert_eq!(p("W1AW/4/QRP").as_deref(), Some("W4"));
    }

    #[test]
    fn non_callsign_tokens_return_none() {
        assert_eq!(p(""), None);
        assert_eq!(p("   "), None);
        assert_eq!(p("73"), None);
        assert_eq!(p("RR73"), None);
        assert_eq!(p("599"), None);
        assert_eq!(p("<...>"), None);
        assert_eq!(p("R-12"), None);
        assert_eq!(p("FN42"), None);
        assert_eq!(p("W1"), None);
    }
}
