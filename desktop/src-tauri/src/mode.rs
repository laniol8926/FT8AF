//! Operating-mode descriptor, ported from Android's `ModeProfile.java`: all
//! the per-mode timing/symbol/synthesis parameters live here so the engine
//! stays mode-agnostic rather than hard-coding FT8's 15 s/79-tone assumptions.
//!
//! FT2 transmits as well as receives, same as FT4/FT8. It reuses FT4's exact
//! tone-generation algorithm (same LDPC, XOR scramble, Costas pattern layout,
//! 105-symbol structure -- see `constants.h`'s `FTX_PROTOCOL_FT2` doc comment
//! in the vendored C core) at double the baud, so `dsp/encode.rs`'s
//! `encode_tones_mode` already produces correct FT2 tones via the shared
//! `ft4_encode` dispatch with no FT2-specific native code needed -- matching
//! `ModeProfile.encode()`'s identical `ft4Encode` dispatch for FT2 on
//! Android, which has never gated FT2 TX at all. Desktop's own TX gate was
//! removed once this was confirmed (previously: an overly cautious port
//! decision, not a real technical gap -- see this file's git history).
//!
//! FT2's TX timing margin is real tighter than FT4/FT8's, though: a 3.75 s
//! cycle holding a 2.52 s audio burst leaves only ~1.23 s of slack (vs
//! FT4's ~2.46 s, FT8's ~2.36 s) for the PTT-delay + late-start-tolerance
//! budget `tx_latest_ms` computes -- about half the cushion against a slow
//! CAT command or audio-device hiccup delaying TX start. Live-test FT2 TX
//! specifically for a late-start/clipped-Costas symptom before trusting it
//! unattended, per this project's standing caution on TX-timing changes.
//!
//! Decode works because desktop's FFI layer (dsp/ffi.rs, dsp/decoder.rs)
//! links the shared `ft8_lib` monitor/decoder directly rather than going
//! through Android's JNI wrappers, and `common/monitor.c` already derives
//! FT2's slot_time/symbol_period from `FTX_PROTOCOL_FT2` -- unlike Android,
//! which needed a separate `ft2_decode_jni.cpp` entry-point family, desktop
//! only needed the third protocol constant plumbed through.

use serde::{Deserialize, Serialize};

/// Margin desktop targets between the end of a mode's TX audio and the point
/// it hands the just-finished RX slot to the decoder (see `DECODE_AT_MS`'s
/// doc comment in engine.rs) -- FT8's existing 13_200ms trigger is exactly
/// 12_640 (its audio_ms) + 560, so FT4 uses the same margin rather than a
/// re-derived one.
const DECODE_MARGIN_MS: i64 = 560;

/// Default "PTT Delay" (ms) -- how long a rig needs after a keyed-up PTT
/// before transmit audio begins (see `engine.rs`'s real `PTT_DELAY_MS`
/// sleep, which must stay the same value as whatever's configured, since
/// `tx_latest_ms` promising a window the real settle sleep doesn't honor
/// would reintroduce exactly the late-start bug class this margin exists
/// to prevent). `Settings -> Advanced -> "PTT Delay"`, Android's own
/// `pttDelay` key/default/range (0-190ms, 10ms steps).
pub const DEFAULT_PTT_DELAY_MS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Ft8,
    Ft4,
    Ft2,
}

impl Mode {
    /// Resolve a persisted config value ("FT8"/"FT4"/"FT2", any case),
    /// defaulting to FT8 for anything else -- matches `ModeProfile.fromId`'s
    /// forward-compat fallback for an unknown persisted value.
    pub fn from_config(value: &str) -> Mode {
        if value.eq_ignore_ascii_case("FT4") {
            Mode::Ft4
        } else if value.eq_ignore_ascii_case("FT2") {
            Mode::Ft2
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
            Mode::Ft2 => "FT2",
        }
    }

    /// TX/RX cycle length in milliseconds -- matches `ModeProfile.slotMillis`.
    pub fn cycle_ms(self) -> i64 {
        match self {
            Mode::Ft8 => 15_000,
            Mode::Ft4 => 7_500,
            Mode::Ft2 => 3_750,
        }
    }

    /// GFSK symbol period in seconds -- matches `ModeProfile.symbolPeriod`.
    pub fn symbol_period(self) -> f32 {
        match self {
            Mode::Ft8 => 0.160,
            Mode::Ft4 => 0.048,
            Mode::Ft2 => 0.024,
        }
    }

    /// GFSK Gaussian BT product -- matches `ModeProfile.symbolBt`.
    pub fn symbol_bt(self) -> f32 {
        match self {
            Mode::Ft8 => 2.0,
            Mode::Ft4 => 1.0,
            Mode::Ft2 => 1.0,
        }
    }

    /// Number of channel symbols/tones -- matches `ModeProfile.numTones`.
    /// FT2 reuses FT4's tone layout (4-GFSK, 4 Costas, 105 symbols, same
    /// LDPC) at double the baud -- see `ModeProfile.FT2`'s doc comment.
    pub fn num_tones(self) -> usize {
        match self {
            Mode::Ft8 => 79,
            Mode::Ft4 => 105,
            Mode::Ft2 => 105,
        }
    }

