//! Small callsign-matching predicates shared by the live Decode list
//! (`engine.rs`'s `UiMessage` construction) and the QSO auto-sequencer
//! (`qso.rs`'s `process_rx`), ported from Android's `Ft8Message.checkIsCQ`
//! and `GeneralVariables.checkIsMyCallsign`/`getShortCallsign` so both apps
//! agree on what counts as a CQ and what counts as "addressed to me".

/// Whether a decoded message's `call_to` is a CQ-style address. Android
/// accepts "CQ", "DE", and "QRZ" (`Ft8Message.checkIsCQ`) -- `DE` and `QRZ`
/// are both seen on the air as CQ-equivalent openers, not just a bare "CQ".
/// Takes the first whitespace-separated token (mirroring
/// `callsignTo.trim().split(" ")[0]`) since `call_to` is a single callsign
/// token in practice, but a stray embedded space must not sink the match.
pub fn is_cq_callsign(call_to: &str) -> bool {
    let first = call_to.trim().split(' ').next().unwrap_or("");
    first.eq_ignore_ascii_case("CQ") || first.eq_ignore_ascii_case("DE") || first.eq_ignore_ascii_case("QRZ")
}

/// The longest `/`-separated segment of a callsign, e.g. `"W1AW/P"` ->
/// `"W1AW"`, `"KH6/W1AW"` -> `"W1AW"`. Ported from
/// `GeneralVariables.getShortCallsign`: a callsign with no `/` returns
/// itself unchanged. Ties keep the first (longest-so-far) segment, matching
/// the Java `>` (not `>=`) comparison.
fn short_callsign(call: &str) -> &str {
    if !call.contains('/') {
        return call;
    }
    let mut best = call;
    let mut best_len = 0;
    for part in call.split('/') {
        if part.len() > best_len {
            best = part;
            best_len = part.len();
        }
    }
    best
}

/// Whether `call_to` is addressed to the operator (`my_call`). Ported from
/// `GeneralVariables.checkIsMyCallsign`: the *operator's own* callsign is
/// reduced to its longest portable segment first (so `my_call = "W1AW"`
/// still matches a `call_to` of `"W1AW/P"`), then checked as a **substring**
/// of the decoded `call_to` as-is -- not an exact match, and the decoded
/// side is never stripped. An empty `my_call` never matches anything
/// (unconfigured station).
pub fn is_my_callsign(call_to: &str, my_call: &str) -> bool {
    if my_call.is_empty() {
        return false;
    }
    let call_to = call_to.to_uppercase();
    let short = short_callsign(&my_call.to_uppercase()).to_string();
    call_to.contains(&short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_cq_callsign_accepts_cq_de_and_qrz() {
        assert!(is_cq_callsign("CQ"));
        assert!(is_cq_callsign("cq")); // case-insensitive
        assert!(is_cq_callsign("DE"));
        assert!(is_cq_callsign("QRZ"));
        assert!(is_cq_callsign("  CQ  ")); // Android trims first
    }

    #[test]
    fn is_cq_callsign_rejects_a_real_callsign() {
        assert!(!is_cq_callsign("K1ABC"));
        assert!(!is_cq_callsign(""));
    }

    #[test]
    fn short_callsign_keeps_the_longest_slash_segment() {
        assert_eq!(short_callsign("W1AW/P"), "W1AW");
        assert_eq!(short_callsign("KH6/W1AW"), "W1AW");
        assert_eq!(short_callsign("W1AW"), "W1AW"); // no slash: unchanged
    }

    #[test]
    fn is_my_callsign_matches_a_portable_call_to_against_a_plain_my_call() {
        // my_call "W1AW" (no portable form) must still match a decoded
        // call_to of "W1AW/P" -- the exact case qso.rs's old strict
        // equality missed.
        assert!(is_my_callsign("W1AW/P", "W1AW"));
    }

    #[test]
    fn is_my_callsign_strips_a_portable_my_call_before_matching() {
        // Operator is signing portable; call_to is their plain base call.
        assert!(is_my_callsign("W1AW", "W1AW/P"));
    }

    #[test]
    fn is_my_callsign_is_case_insensitive() {
        assert!(is_my_callsign("w1aw", "W1AW"));
    }

    #[test]
    fn is_my_callsign_rejects_a_different_station_and_an_empty_my_call() {
        assert!(!is_my_callsign("K1XYZ", "W1AW"));
        assert!(!is_my_callsign("W1AW", ""));
    }
}
