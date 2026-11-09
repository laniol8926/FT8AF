//! FT8/FT4 encode path — safe wrappers over pack77 / ft8_encode|ft4_encode /
//! synth_gfsk_offset, mirroring `GenerateFT8.generateFt8`/`ModeProfile.encode`
//! on the Java side.

use super::ffi;
use crate::mode::Mode;
use std::ffi::CString;

pub const FT8_NN: usize = 79; // total channel symbols
pub const FT8_SYMBOL_PERIOD: f32 = 0.160;
pub const FT8_SYMBOL_BT: f32 = 2.0;
/// FT4's channel-symbol count -- matches `Mode::Ft4.num_tones()`, kept as its
/// own const (like `FT8_NN`) for sizing fixed-size buffers at compile time.
pub const FT4_NN: usize = 105;

/// Pack a structured FT8 message into a 10-byte (77-bit) payload.
/// Returns None if pack77 rejects the message (nonzero status).
pub fn pack77(msg: &str) -> Option<[u8; 10]> {
    let c = CString::new(msg).ok()?;
    let mut payload = [0u8; 10];
    let rc = unsafe { ffi::pack77(c.as_ptr(), payload.as_mut_ptr()) };
    if rc == 0 {
        Some(payload)
    } else {
        None
    }
}

/// Force free-text (i3=0, n3=0) packing regardless of structure.
pub fn pack_free_text(msg: &str) -> [u8; 10] {
    let c = CString::new(msg).unwrap_or_default();
    let mut payload = [0u8; 10];
    unsafe { ffi::packtext77(c.as_ptr(), payload.as_mut_ptr()) };
    payload
}

/// Encode a 10-byte payload into 79 FT8 tones (CRC + LDPC added internally).
pub fn encode_tones(payload: &[u8; 10]) -> [u8; FT8_NN] {
    let mut tones = [0u8; FT8_NN];
    unsafe { ffi::ft8_encode(payload.as_ptr(), tones.as_mut_ptr()) };
    tones
}

/// Encode a 10-byte payload into `mode`'s channel tones (CRC + LDPC added
/// internally) -- dispatches to `ft8_encode`/`ft4_encode`, matching
/// `ModeProfile.encode`. A `Vec` (not a fixed array) since the tone count
/// varies by mode (79 FT8, 105 FT4).
pub fn encode_tones_mode(payload: &[u8; 10], mode: Mode) -> Vec<u8> {
    let mut tones = vec![0u8; mode.num_tones()];
    unsafe {
        if mode.is_ft8() {
            ffi::ft8_encode(payload.as_ptr(), tones.as_mut_ptr());
        } else {
            ffi::ft4_encode(payload.as_ptr(), tones.as_mut_ptr());
        }
    }
    tones
}

/// Add the 14-bit CRC, producing the 12-byte a91 (77-bit payload + CRC).
pub fn add_crc(payload: &[u8; 10]) -> [u8; 12] {
    let mut a91 = [0u8; 12];
    unsafe { ffi::ftx_add_crc(payload.as_ptr(), a91.as_mut_ptr()) };
    a91
}

/// Number of `f32` samples a GFSK waveform of `n_tones` channel symbols occupies
/// at `sample_rate`. This is the single source of truth for the buffer size:
/// [`synth_gfsk`] writes exactly this many samples and [`generate_ft8`] allocates
/// exactly this many, so the allocation and the write count can never diverge.
///
/// The count is *per-symbol* rounded — `n_tones * round(sample_rate * symbol_period)`
/// — matching the native `synth_gfsk` in `ft8_lib`. Sizing the buffer with a single
/// *total*-rounded `round(n_tones * symbol_period * sample_rate)` instead comes up a
/// few samples short at a non-integral sample rate (the two roundings agree only
/// when `sample_rate * symbol_period` is integral, as at the shipped 12 kHz), which
/// tripped `synth_gfsk`'s length assert below. This is the desktop sibling of the
/// Android heap-OOB write fixed in PR #521.
// Only reached directly from this module's own FT8-specific regression tests
// below now that generate_ft8 routes through generate_signal/waveform_len_for.
#[allow(dead_code)]
pub(crate) fn waveform_len(n_tones: usize, sample_rate: i32) -> usize {
    waveform_len_for(n_tones, sample_rate, FT8_SYMBOL_PERIOD)
}

