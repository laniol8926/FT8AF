//! The `STATE` to log for a QSO, ported verbatim from Android's
//! `LoggedState.kt`.
//!
//! QRZ's registered state is the station's home address: exact, but wrong
//! when the station is operating away from home. So it's used only when
//! QRZ's grid agrees with the grid the station actually sent (same 4
//! characters) and the QSO isn't portable; otherwise the state comes from
//! the worked grid instead.
//!
//! **Narrower than Android's own portable check**: Android's caller also
//! treats a non-empty ADIF `SIG`/`SIG_INFO` (a POTA/park activation) as
//! portable. Desktop's `QsoRecord` has no `SIG`/`SIG_INFO` field at all yet
//! (confirmed: no POTA-activation tracking exists on a QSO today), so only
//! the callsign-suffix half of "portable" is checked here -- a real gap,
//! not a silent approximation: a POTA activator logged under their plain
//! home callsign will use their QRZ home state even though they're
//! portable, same as it would without this feature at all. Fix this
//! properly once desktop tracks POTA activation on a QSO.

/// Picks the STATE to log. `qrz_state`/`qrz_grid` come from a QRZ XML
/// lookup of the other station (may be absent -- no subscription, no
/// cached lookup yet, or QRZ has no state on file); `grid_state` comes from
/// `us_states::state_for_grid` on the worked grid; `worked_grid` is the
/// grid actually heard/logged for this QSO.
pub fn pick_logged_state(
    grid_state: Option<&str>,
    qrz_state: Option<&str>,
    qrz_grid: Option<&str>,
    worked_grid: Option<&str>,
    portable: bool,
) -> Option<String> {
    let q = qrz_state
        .map(str::trim)
        .map(str::to_uppercase)
        .filter(|s| !s.is_empty());
    let same_grid = match (qrz_grid, worked_grid) {
        (Some(qg), Some(wg)) if qg.len() >= 4 && wg.len() >= 4 => {
            qg[..4].eq_ignore_ascii_case(&wg[..4])
        }
        _ => false,
    };
    let worked_grid_blank = worked_grid.map(str::trim).unwrap_or_default().is_empty();
    if !portable && q.is_some() && (same_grid || worked_grid_blank) {
        return q;
    }
    grid_state
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// True for a `/P`, `/M`, `/MM`, `/R`, `/AM` or `/QRP` suffix (operating
/// away from home).
pub fn is_portable_call(call: &str) -> bool {
    let upper = call.to_uppercase();
    let mut parts = upper.split('/');
    parts.next(); // the base call itself, never a suffix
    parts.any(|p| matches!(p, "P" | "M" | "MM" | "R" | "AM" | "QRP"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_qrz_state_when_grids_agree_and_not_portable() {
        let picked = pick_logged_state(Some("NY"), Some("ct"), Some("FN31pr"), Some("FN31ab"), false);
        assert_eq!(picked, Some("CT".to_string()));
    }

    #[test]
    fn falls_back_to_grid_state_when_grids_disagree() {
        // Operating from a different grid than QRZ's home grid -- not
        // "at home," so the worked-grid state wins even though QRZ has one.
        let picked = pick_logged_state(Some("NY"), Some("CT"), Some("FN31pr"), Some("FN20xy"), false);
        assert_eq!(picked, Some("NY".to_string()));
    }

    #[test]
    fn falls_back_to_grid_state_when_portable() {
        let picked = pick_logged_state(Some("NY"), Some("CT"), Some("FN31pr"), Some("FN31ab"), true);
        assert_eq!(picked, Some("NY".to_string()));
    }

    #[test]
    fn qrz_state_wins_with_no_worked_grid_at_all() {
        // No exchanged grid to compare against -- QRZ's home state is the
        // best information available, same as Android's `workedGrid.isNullOrBlank()`.
        let picked = pick_logged_state(None, Some("CT"), Some("FN31pr"), None, false);
        assert_eq!(picked, Some("CT".to_string()));
    }

    #[test]
    fn returns_none_when_nothing_is_known() {
        assert_eq!(pick_logged_state(None, None, None, None, false), None);
    }

    #[test]
    fn blank_qrz_state_is_treated_as_absent() {
        let picked = pick_logged_state(Some("NY"), Some("   "), Some("FN31pr"), Some("FN31ab"), false);
        assert_eq!(picked, Some("NY".to_string()));
    }

    #[test]
    fn is_portable_call_matches_known_suffixes() {
        for suffix in ["P", "M", "MM", "R", "AM", "QRP"] {
            assert!(is_portable_call(&format!("K1ABC/{suffix}")), "{suffix}");
        }
        assert!(!is_portable_call("K1ABC"));
        assert!(!is_portable_call("K1ABC/VE3")); // a prefix-style DXpedition suffix, not portable
    }

    #[test]
    fn is_portable_call_is_case_insensitive() {
        assert!(is_portable_call("k1abc/p"));
    }
}
