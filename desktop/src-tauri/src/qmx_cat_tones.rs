//! Transmit FT8 on a QRP Labs QMX by sending each tone as a Hamlib CAT
//! command instead of audio -- ported (in spirit) from the Android app's
//! `QmxCatTones` / `FT8TransmitSignal.playViaQmxCatTones`; see
//! `docs/qmx-transmit.md` for the full story and real-hardware measurements.
//!
//! In Digi mode the QMX accepts `TAnnnn.nn;` (firmware 1.04 CAT manual): it
//! transmits a single carrier at the USB dial frequency plus that audio
//! frequency (fractional Hz allowed). The first `TA` after `TX;` keys down
//! with a shaped envelope; `TA0;` (anything under 10 Hz) keys up the same
//! way. Sending one `TA` per symbol at the symbol rate reproduces the mode's
//! FSK exactly, with no audio path involved -- the fix for a QMX that audio
//! alone can't drive hard enough (confirmed on real hardware: the QMX's
//! input meter needs 3 bars to transmit; audio-only drive sometimes can't
//! get past 1-2 even at full software scale). See
//! [`crate::rig::needs_cat_tone_tx`] for when this path is used, and
//! [`crate::engine`] for the per-tick symbol clock that calls these helpers.
//!
//! Pure: command formatting and tone math only, so it's unit-testable
//! without a real rig.

/// Key-up: the QMX treats any `TA` below 10 Hz as "stop transmitting".
pub const KEY_UP: &str = "TA0;";

/// The `TAnnnn.nn;` command for an audio tone of `hz`, to 0.01 Hz.
pub fn ta_command(hz: f64) -> String {
    format!("TA{hz:.2};")
}

/// Audio frequency of channel tone `tone` (0-7 for FT8) above `base_hz`. Tone
/// spacing equals the symbol rate, 1 / symbol period (6.25 Hz for FT8's
/// 0.160 s symbols).
pub fn tone_hz(base_hz: f64, tone: u8, symbol_period_sec: f32) -> f64 {
    base_hz + tone as f64 / symbol_period_sec as f64
}

/// Symbols to skip when the over starts `late_ms` into its slot, so the
/// transmission still ends on the slot boundary -- mirrors the audio path's
/// leading-sample clip (`skip_cursor` in `audio/output.rs`) for the same
/// lateness, just in symbol units instead of samples.
pub fn symbols_to_skip(late_ms: i64, symbol_period_sec: f32, num_tones: usize) -> usize {
    if late_ms <= 0 {
        return 0;
    }
    let skip = (late_ms as f64 / (symbol_period_sec as f64 * 1000.0)) as usize;
    skip.min(num_tones)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ta_command_formats_to_two_decimal_places() {
        assert_eq!(ta_command(1500.0), "TA1500.00;");
        assert_eq!(ta_command(1431.25), "TA1431.25;");
        assert_eq!(ta_command(0.0), "TA0.00;");
    }

    #[test]
    fn tone_hz_spaces_tones_by_the_symbol_rate() {
        // FT8: 1 / 0.160 s = 6.25 Hz spacing. 0.160 isn't exactly
        // representable as f32, so 1 / 0.160 picks up a tiny rounding error
        // once widened to f64 (e.g. 1506.2500001396984, not 1506.25 on the
        // nose) -- compare with a generous epsilon rather than exact
        // equality, which is what the real caller (`ta_command`, formatting
        // to 0.01 Hz) effectively does too.
        let base = 1500.0;
        let close = |got: f64, want: f64| assert!((got - want).abs() < 1e-3, "{got} != {want}");
        close(tone_hz(base, 0, 0.160), 1500.0);
        close(tone_hz(base, 1, 0.160), 1506.25);
        close(tone_hz(base, 7, 0.160), 1543.75);
    }

    #[test]
    fn symbols_to_skip_is_zero_for_an_on_time_or_early_start() {
        assert_eq!(symbols_to_skip(0, 0.160, 79), 0);
        assert_eq!(symbols_to_skip(-50, 0.160, 79), 0);
    }

    #[test]
    fn symbols_to_skip_converts_lateness_to_whole_symbols() {
        // 320 ms late / 160 ms per symbol = exactly 2 symbols.
        assert_eq!(symbols_to_skip(320, 0.160, 79), 2);
        // Partial symbols don't round up -- only whole elapsed symbol periods
        // are skipped, matching the audio path's sample-exact clip.
        assert_eq!(symbols_to_skip(399, 0.160, 79), 2);
        assert_eq!(symbols_to_skip(400, 0.160, 79), 2);
    }

    #[test]
    fn symbols_to_skip_clamps_to_the_tone_count() {
        // A clip longer than the whole message skips at most every tone,
        // never overruns the array it will index into.
        assert_eq!(symbols_to_skip(60_000, 0.160, 79), 79);
    }
}