    /// Whether the native decoder/encoder should use the FT8 protocol
    /// (false == FT4/FT2) -- matches `ModeProfile.isFt8`. Used both for
    /// decode (selects the FFI protocol constant) and for encode
    /// (`dsp/encode.rs`'s `encode_tones_mode` dispatches FT4 and FT2 alike
    /// to the shared `ft4_encode`, since their tone-generation algorithm is
    /// identical -- see this module's own doc comment).
    pub fn is_ft8(self) -> bool {
        matches!(self, Mode::Ft8)
    }

    /// The native `ftx_protocol_t` value for this mode (constants.h enum
    /// order: FT4 = 0, FT8 = 1, FT2 = 2), passed to the decoder so it
    /// configures the monitor for the right symbol period/sync layout --
    /// matches `ModeProfile.ftxProtocol()`.
    pub fn ffi_protocol(self) -> i32 {
        match self {
            Mode::Ft4 => 0,
            Mode::Ft8 => 1,
            Mode::Ft2 => 2,
        }
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
    /// matches engine.rs's `TX_LATEST_MS`, generalized per mode. Takes the
    /// *configured* PTT delay (Settings -> Advanced -> "PTT Delay",
    /// `Engine::ptt_delay_ms`) rather than a fixed constant, so lengthening
    /// it for a slow-settling rig correctly shrinks this window too --
    /// promising a window the real PTT-settle sleep doesn't honor would
    /// reintroduce the late-start/clipped-Costas bug class this margin
    /// exists to prevent.
    pub fn tx_latest_ms(self, ptt_delay_ms: i64) -> i64 {
        self.audio_slack_ms() - ptt_delay_ms
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
    use super::{Mode, DEFAULT_PTT_DELAY_MS};

    #[test]
    fn ft8_matches_existing_engine_constants() {
        // These must stay exactly engine.rs's CYCLE_MS/TX_LATEST_MS/DECODE_AT_MS
        // for FT8 -- Mode::Ft8 replaces those constants, not just approximates them.
        assert_eq!(Mode::Ft8.cycle_ms(), 15_000);
        assert_eq!(Mode::Ft8.audio_ms(), 12_640);
        assert_eq!(Mode::Ft8.tx_latest_ms(DEFAULT_PTT_DELAY_MS), 2_260);
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
        assert_eq!(Mode::Ft4.tx_latest_ms(DEFAULT_PTT_DELAY_MS), 2_360);
        assert!(!Mode::Ft4.is_ft8());
    }

    #[test]
    fn from_config_is_case_insensitive_and_defaults_to_ft8() {
        assert_eq!(Mode::from_config("FT4"), Mode::Ft4);
        assert_eq!(Mode::from_config("ft4"), Mode::Ft4);
        assert_eq!(Mode::from_config("FT8"), Mode::Ft8);
        assert_eq!(Mode::from_config(""), Mode::Ft8);
        assert_eq!(Mode::from_config("FT2"), Mode::Ft2);
        assert_eq!(Mode::from_config("ft2"), Mode::Ft2);
        assert_eq!(Mode::from_config("garbage"), Mode::Ft8);
    }

    #[test]
    fn display_name_round_trips_through_from_config() {
        for m in [Mode::Ft8, Mode::Ft4, Mode::Ft2] {
            assert_eq!(Mode::from_config(m.display_name()), m);
        }
    }

    #[test]
    fn default_is_ft8() {
        assert_eq!(Mode::default(), Mode::Ft8);
    }

    #[test]
    fn ft2_matches_android_modeprofile() {
        // ModeProfile.FT2(..., 3750, 3000, 0.024f, 1.0f, 105, false) --
        // audioMillis/audioSlackMillis are derived, not listed directly there.
        assert_eq!(Mode::Ft2.cycle_ms(), 3_750);
        assert_eq!(Mode::Ft2.num_tones(), 105);
        assert_eq!(Mode::Ft2.symbol_period(), 0.024);
        assert_eq!(Mode::Ft2.symbol_bt(), 1.0);
        assert_eq!(Mode::Ft2.audio_ms(), 2_520);
        assert_eq!(Mode::Ft2.audio_slack_ms(), 1_230);
        assert_eq!(Mode::Ft2.tx_latest_ms(DEFAULT_PTT_DELAY_MS), 1_130);
        assert_eq!(Mode::Ft2.decode_at_ms(), 3_080);
        assert!(!Mode::Ft2.is_ft8());
    }

    #[test]
    fn tx_latest_ms_shrinks_as_ptt_delay_grows() {
        // A slower-settling rig needs more PTT lead, which eats directly into
        // the window TX can still key up in -- the whole point of making
        // this configurable rather than a fixed constant.
        assert_eq!(Mode::Ft8.tx_latest_ms(0), 2_360);
        assert_eq!(Mode::Ft8.tx_latest_ms(190), 2_170);
    }

    #[test]
    fn ffi_protocol_matches_constants_h_enum_order() {
        // ft8_lib/ft8/constants.h: FT4 = 0, FT8 = 1, FT2 = 2.
        assert_eq!(Mode::Ft4.ffi_protocol(), 0);
        assert_eq!(Mode::Ft8.ffi_protocol(), 1);
        assert_eq!(Mode::Ft2.ffi_protocol(), 2);
    }
}
