//! TX protection: SWR halt + lockout, ALC auto-volume, and the Tune max-on
//! timeout. Ported from Android's `MeterProtectionController.java` (SWR/ALC)
//! and `TuneController.java` (tune timing/lockout interaction) -- see
//! `TransmissionSettings.kt` for the Settings screen these back.
//!
//! Two real differences from Android, each documented at the function that
//! has it:
//!   - SWR here is an exact ratio from the QMX's own `SW;` CAT reply
//!     (`qmx_meters::parse_swr`), not Android's generic normalized 0-255
//!     value read from many different rigs' native meter callbacks -- no
//!     normalized<->ratio conversion is needed or implemented here, and the
//!     halt threshold is a plain ratio (e.g. `3.0`), not `0..255`.
//!   - The QMX firmware (1.04 CAT manual) exposes no ALC CAT query at all,
//!     and desktop's `poll_meters` only ever reads `SW;`/`PC;` -- so
//!     `AlcAutoVolume` is built and its settings persisted per an explicit
//!     "do all the logic... better to encorporate it now" instruction, but
//!     nothing in `engine.rs` calls `push_sample`/`on_tx_cycle_end` yet.
//!     Until a future ALC data source exists, it stays wired up but inert
//!     -- correctly disclosed in `TransmissionSettings.tsx`, not hidden.

/// Whether an SWR reading should halt TX right now. Never true while
/// `tuning` (a user Tune is deliberately driving SWR down; seeing high SWR
/// there is expected, not a fault -- Tune gets its own bounded max-on
/// timeout via [`tune_expired`] instead, matching Android's own
/// `shouldHaltForSwr`/`TuneController` split). Strictly greater-than the
/// threshold, matching Android's `normalizedSwr > threshold` exactly (an
/// exact-equal reading does not halt).
pub fn should_halt_for_swr(swr_ratio: Option<f32>, enabled: bool, threshold: f32, tuning: bool) -> bool {
    if !enabled || tuning {
        return false;
    }
    matches!(swr_ratio, Some(r) if r > threshold)
}

/// Default/min/max for the SWR halt threshold, as a plain ratio. Android's
/// own default is `swrHaltThreshold=120` normalized, documented as "~3.0:1".
pub const DEFAULT_SWR_HALT_THRESHOLD: f32 = 3.0;

// --- Tune max-on timeout -----------------------------------------------

/// Ported from `TuneController`'s `DEFAULT_MAX_ON_SECONDS`/`MIN_MAX_ON_SECONDS`/
/// `MAX_MAX_ON_SECONDS` -- same exact bounds, since this is a direct
/// Android-equivalent safety feature (desktop's Tune had no timeout at all
/// before this).
pub const DEFAULT_TUNE_MAX_ON_SECONDS: i64 = 10;
pub const MIN_TUNE_MAX_ON_SECONDS: i64 = 2;
pub const MAX_TUNE_MAX_ON_SECONDS: i64 = 60;

/// Clamp a configured max-on time to the legal range, same leniency as
/// `TuneController.clampMaxOnSeconds`: non-positive falls back to the
/// default (not the minimum -- a cleared/corrupt setting should behave as
/// "unset", not "as short as possible").
pub fn clamp_tune_max_on_seconds(seconds: i64) -> i64 {
    if seconds <= 0 {
        DEFAULT_TUNE_MAX_ON_SECONDS
    } else if seconds < MIN_TUNE_MAX_ON_SECONDS {
        MIN_TUNE_MAX_ON_SECONDS
    } else {
        seconds.min(MAX_TUNE_MAX_ON_SECONDS)
    }
}

// --- PTT Delay -----------------------------------------------------------

/// Default/max for "PTT Delay" (ms) -- Android's own `pttDelay`
/// default/range (0-190ms, a 20-step/10ms-increment slider in its UI).
/// Feeds both the real PTT-settle sleep (`Engine`'s own `PTT_DELAY_MS`
/// sleep sites) and `Mode::tx_latest_ms`'s eligibility window -- the two
/// must never disagree, see that function's doc comment.
pub const DEFAULT_PTT_DELAY_MS: i64 = crate::mode::DEFAULT_PTT_DELAY_MS;
pub const MAX_PTT_DELAY_MS: i64 = 190;

/// Clamp a configured PTT delay to `0..=MAX_PTT_DELAY_MS`. Unlike Tune's
/// max-on timeout, `0` is a real, legitimate value here (a QSK rig with no
/// settle time at all) -- it is not treated as "unset".
pub fn clamp_ptt_delay_ms(ms: i64) -> i64 {
    ms.clamp(0, MAX_PTT_DELAY_MS)
}

// --- Late-start Tolerance --------------------------------------------------