/// Mode-generalized [`waveform_len`]: same per-symbol rounding, parameterized
/// on `symbol_period` instead of always FT8's.
pub(crate) fn waveform_len_for(n_tones: usize, sample_rate: i32, symbol_period: f32) -> usize {
    let n_spsym = (0.5 + sample_rate as f64 * symbol_period as f64) as usize;
    n_tones * n_spsym
}

/// Render `tones` as a GFSK waveform into `signal[offset..]`.
///
/// # Panics
/// Panics if `signal` is too short for the waveform starting at `offset`.
pub fn synth_gfsk(tones: &[u8], f0: f32, sample_rate: i32, signal: &mut [f32], offset: usize) {
    synth_gfsk_for(tones, f0, sample_rate, signal, offset, FT8_SYMBOL_BT, FT8_SYMBOL_PERIOD);
}

/// Mode-generalized [`synth_gfsk`]: same buffer-length assert and FFI call,
/// parameterized on `symbol_bt`/`symbol_period` instead of always FT8's.
///
/// # Panics
/// Panics if `signal` is too short for the waveform starting at `offset`.
pub fn synth_gfsk_for(
    tones: &[u8],
    f0: f32,
    sample_rate: i32,
    signal: &mut [f32],
    offset: usize,
    symbol_bt: f32,
    symbol_period: f32,
) {
    let n_wave = waveform_len_for(tones.len(), sample_rate, symbol_period);
    assert!(
        offset + n_wave <= signal.len(),
        "synth_gfsk: signal buffer too short (need {} samples from offset {}, have {})",
        n_wave, offset, signal.len(),
    );
    unsafe {
        ffi::synth_gfsk_offset(
            tones.as_ptr(),
            tones.len() as i32,
            f0,
            symbol_bt,
            symbol_period,
            sample_rate,
            signal.as_mut_ptr(),
            offset as i32,
        );
    }
}

/// Full encode: structured message text -> 12.64 s of mono `f32` audio at
/// `sample_rate`, centered on `base_freq_hz`. Returns None if the text can't be
/// packed as a structured message. Mirrors `GenerateFT8.generateFt8`.
pub fn generate_ft8(text: &str, base_freq_hz: f32, sample_rate: i32) -> Option<Vec<f32>> {
    generate_signal(text, base_freq_hz, sample_rate, Mode::Ft8)
}

