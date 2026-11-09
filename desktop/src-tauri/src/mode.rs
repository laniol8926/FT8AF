//! Operating-mode descriptor, ported from Android's `ModeProfile.java`: all
//! the per-mode timing/symbol/synthesis parameters live here so the engine
//! stays mode-agnostic rather than hard-coding FT8's 15 s/79-tone assumptions.
//!
//! Only FT8 and FT4 are implemented (`ModeProfile` also lists FT2, deferred
//! here -- it needs its own native decode entry points Android's comment
//! describes as "from-source" `ft2_decode_jni.cpp`-equivalent bindings this
//! desktop FFI layer doesn't have at all yet, unlike FT4 which reuses the
//! same `ft8_lib` monitor/decoder with a different `ftx_protocol_t`).

use serde::{Deserialize, Serialize};

/// Margin desktop targets between the end of a mode's TX audio and the point
/// it hands the just-finished RX slot to the decoder (see `DECODE_AT_MS`'s
/// doc comment in engine.rs) -- FT8's existing 13_200ms trigger is exactly
/// 12_640 (its audio_ms) + 560, so FT4 uses the same margin rather than a
/// re-derived one.
const DECODE_MARGIN_MS: i64 = 560;

/// How long before a keyed-up PTT a rig needs (see `PTT_DELAY_MS` in
/// engine.rs) -- shared across modes, so `tx_latest_ms` can subtract it.
const PTT_DELAY_MS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Ft8,
    Ft4,
}

impl Mode {
    /// Resolve a persisted config value ("FT8"/"FT4", any case), defaulting
    /// to FT8 for anything else -- matches `ModeProfile.fromId`'s
    /// forward-compat fallback for an unknown persisted value.
    pub fn from_config(value: &str) -> Mode {
        if value.eq_ignore_ascii_case("FT4") {
            Mode::Ft4
        } else {
            Mode::Ft8
        }
    }

    /// The protocol name used for logs, ADIF, PSKReporter, and config --
    /// matches `ModeProfile.displayName`.
    pub fn display_name(self) -> &'static str {
        match self {
            Mode::Ft8 => "FT8",
            Mode::Ft4 => "FT4",
        }
    }

    /// TX/RX cycle length in milliseconds -- matches `ModeProfile.slotMillis`.
    pub fn cycle_ms(self) -> i64 {
        match self {
            Mode::Ft8 => 15_000,
            Mode::Ft4 => 7_500,
        }
    }

    /// GFSK symbol period in seconds -- matches `ModeProfile.symbolPeriod`.
    pub fn symbol_period(self) -> f32 {
        match self {
            Mode::Ft8 => 0.160,
            Mode::Ft4 => 0.048,
        }
    }

    /// GFSK Gaussian BT product -- matches `ModeProfile.symbolBt`.
    pub fn symbol_bt(self) -> f32 {
        match self {
            Mode::Ft8 => 2.0,
            Mode::Ft4 => 1.0,
        }
    }

    /// Number of channel symbols/tones -- matches `ModeProfile.numTones`.
    pub fn num_tones(self) -> usize {
        match self {
            Mode::Ft8 => 79,
            Mode::Ft4 => 105,
        }
    }

    /// Whether the native decoder/encoder should use the FT8 protocol
    /// (false == FT4) -- matches `ModeProfile.isFt8`, passed straight into
    /// `Decoder::new`'s `is_ft8` argument.
    pub fn is_ft8(self) -> bool {
        matches!(self, Mode::Ft8)
    }

    /// Audio waveform length in milliseconds, rounded -- matches
    /// `ModeProfile.audioMillis` (`numTones * symbolPeriod * 1000`).
    pub fn audio_ms(self) -> i64 {
        (self.num_tones() as f32 * self.symbol_period() * 1000.0).round() as i64
    }

    /// Slack between the end of the audio and the slot boundary -- matches
    /// `ModeProfile.audioSlackMillis`.
    pub fn audio_slack_ms(self) -> i64 {
        self.cycle_ms() - self.audio_ms()
    }

    /// Latest point into the cycle TX can still key up and have the full
    /// waveform (plus the PTT lead) land before the next slot boundary --
    /// matches engine.rs's `TX_LATEST_MS`, generalized per mode.
    pub fn tx_latest_ms(self) -> i64 {
        self.audio_slack_ms() - PTT_DELAY_MS
    }

    /// Point in the UTC cycle at which this mode's just-finished RX slot is
    /// handed to the decoder -- matches engine.rs's `DECODE_AT_MS`,
    /// generalized per mode (FT8: 12_640 + 560 == the existing 13_200).
    pub fn decode_at_ms(self) -> i64 {
        self.audio_ms() + DECODE_MARGIN_MS
    }
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Ft8
    }
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn ft8_matches_existing_engine_constants() {
        // These must stay exactly engine.rs's CYCLE_MS/TX_LATEST_MS/DECODE_AT_MS
        // for FT8 -- Mode::Ft8 replaces those constants, not just approximates them.
        assert_eq!(Mode::Ft8.cycle_ms(), 15_000);
        assert_eq!(Mode::Ft8.audio_ms(), 12_640);
        assert_eq!(Mode::Ft8.tx_latest_ms(), 2_260);
        assert_eq!(Mode::Ft8.decode_at_ms(), 13_200);
        assert!(Mode::Ft8.is_ft8());
    }

    #[test]
    fn ft4_matches_android_modeprofile() {
        // ModeProfile.FT4(..., 7500, 6500, 0.048f, 1.0f, 105, false) --
        // audioMillis/audioSlackMillis are derived, not listed directly there.
        assert_eq!(Mode::Ft4.cycle_ms(), 7_500);
        assert_eq!(Mode::Ft4.num_tones(), 105);
        assert_eq!(Mode::Ft4.symbol_period(), 0.048);
        assert_eq!(Mode::Ft4.symbol_bt(), 1.0);
        assert_eq!(Mode::Ft4.audio_ms(), 5_040);
        assert_eq!(Mode::Ft4.audio_slack_ms(), 2_460);
        assert_eq!(Mode::Ft4.tx_latest_ms(), 2_360);
        assert!(!Mode::Ft4.is_ft8());
    }

    #[test]
    fn from_config_is_case_insensitive_and_defaults_to_ft8() {
        assert_eq!(Mode::from_config("FT4"), Mode::Ft4);
        assert_eq!(Mode::from_config("ft4"), Mode::Ft4);
        assert_eq!(Mode::from_config("FT8"), Mode::Ft8);
        assert_eq!(Mode::from_config(""), Mode::Ft8);
        assert_eq!(Mode::from_config("FT2"), Mode::Ft8); // not implemented -- falls back
        assert_eq!(Mode::from_config("garbage"), Mode::Ft8);
    }

    #[test]
    fn display_name_round_trips_through_from_config() {
        for m in [Mode::Ft8, Mode::Ft4] {
            assert_eq!(Mode::from_config(m.display_name()), m);
        }
    }

    #[test]
    fn default_is_ft8() {
        assert_eq!(Mode::default(), Mode::Ft8);
    }
}