/// Max for "Late-start Tolerance" (ms) -- Android's own
/// `MAX_LATE_START_TOLERANCE_MS`. Android's *default* is 2000ms; desktop's
/// own default is deliberately `0`, not ported alongside the range --
/// `tx_slot_eligible`'s existing behavior (refuse to start at all once too
/// late, never transmit a clipped waveform) is strictly safer than
/// Android's "clip some leading audio and go anyway", and every desktop
/// install has only ever had that safer behavior. Introducing this knob
/// must not silently start clipping real transmissions for existing users
/// -- same reasoning as `autoCqAfterQso` keeping desktop's own prior
/// default instead of adopting Android's.
pub const MAX_LATE_START_TOLERANCE_MS: i64 = 4000;

/// Clamp a configured late-start tolerance to `0..=MAX_LATE_START_TOLERANCE_MS`.
/// `0` is the real, safe default (see this section's own doc comment), not
/// treated as "unset".
pub fn clamp_late_start_tolerance_ms(ms: i64) -> i64 {
    ms.clamp(0, MAX_LATE_START_TOLERANCE_MS)
}

#[cfg(test)]
mod ptt_and_late_start_tests {
    use super::*;

    #[test]
    fn ptt_delay_clamps_into_range() {
        assert_eq!(clamp_ptt_delay_ms(-50), 0);
        assert_eq!(clamp_ptt_delay_ms(0), 0);
        assert_eq!(clamp_ptt_delay_ms(100), 100);
        assert_eq!(clamp_ptt_delay_ms(190), 190);
        assert_eq!(clamp_ptt_delay_ms(500), 190);
    }

    #[test]
    fn late_start_tolerance_clamps_into_range_and_defaults_to_zero() {
        assert_eq!(clamp_late_start_tolerance_ms(-100), 0);
        assert_eq!(clamp_late_start_tolerance_ms(0), 0);
        assert_eq!(clamp_late_start_tolerance_ms(2000), 2000);
        assert_eq!(clamp_late_start_tolerance_ms(4000), 4000);
        assert_eq!(clamp_late_start_tolerance_ms(9000), 4000);
    }
}

/// True once a tune session started `elapsed_ms` ago should be force-stopped.
pub fn tune_expired(elapsed_ms: i64, max_on_seconds: i64) -> bool {
    elapsed_ms >= max_on_seconds * 1000
}

// --- Tune method -----------------------------------------------------------

/// Ported from `TuneMethod.java`. `atu_available` is always `false` on
/// desktop today: no currently-supported rig/Hamlib path exposes an
/// ATU-start CAT command here (the QMX, the one rig this app has real
/// meter/CAT-tone support for, has no internal ATU at all) -- so
/// `AUTOMATIC` and `TONE` currently behave identically (both fall back to
/// the carrier tone). `INTERNAL` still differs for real: it refuses to key
/// a surprise carrier when the operator explicitly asked for the rig's own
/// tuner and none is available, matching Android's documented safety
/// reasoning exactly. The setting and this logic are real and ready for
/// the day a rig backend adds ATU-start support -- only `atu_available`'s
/// input needs to change, not this function.
pub const TUNE_METHOD_AUTOMATIC: i32 = 0;
pub const TUNE_METHOD_INTERNAL: i32 = 1;
pub const TUNE_METHOD_TONE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuneAction {
    Carrier,
    RigAtu,
    Unavailable,
}

/// Unknown values clamp to AUTOMATIC, matching `TuneMethod.clamp`.
pub fn clamp_tune_method(method: i32) -> i32 {
    if method == TUNE_METHOD_INTERNAL || method == TUNE_METHOD_TONE {
        method
    } else {
        TUNE_METHOD_AUTOMATIC
    }
}

pub fn decide_tune_action(method: i32, atu_available: bool) -> TuneAction {
    match clamp_tune_method(method) {
        TUNE_METHOD_INTERNAL => {
            if atu_available {
                TuneAction::RigAtu
            } else {
                TuneAction::Unavailable
            }
        }
        TUNE_METHOD_TONE => TuneAction::Carrier,
        _ /* AUTOMATIC */ => {
            if atu_available {
                TuneAction::RigAtu
            } else {
                TuneAction::Carrier
            }
        }
    }
}

// --- ALC auto-volume -----------------------------------------------------

/// Matches Android's `ALC_WINDOW_SIZE`/`VOLUME_GAIN`/`VOLUME_MAX_STEP`/
/// `VOLUME_MIN`/`VOLUME_MAX` exactly. ALC stays on Android's own normalized
/// 0-255 scale (see module doc) even though desktop has no live reading in
/// that scale yet -- deliberate, so a future real feed and Android's own
/// settings-backup export/import both need no unit conversion. `VOLUME_MIN`/
/// `VOLUME_MAX`/the gain step apply directly to desktop's existing 0.0-1.0
/// `tx_gain`, which already matches Android's `volumePercent` scale.
pub const ALC_WINDOW_SIZE: usize = 5;
pub const VOLUME_GAIN: f32 = 0.4;
pub const VOLUME_MAX_STEP: f32 = 0.08;
pub const VOLUME_MIN: f32 = 0.05;
pub const VOLUME_MAX: f32 = 1.0;
pub const DEFAULT_ALC_TARGET_LOW: f32 = 60.0;
pub const DEFAULT_ALC_TARGET_HIGH: f32 = 100.0;