/// Mode-generalized [`generate_ft8`]: structured message text -> this mode's
/// full audio (12.64 s FT8, 5.04 s FT4) at `sample_rate`, centered on
/// `base_freq_hz`. Returns None if the text can't be packed as a structured
/// message (`pack77` itself is shared/mode-independent -- only the
/// tone-generation and waveform synthesis differ by mode).
pub fn generate_signal(text: &str, base_freq_hz: f32, sample_rate: i32, mode: Mode) -> Option<Vec<f32>> {
    let payload = pack77(text)?;
    let tones = encode_tones_mode(&payload, mode);
    // Size the buffer with the same per-symbol rounding synth_gfsk_for uses for
    // its write count, so it always fits exactly (see [`waveform_len_for`]).
    let num_samples = waveform_len_for(tones.len(), sample_rate, mode.symbol_period());
    let mut signal = vec![0.0f32; num_samples];
    synth_gfsk_for(
        &tones, base_freq_hz, sample_rate, &mut signal, 0, mode.symbol_bt(), mode.symbol_period(),
    );
    Some(signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independently replicates the write count `synth_gfsk` computes (per-symbol
    /// rounding), so this test fails if `waveform_len` ever drifts from it.
    fn synth_write_count(n_tones: usize, sample_rate: i32) -> usize {
        let n_spsym = (0.5 + sample_rate as f64 * FT8_SYMBOL_PERIOD as f64) as usize;
        n_tones * n_spsym
    }

    /// The old buffer size `generate_ft8` used to allocate (total-rounded). Kept
    /// here only to document the pre-fix behaviour the tests below contrast with.
    fn old_total_rounded_len(n_tones: usize, sample_rate: i32) -> usize {
        (0.5 + n_tones as f32 * FT8_SYMBOL_PERIOD * sample_rate as f32) as usize
    }

    #[test]
    fn allocation_always_covers_synth_write_count() {
        // Across integral and non-integral sample rates the allocated buffer must
        // exactly match synth_gfsk's write count — never short (would panic on the
        // `offset + n_wave <= signal.len()` assert), never wastefully over-sized.
        for &rate in &[8000, 11025, 12000, 12001, 12004, 12005, 16000, 22050, 24000, 44100, 48000] {
            let alloc = waveform_len(FT8_NN, rate);
            let need = synth_write_count(FT8_NN, rate);
            assert_eq!(alloc, need, "rate {rate}: buffer {alloc} must match write count {need}");
        }
    }

    #[test]
    fn byte_identical_at_shipped_rate() {
        // At the app's fixed 12 kHz rate the new per-symbol size equals the old
        // total-rounded size (12.64 s * 12000 = 151680), so shipped output is
        // unchanged — this is a pure robustness fix for other rates.
        assert_eq!(waveform_len(FT8_NN, 12000), 151680);
        assert_eq!(waveform_len(FT8_NN, 12000), old_total_rounded_len(FT8_NN, 12000));
    }

    #[test]
    fn old_total_rounded_size_underran_at_nonintegral_rate() {
        // Regression guard documenting the bug: at 12004 Hz the old total-rounded
        // buffer was shorter than synth_gfsk's per-symbol write count, so the assert
        // fired (the desktop sibling of PR #521). The new size covers it.
        let rate = 12004;
        let need = synth_write_count(FT8_NN, rate);
        assert!(
            old_total_rounded_len(FT8_NN, rate) < need,
            "expected the old size to underrun the write count at {rate} Hz",
        );
        assert!(waveform_len(FT8_NN, rate) >= need);
    }

    #[test]
    fn mode_aware_wrappers_match_the_ft8_only_ones_for_ft8() {
        let signal_old = generate_ft8("CQ K1ABC FN42", 1000.0, 12000).unwrap();
        let signal_new = generate_signal("CQ K1ABC FN42", 1000.0, 12000, Mode::Ft8).unwrap();
        assert_eq!(signal_old, signal_new, "generate_signal(.., Mode::Ft8) must be byte-identical to generate_ft8");

        let payload = pack77("CQ K1ABC FN42").unwrap();
        assert_eq!(encode_tones(&payload).to_vec(), encode_tones_mode(&payload, Mode::Ft8));
    }

    #[test]
    fn ft4_tone_count_and_signal_length_match_modeprofile() {
        let payload = pack77("CQ K1ABC FN42").unwrap();
        let tones = encode_tones_mode(&payload, Mode::Ft4);
        assert_eq!(tones.len(), Mode::Ft4.num_tones());
        assert_eq!(tones.len(), FT4_NN);

        let signal = generate_signal("CQ K1ABC FN42", 1000.0, 12000, Mode::Ft4).unwrap();
        // 5.04s of audio at 12kHz == 60480 samples, matching Mode::Ft4.audio_ms().
        assert_eq!(signal.len(), waveform_len_for(FT4_NN, 12000, Mode::Ft4.symbol_period()));
        let expected_samples = (Mode::Ft4.audio_ms() as f64 / 1000.0 * 12000.0).round() as usize;
        // Per-symbol rounding can differ by a handful of samples from the
        // naive total-duration estimate; assert they're close, not identical.
        assert!(
            (signal.len() as i64 - expected_samples as i64).unsigned_abs() < 200,
            "FT4 signal length {} should be close to {} (5.04s @ 12kHz)",
            signal.len(), expected_samples,
        );
    }

    #[test]
    fn ft4_waveform_len_matches_its_own_synth_write_count() {
        fn ft4_synth_write_count(n_tones: usize, sample_rate: i32) -> usize {
            let n_spsym = (0.5 + sample_rate as f64 * Mode::Ft4.symbol_period() as f64) as usize;
            n_tones * n_spsym
        }
        for &rate in &[8000, 12000, 12004, 44100, 48000] {
            let alloc = waveform_len_for(FT4_NN, rate, Mode::Ft4.symbol_period());
            let need = ft4_synth_write_count(FT4_NN, rate);
            assert_eq!(alloc, need, "rate {rate}: FT4 buffer {alloc} must match write count {need}");
        }
    }

    #[test]
    fn unpackable_text_returns_none_for_every_mode() {
        // An embedded NUL makes CString::new fail inside pack77 -- a reliable
        // way to force None regardless of pack77's own structured/free-text
        // fallback rules, which generate_signal must propagate for every mode
        // (pack77 is called once, before the mode-specific tone/synth branch).
        let unpackable = "bad\0text";
        assert!(generate_signal(unpackable, 1000.0, 12000, Mode::Ft8).is_none());
        assert!(generate_signal(unpackable, 1000.0, 12000, Mode::Ft4).is_none());
    }
}
