//! US state lookup, ported from the Android app's `GeneralVariables`
//! (`stateForGrid`, `workedStateFor`, `US_STATE_CODES`, `US_CALL`,
//! `baseCallsign`). Backs the Decode list's New State filter and its
//! worked-state-per-band/mode aggregation.
//!
//! [`state_for_grid`] resolves a 4-character Maidenhead grid to a USPS state
//! code via a bundled table (`resources/us_grid_states.json`, the same
//! `assets/us_grid_states.json` Android ships); non-US grids return `None`
//! naturally since the table is US-only. [`worked_state_for`] additionally
//! prefers a logged ADIF `STATE` when it's a plausible US state *and* the
//! QSO is identifiably a US station -- see its doc comment -- since a grid
//! square alone can't distinguish some neighbouring states (Android's
//! comment: "DE, RI share grid squares with their neighbours").

use once_cell::sync::Lazy;
use std::collections::HashMap;

const US_GRID_STATES_JSON: &str = include_str!("../resources/us_grid_states.json");

static GRID_STATE_MAP: Lazy<HashMap<String, String>> = Lazy::new(|| {
    serde_json::from_str::<HashMap<String, String>>(US_GRID_STATES_JSON)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k.to_uppercase(), v))
        .collect()
});

/// The 50 US states plus DC (USPS codes), for validating a logged ADIF STATE.
const US_STATE_CODES: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "DC", "FL", "GA", "HI", "ID", "IL", "IN", "IA",
    "KS", "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ", "NM",
    "NY", "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA",
    "WV", "WI", "WY",
];

/// Resolve a 4+ character Maidenhead grid to a US state code, or `None` if
/// it isn't a US grid (the table is US-only) or is too short to look up.
pub fn state_for_grid(grid: &str) -> Option<String> {
    if grid.len() < 4 {
        return None;
    }
    let grid4 = grid[..4].to_uppercase();
    GRID_STATE_MAP.get(&grid4).cloned()
}

/// US amateur callsign prefixes: K, N, W, AA-AL (incl. KH/KL/KP), matching
/// Android's `US_CALL` regex `^(?:[KNW]|A[A-L])[0-9A-Z]*[0-9][A-Z]+$`.
fn is_us_call(call: &str) -> bool {
    let bytes = call.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let rest_start = if bytes[0] == b'K' || bytes[0] == b'N' || bytes[0] == b'W' {
        1
    } else if bytes[0] == b'A' && bytes.len() > 1 && (b'A'..=b'L').contains(&bytes[1]) {
        2
    } else {
        return false;
    };
    if rest_start >= bytes.len() {
        return false;
    }
    let rest = &bytes[rest_start..];
    // Must be alnum (uppercase) from here on, contain at least one digit,
    // and end in a letter -- i.e. there is no digit after the last one.
    let mut last_digit: Option<usize> = None;
    for (i, &c) in rest.iter().enumerate() {
        if c.is_ascii_digit() {
            last_digit = Some(i);
        } else if !c.is_ascii_uppercase() {
            return false;
        }
    }
    match last_digit {
        Some(d) => d != rest.len() - 1, // at least one letter after the last digit
        None => false,
    }
}

/// The home callsign part of a portable/prefixed call
/// (`"W1AW/4"` -> `"W1AW"`, `"KH6/W1AW"` -> `"W1AW"`): the longest `/`-part
/// that contains a digit and ends in a letter, i.e. looks like a whole
/// callsign rather than a portable prefix/suffix.
fn base_callsign(callsign: &str) -> String {
    let c = callsign.trim().to_uppercase().replace('<', "").replace('>', "");
    let mut best = String::new();
    for part in c.split('/') {
        let has_digit = part.bytes().any(|b| b.is_ascii_digit());
        let ends_letter = part.bytes().last().is_some_and(|b| b.is_ascii_uppercase());
        if has_digit && ends_letter && part.len() > best.len() {
            best = part.to_string();
        }
    }
    if best.is_empty() {
        c
    } else {
        best
    }
}

/// The US state a logged QSO counts for: the logged ADIF STATE when it's a
/// US state code and the QSO is identifiably a US station (its grid is a US
/// grid, or with no/short grid, its callsign has a US prefix) -- STATE also
/// holds other countries' subdivisions (e.g. Western Australia's "WA"),
/// which must not count as Washington. Otherwise, the state is derived from
/// the grid alone. Mirrors `GeneralVariables.workedStateFor`.
pub fn worked_state_for(
    logged_state: Option<&str>,
    grid: Option<&str>,
    callsign: Option<&str>,
) -> Option<String> {
    let from_grid = grid.and_then(state_for_grid);
    if let Some(logged) = logged_state {
        let s = logged.trim().to_uppercase();
        let grid_too_short = grid.map(|g| g.trim().len() < 4).unwrap_or(true);
        let us_station = from_grid.is_some()
            || (grid_too_short && callsign.is_some_and(|c| is_us_call(&base_callsign(c))));
        if us_station && US_STATE_CODES.contains(&s.as_str()) {
            return Some(s);
        }
    }
    from_grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_for_grid_resolves_known_us_grids() {
        // FN42 (Hartford, CT area) is well inside WSJT-X's own example grid
        // range and, being New England, unambiguous.
        assert!(state_for_grid("FN42").is_some());
    }

    #[test]
    fn state_for_grid_rejects_short_or_non_us_grids() {
        assert_eq!(state_for_grid("FN4"), None); // too short
        assert_eq!(state_for_grid(""), None);
        assert_eq!(state_for_grid("JO01"), None); // London-area grid, not US
    }

    #[test]
    fn is_us_call_accepts_k_n_w_and_rejects_others() {
        assert!(is_us_call("W1AW"));
        assert!(is_us_call("K1ABC"));
        assert!(is_us_call("N0AX"));
        assert!(is_us_call("AA1ZZZ"));
        assert!(!is_us_call("VE3ABC")); // Canada, not US
        assert!(!is_us_call("G4XYZ")); // England
        assert!(!is_us_call("W1")); // no letter suffix after the digit
        assert!(!is_us_call(""));
    }

    #[test]
    fn base_callsign_strips_portable_parts() {
        assert_eq!(base_callsign("W1AW/4"), "W1AW");
        assert_eq!(base_callsign("KH6/W1AW"), "W1AW");
        assert_eq!(base_callsign("W1AW/P"), "W1AW");
        assert_eq!(base_callsign("W1AW"), "W1AW");
    }

    #[test]
    fn worked_state_for_prefers_logged_state_for_a_us_station() {
        // A US station (resolvable via grid) with a logged non-US-looking
        // STATE elsewhere (e.g. a province code) falls through to the grid.
        assert_eq!(
            worked_state_for(Some("CT"), Some("FN42"), Some("W1AW")),
            Some("CT".to_string())
        );
    }

    #[test]
    fn worked_state_for_falls_back_to_grid_when_state_not_a_us_code() {
        // "WA" is a real US code (Washington) but also Western Australia's
        // ADIF subdivision; without a US-identifiable grid/call it must not
        // be trusted as Washington.
        assert_eq!(worked_state_for(Some("WA"), None, Some("VK6ABC")), None);
    }

    #[test]
    fn worked_state_for_uses_grid_when_nothing_logged() {
        assert_eq!(worked_state_for(None, Some("FN42"), None), state_for_grid("FN42"));
    }

    #[test]
    fn worked_state_for_us_call_with_no_grid_trusts_logged_state() {
        assert_eq!(
            worked_state_for(Some("ct"), None, Some("W1AW")),
            Some("CT".to_string())
        );
    }
}