/// Accumulates ALC samples across one TX cycle and makes exactly one
/// proportional volume correction per cycle, in [`on_tx_cycle_end`] --
/// mirrors Android's own reasoning exactly: the TX audio's volume is baked
/// in once per cycle (resampled/gained before `prepare` hands it to the
/// output stream), so a correction can only take effect on the *next*
/// cycle, and correcting on every reading would just repeatedly react to a
/// value that can't have changed yet and overshoot.
#[derive(Default)]
pub struct AlcAutoVolume {
    window: Vec<f32>,
}

impl AlcAutoVolume {
    /// Record one ALC reading (0-255 normalized) taken during the current
    /// TX cycle. No-op data structure only -- nothing decides here.
    pub fn push_sample(&mut self, normalized_alc: f32) {
        self.window.push(normalized_alc);
        if self.window.len() > ALC_WINDOW_SIZE {
            self.window.remove(0);
        }
    }

    fn average(&self) -> Option<f32> {
        if self.window.is_empty() {
            None
        } else {
            Some(self.window.iter().sum::<f32>() / self.window.len() as f32)
        }
    }

    /// Call once when a TX cycle ends (whether or not any samples arrived).
    /// Returns the corrected gain only when a correction is actually
    /// warranted -- i.e. at least one sample arrived this cycle AND its
    /// average falls outside `[target_low, target_high]` -- matching
    /// Android's `onTxCycleEnd` exactly (it only touches volume when
    /// `avg < low || avg > high`). Always clears the window afterward,
    /// matching Android's unconditional `resetAccumulators()`.
    pub fn on_tx_cycle_end(&mut self, current_gain: f32, target_low: f32, target_high: f32) -> Option<f32> {
        let avg = self.average();
        self.window.clear();
        let avg = avg?;
        if avg >= target_low && avg <= target_high {
            return None;
        }
        let target = (target_low + target_high) / 2.0;
        let error = (target - avg) / 255.0;
        let step = (error * VOLUME_GAIN).clamp(-VOLUME_MAX_STEP, VOLUME_MAX_STEP);
        let new_gain = (current_gain + step).clamp(VOLUME_MIN, VOLUME_MAX);
        if new_gain == current_gain {
            None
        } else {
            Some(new_gain)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- should_halt_for_swr (mirrors MeterProtectionControllerTest's
    // shouldHalt_onlyWhenEnabledValidAndOverThreshold / shouldHalt_neverWhileTuning) ---

    #[test]
    fn halts_only_when_enabled_valid_and_over_threshold() {
        assert!(should_halt_for_swr(Some(3.75), true, 3.0, false));
        // At/under threshold -> no halt (strictly greater-than).
        assert!(!should_halt_for_swr(Some(3.0), true, 3.0, false));
        assert!(!should_halt_for_swr(Some(2.25), true, 3.0, false));
        // Protection off -> never halt, even at a high reading.
        assert!(!should_halt_for_swr(Some(7.0), false, 3.0, false));
        // No reading from the rig -> never halt.
        assert!(!should_halt_for_swr(None, true, 3.0, false));
    }

    #[test]
    fn never_halts_while_tuning() {
        assert!(!should_halt_for_swr(Some(3.75), true, 3.0, true));
        assert!(!should_halt_for_swr(Some(10.0), true, 3.0, true));
        // The same reading DOES halt once the tune ends (tuning=false).
        assert!(should_halt_for_swr(Some(3.75), true, 3.0, false));
    }

    // --- tune timeout ---

    #[test]
    fn tune_max_on_clamps_like_android() {
        assert_eq!(clamp_tune_max_on_seconds(0), DEFAULT_TUNE_MAX_ON_SECONDS);
        assert_eq!(clamp_tune_max_on_seconds(-5), DEFAULT_TUNE_MAX_ON_SECONDS);
        assert_eq!(clamp_tune_max_on_seconds(1), MIN_TUNE_MAX_ON_SECONDS);
        assert_eq!(clamp_tune_max_on_seconds(30), 30);
        assert_eq!(clamp_tune_max_on_seconds(500), MAX_TUNE_MAX_ON_SECONDS);
    }

    #[test]
    fn tune_expires_at_the_deadline_not_before() {
        assert!(!tune_expired(9_999, 10));
        assert!(tune_expired(10_000, 10));
        assert!(tune_expired(15_000, 10));
    }

    // --- ALC auto-volume ---

    // --- tune method ---

    #[test]
    fn tune_method_clamps_unknown_to_automatic() {
        assert_eq!(clamp_tune_method(99), TUNE_METHOD_AUTOMATIC);
        assert_eq!(clamp_tune_method(-1), TUNE_METHOD_AUTOMATIC);
        assert_eq!(clamp_tune_method(TUNE_METHOD_INTERNAL), TUNE_METHOD_INTERNAL);
    }

    #[test]
    fn automatic_prefers_rig_atu_falls_back_to_carrier() {
        assert_eq!(decide_tune_action(TUNE_METHOD_AUTOMATIC, true), TuneAction::RigAtu);
        assert_eq!(decide_tune_action(TUNE_METHOD_AUTOMATIC, false), TuneAction::Carrier);
    }

    #[test]
    fn internal_never_falls_back_to_a_surprise_carrier() {
        assert_eq!(decide_tune_action(TUNE_METHOD_INTERNAL, true), TuneAction::RigAtu);
        assert_eq!(decide_tune_action(TUNE_METHOD_INTERNAL, false), TuneAction::Unavailable);
    }

    #[test]
    fn tone_always_carrier_regardless_of_atu() {
        assert_eq!(decide_tune_action(TUNE_METHOD_TONE, true), TuneAction::Carrier);
        assert_eq!(decide_tune_action(TUNE_METHOD_TONE, false), TuneAction::Carrier);
    }

    #[test]
    fn alc_no_samples_this_cycle_makes_no_correction() {
        let mut alc = AlcAutoVolume::default();
        assert_eq!(alc.on_tx_cycle_end(0.5, 60.0, 100.0), None);
    }

    #[test]
    fn alc_within_target_window_makes_no_correction() {
        let mut alc = AlcAutoVolume::default();
        alc.push_sample(80.0);
        assert_eq!(alc.on_tx_cycle_end(0.5, 60.0, 100.0), None);
    }

    #[test]
    fn alc_hot_reading_reduces_volume() {
        let mut alc = AlcAutoVolume::default();
        for _ in 0..3 {
            alc.push_sample(200.0);
        }
        // target = 80, avg = 200 -> error = (80-200)/255 = -0.47, raw step -0.19,
        // clamped to -VOLUME_MAX_STEP.
        let new_gain = alc.on_tx_cycle_end(0.5, 60.0, 100.0).expect("should correct");
        assert!(new_gain < 0.5, "expected volume to drop, got {new_gain}");
        assert!((new_gain - (0.5 - VOLUME_MAX_STEP)).abs() < 1e-5);
    }

    #[test]
    fn alc_cold_reading_increases_volume() {
        let mut alc = AlcAutoVolume::default();
        alc.push_sample(10.0);
        let new_gain = alc.on_tx_cycle_end(0.5, 60.0, 100.0).expect("should correct");
        assert!(new_gain > 0.5, "expected volume to rise, got {new_gain}");
    }

    #[test]
    fn alc_step_is_capped_at_max_step() {
        let mut alc = AlcAutoVolume::default();
        alc.push_sample(255.0); // as hot as it gets
        let new_gain = alc.on_tx_cycle_end(0.9, 60.0, 100.0).expect("should correct");
        assert!((0.9 - new_gain - VOLUME_MAX_STEP).abs() < 1e-5);
    }

    #[test]
    fn alc_never_corrects_below_volume_min_or_above_max() {
        let mut too_hot = AlcAutoVolume::default();
        too_hot.push_sample(255.0);
        let g = too_hot.on_tx_cycle_end(VOLUME_MIN, 60.0, 100.0).unwrap_or(VOLUME_MIN);
        assert!(g >= VOLUME_MIN);

        let mut too_cold = AlcAutoVolume::default();
        too_cold.push_sample(0.0);
        let g = too_cold.on_tx_cycle_end(VOLUME_MAX, 60.0, 100.0).unwrap_or(VOLUME_MAX);
        assert!(g <= VOLUME_MAX);
    }

    #[test]
    fn alc_window_averages_and_caps_at_five_samples() {
        let mut alc = AlcAutoVolume::default();
        // Push 7 samples; only the last 5 (120,130,140,150,160) should count.
        for v in [10.0, 20.0, 120.0, 130.0, 140.0, 150.0, 160.0] {
            alc.push_sample(v);
        }
        assert_eq!(alc.average(), Some((120.0 + 130.0 + 140.0 + 150.0 + 160.0) / 5.0));
    }

    #[test]
    fn alc_window_clears_after_cycle_end_regardless_of_correction() {
        let mut alc = AlcAutoVolume::default();
        alc.push_sample(80.0); // within window, no correction
        alc.on_tx_cycle_end(0.5, 60.0, 100.0);
        assert_eq!(alc.average(), None);
    }
}
