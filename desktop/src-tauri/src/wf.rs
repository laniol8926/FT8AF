//! Live waterfall spectrum pipeline: windowed Welch-averaged FFT → dB →
//! noise-floor-referenced u8 rows (issue #428 developer knobs).
//!
//! Extracted from `engine.rs::emit_waterfall_row` so the row math is pure and
//! unit-testable, and so the window function, FFT size, and averaging count can
//! be changed at runtime. Defaults reproduce the previous hard-coded pipeline
//! byte-for-byte (Hann, 2048, 6 segments at 50% overlap). Display-only: the
//! decoder runs its own FFT inside the C core and is unaffected.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};

/// Displayed top of the audio band (matches decoder f_max) -- the *default*
/// for `WfConfig::display_hz`, now operator-adjustable ("Width" on the
/// waterfall strip, matching Android's `GeneralVariables.spectrumWidth`).
pub const WF_MAX_HZ: f32 = 3_500.0;
/// `WfConfig.display_hz` clamp range, matching Android's
/// `MIN_SPECTRUM_WIDTH_HZ`/`MAX_SPECTRUM_WIDTH_HZ` exactly.
pub const WF_MIN_DISPLAY_HZ: f32 = 2_500.0;
pub const WF_MAX_DISPLAY_HZ: f32 = 5_000.0;
/// dB above the black point that maps to full brightness.
pub const WF_SPAN_DB: f32 = 26.0;
/// Black point sits this far above the noise floor so noise stays dark.
pub const WF_BLACK_OFFSET_DB: f32 = 4.0;

/// FFT sizes the developer knob may select. Anything else sanitizes to 2048.
pub const ALLOWED_FFT_SIZES: [usize; 5] = [512, 1024, 2048, 4096, 8192];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WfWindow {
    Rect,
    Hann,
    Hamming,
    Blackman,
    BlackmanHarris,
}

impl WfWindow {
    /// Window coefficients in the periodic form (denominator N, not N-1),
    /// matching the Hann table the engine has always used:
    /// `0.5 - 0.5*cos(2πi/N)`.
    pub fn coeffs(self, n: usize) -> Vec<f32> {
        let n_f = n as f32;
        (0..n)
            .map(|i| {
                let x = 2.0 * std::f32::consts::PI * i as f32 / n_f;
                match self {
                    WfWindow::Rect => 1.0,
                    WfWindow::Hann => 0.5 - 0.5 * x.cos(),
                    WfWindow::Hamming => 0.54 - 0.46 * x.cos(),
                    WfWindow::Blackman => 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos(),
                    WfWindow::BlackmanHarris => {
                        0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                            - 0.01168 * (3.0 * x).cos()
                    }
                }
            })
            .collect()
    }

    /// Stable string form used as the `wf_window` config value.
    pub fn as_str(self) -> &'static str {
        match self {
            WfWindow::Rect => "rect",
            WfWindow::Hann => "hann",
            WfWindow::Hamming => "hamming",
            WfWindow::Blackman => "blackman",
            WfWindow::BlackmanHarris => "blackman_harris",
        }
    }

    /// Parse a config value; unknown strings fall back to the default (Hann).
    pub fn parse(s: &str) -> WfWindow {
        match s {
            "rect" => WfWindow::Rect,
            "hann" => WfWindow::Hann,
            "hamming" => WfWindow::Hamming,
            "blackman" => WfWindow::Blackman,
            "blackman_harris" => WfWindow::BlackmanHarris,
            _ => WfWindow::Hann,
        }
    }
}

/// Developer-tunable waterfall FFT parameters (issue #428), plus the
/// operator-facing display width. Persisted in the config table as
/// `wf_window` / `wf_fft_size` / `wf_avg` / `wf_display_hz`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WfConfig {
    pub window: WfWindow,
    pub fft_size: usize,
    /// Welch segments averaged per emitted row (50% overlap).
    pub avg: usize,
    /// Displayed span of the audio band in Hz -- "Width" on the waterfall
    /// strip, matching Android's `spectrumWidth`. Display-only geometry: it
    /// controls how many already-computed FFT bins are included in each
    /// emitted row (and so what the frontend's ruler/TX-marker math divides
    /// by), never the decoder's own analysis window.
    #[serde(default = "default_display_hz")]
    pub display_hz: f32,
    /// "NR" on the waterfall strip -- matches Android's `deNoise` exactly: a
    /// *display-only* band-tilt flattening (subtract each bin's local
    /// sliding-window dB average before mapping to brightness), ported from
    /// the shared `ft8af_magnitudes_to_display`'s denoise branch
    /// (`fft_display.c`). Never touches the decoder, which runs its own FFT
    /// in the C core independently -- same as Android's own NR toggle.
    /// Off by default, matching Android's `MainViewModel.deNoise = false`
    /// and preserving this pipeline's existing look until opted in.
    #[serde(default)]
    pub denoise: bool,
}

fn default_display_hz() -> f32 {
    WF_MAX_HZ
}

impl Default for WfConfig {
    fn default() -> Self {
        WfConfig { window: WfWindow::Hann, fft_size: 2048, avg: 6, display_hz: WF_MAX_HZ, denoise: false }
    }
}

impl WfConfig {
    /// Clamp out-of-range values to safe ones. Applied to anything arriving
    /// from the config table or the UI before it reaches the processor.
    pub fn sanitize(mut self) -> Self {
        if !ALLOWED_FFT_SIZES.contains(&self.fft_size) {
            self.fft_size = WfConfig::default().fft_size;
        }
        self.avg = self.avg.clamp(1, 16);
        // Mirrors Android's setSpectrumWidth: a corrupted/hand-edited 0 or
        // negative value must never reach the cols/bin_hz division below --
        // NaN/negative column counts would panic the Vec allocations.
        if !self.display_hz.is_finite() {
            self.display_hz = WF_MAX_HZ;
        }
        self.display_hz = self.display_hz.clamp(WF_MIN_DISPLAY_HZ, WF_MAX_DISPLAY_HZ);
        self
    }

    /// Hop between averaged segments (50% overlap).
    pub fn step(&self) -> usize {
        self.fft_size / 2
    }

    /// Samples of recent audio one row consumes.
    pub fn samples_needed(&self) -> usize {
        self.fft_size + (self.avg - 1) * self.step()
    }
}

/// Owns the FFT plan, window table, and smoothed noise-floor state for the
/// live waterfall. Rebuilt whenever the config changes.
pub struct WfProcessor {
    cfg: WfConfig,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Smoothed noise-floor estimate for auto color scaling. Restarts at
    /// -60 dB on rebuild and reconverges within a couple of seconds.
    floor_db: f32,
}

impl WfProcessor {
    pub fn new(cfg: WfConfig) -> Self {
        let cfg = cfg.sanitize();
        let fft = FftPlanner::<f32>::new().plan_fft_forward(cfg.fft_size);
        let window = cfg.window.coeffs(cfg.fft_size);
        WfProcessor { cfg, fft, window, floor_db: -60.0 }
    }

    pub fn config(&self) -> WfConfig {
        self.cfg
    }

    pub fn samples_needed(&self) -> usize {
        self.cfg.samples_needed()
    }

    /// Turn the most recent `samples_needed()` samples into one waterfall row:
    /// (u8 brightness per column, Hz per column). Returns None while the ring
    /// buffer is still warming up. Color is auto-scaled relative to a smoothed
    /// noise-floor estimate so the floor stays dark regardless of input gain.
    pub fn process_row(&mut self, samples: &[f32], sample_rate: f32) -> Option<(Vec<u8>, f32)> {
        let need = self.samples_needed();
        if samples.len() < need {
            return None;
        }
        let n = self.cfg.fft_size;
        let avg = self.cfg.avg;
        let step = self.cfg.step();

        let bin_hz = sample_rate / n as f32;
        let cols = ((self.cfg.display_hz / bin_hz) as usize).min(n / 2).max(1);

        // Average `avg` overlapping FFTs (Welch) to cut the per-bin variance of
        // a single FFT, so the noise floor reads smooth instead of speckled.
        let mut power = vec![0f32; cols];
        let mut buf: Vec<Complex<f32>> = vec![Complex { re: 0.0, im: 0.0 }; n];
        for seg in 0..avg {
            let off = seg * step;
            for i in 0..n {
                buf[i] = Complex { re: samples[off + i] * self.window[i], im: 0.0 };
            }
            self.fft.process(&mut buf);
            for (i, p) in power.iter_mut().enumerate() {
                *p += buf[i].re * buf[i].re + buf[i].im * buf[i].im;
            }
        }

        // Averaged power -> dB magnitude per bin. Normalization is kept
        // identical to the legacy pipeline (2/N, no window-gain correction):
        // the floor-relative mapping below absorbs the window's constant gain.
        let mut dbs = vec![0f32; cols];
        for (i, &p) in power.iter().enumerate() {
            let a = p / avg as f32;
            let mag = (a.sqrt() * 2.0) / n as f32;
            dbs[i] = 20.0 * (mag + 1e-12).log10();
        }

        // "NR": flatten slowly-varying band tilt before anything else reads
        // `dbs`, so both the noise-floor estimate below and the final
        // brightness see the flattened spectrum -- matches
        // ft8af_magnitudes_to_display's denoise branch exactly (flatten,
        // *then* take the floor reference).
        if self.cfg.denoise {
            flatten_band_tilt(&mut dbs);
        }

        // Estimate the noise floor from the 30th-percentile dB (robust even on
        // a busy band where signals fill much of the spectrum), smoothed over time.
        let mut sorted = dbs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pct = sorted[((cols as f32 * 0.30) as usize).min(cols - 1)];
        self.floor_db = self.floor_db * 0.9 + pct * 0.1;

        let black_point = self.floor_db + WF_BLACK_OFFSET_DB;
        Some((map_to_u8(&dbs, black_point, WF_SPAN_DB), bin_hz))
    }
}

/// "NR" display filter: subtract each bin's local sliding-window dB average
/// from itself, in place -- flattens a slowly-varying broadband noise slope
/// so it doesn't wash out one side of the band. Ported from
/// `ft8af_magnitudes_to_display`'s denoise branch (`fft_display.c`): window
/// is ~5% of the bins, floored at 8, and the per-bin average is computed via
/// a prefix sum so the whole pass is O(n) rather than O(n*window).
fn flatten_band_tilt(dbs: &mut [f32]) {
    let n = dbs.len();
    if n == 0 {
        return;
    }
    let win = (n / 20).max(8).min(n);

    // prefix[i] = sum of dbs[0..i], so a window's sum is prefix[right] - prefix[left].
    let mut prefix = vec![0f32; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + dbs[i];
    }

    let original = dbs.to_vec();
    for i in 0..n {
        let mut left = i as isize - win as isize / 2;
        let mut right = left + win as isize;
        if left < 0 {
            left = 0;
            right = win as isize;
        }
        if right > n as isize {
            right = n as isize;
            left = right - win as isize;
        }
        if left < 0 {
            left = 0;
        }
        let (left, right) = (left as usize, right as usize);
        let avg = (prefix[right] - prefix[left]) / (right - left) as f32;
        dbs[i] = original[i] - avg;
    }
}

/// Map per-bin dB to display brightness: 0 at/below the black point, 255 at
/// black point + span, linear in between.
fn map_to_u8(dbs: &[f32], black_point: f32, span_db: f32) -> Vec<u8> {
    dbs.iter()
        .map(|&db| {
            let t = ((db - black_point) / span_db).clamp(0.0, 1.0);
            (t * 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 12_000.0;

    fn tone(freq: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn window_coefficients_match_definitions() {
        let n = 16;
        for w in [
            WfWindow::Rect,
            WfWindow::Hann,
            WfWindow::Hamming,
            WfWindow::Blackman,
            WfWindow::BlackmanHarris,
        ] {
            let c = w.coeffs(n);
            assert_eq!(c.len(), n);
            // Periodic windows: w[i] == w[n-i] for i in 1..n.
            for i in 1..n {
                assert!(
                    (c[i] - c[n - i]).abs() < 1e-5,
                    "{w:?} not symmetric at {i}: {} vs {}",
                    c[i],
                    c[n - i]
                );
            }
        }
        assert!(WfWindow::Rect.coeffs(n).iter().all(|&v| v == 1.0));
        let hann = WfWindow::Hann.coeffs(n);
        assert!(hann[0].abs() < 1e-6);
        assert!((hann[n / 2] - 1.0).abs() < 1e-6);
        let hamming = WfWindow::Hamming.coeffs(n);
        assert!((hamming[0] - 0.08).abs() < 1e-6);
        assert!((hamming[n / 2] - 1.0).abs() < 1e-6);
        let blackman = WfWindow::Blackman.coeffs(n);
        assert!((blackman[n / 2] - 1.0).abs() < 1e-5);
        let bh = WfWindow::BlackmanHarris.coeffs(n);
        assert!(bh[0].abs() < 1e-4); // 4-term BH endpoint ≈ 6e-5
        assert!((bh[n / 2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn window_string_round_trip() {
        for w in [
            WfWindow::Rect,
            WfWindow::Hann,
            WfWindow::Hamming,
            WfWindow::Blackman,
            WfWindow::BlackmanHarris,
        ] {
            assert_eq!(WfWindow::parse(w.as_str()), w);
        }
        assert_eq!(WfWindow::parse("bogus"), WfWindow::Hann);
    }

    #[test]
    fn tone_lands_in_correct_bin_across_sizes() {
        for fft_size in [1024usize, 2048, 4096] {
            let cfg = WfConfig { window: WfWindow::Hann, fft_size, avg: 1, ..WfConfig::default() }.sanitize();
            let mut p = WfProcessor::new(cfg);
            let samples = tone(1000.0, p.samples_needed());
            let (bins, hz_per_col) = p.process_row(&samples, SR).unwrap();
            assert!((hz_per_col - SR / fft_size as f32).abs() < 1e-3);
            // Strong tones clamp several adjacent bins to 255, so instead of
            // comparing argmax indices, require the expected bin to be at the
            // maximum and bins a few away from it to be strictly darker.
            let expected = (1000.0 / hz_per_col).round() as usize;
            let max = *bins.iter().max().unwrap();
            assert_eq!(
                bins[expected], max,
                "fft {fft_size}: expected bin {expected} not at max"
            );
            assert!(
                bins[expected - 10] < max && bins[expected + 10] < max,
                "fft {fft_size}: tone energy not localized around bin {expected}"
            );
        }
    }

    #[test]
    fn u8_mapping_is_monotonic_and_clamped() {
        let dbs: Vec<f32> = (0..100).map(|i| -80.0 + i as f32).collect();
        let out = map_to_u8(&dbs, -50.0, WF_SPAN_DB);
        for w in out.windows(2) {
            assert!(w[1] >= w[0]);
        }
        assert_eq!(out[0], 0); // well below black point
        assert_eq!(*out.last().unwrap(), 255); // above black + span
        assert_eq!(map_to_u8(&[-50.0 + WF_SPAN_DB], -50.0, WF_SPAN_DB)[0], 255);
    }

    #[test]
    fn averaging_dilutes_a_step_transient() {
        // A tone present only in the newest FFT-length of audio: with 6-segment
        // Welch averaging its power is diluted vs a single-segment row. This is
        // the temporal-smearing mechanism issue #428 asks to make observable.
        // Amplitude is kept small so brightness stays inside the linear span
        // instead of clamping to 255 in both cases.
        let brightness_at_1k = |avg: usize| -> f32 {
            let cfg = WfConfig { window: WfWindow::Rect, fft_size: 2048, avg, ..WfConfig::default() };
            let mut p = WfProcessor::new(cfg);
            let need = p.samples_needed();
            let mut samples = vec![0f32; need];
            let t: Vec<f32> = tone(1000.0, 2048).iter().map(|v| v * 1e-3).collect();
            samples[need - 2048..].copy_from_slice(&t);
            let (bins, hz) = p.process_row(&samples, SR).unwrap();
            bins[(1000.0 / hz).round() as usize] as f32
        };
        let single = brightness_at_1k(1);
        let averaged = brightness_at_1k(6);
        assert!(single > 0.0);
        assert!(
            averaged < single,
            "expected 6-segment average ({averaged}) below single segment ({single})"
        );
    }

    #[test]
    fn samples_needed_and_sanitize() {
        assert_eq!(WfConfig::default().samples_needed(), 7168); // 2048 + 5*1024
        let s = WfConfig { window: WfWindow::Hann, fft_size: 3000, avg: 0, ..WfConfig::default() }.sanitize();
        assert_eq!(s.fft_size, 2048);
        assert_eq!(s.avg, 1);
        let s = WfConfig { window: WfWindow::Rect, fft_size: 512, avg: 99, ..WfConfig::default() }.sanitize();
        assert_eq!(s.fft_size, 512);
        assert_eq!(s.avg, 16);
    }

    #[test]
    fn display_hz_clamps_like_androids_spectrum_width() {
        let s = WfConfig { display_hz: 100.0, ..WfConfig::default() }.sanitize();
        assert_eq!(s.display_hz, WF_MIN_DISPLAY_HZ);
        let s = WfConfig { display_hz: 9_999.0, ..WfConfig::default() }.sanitize();
        assert_eq!(s.display_hz, WF_MAX_DISPLAY_HZ);
        let s = WfConfig { display_hz: 0.0, ..WfConfig::default() }.sanitize();
        assert_eq!(s.display_hz, WF_MIN_DISPLAY_HZ, "zero must clamp, not divide-by-zero downstream");
        let s = WfConfig { display_hz: f32::NAN, ..WfConfig::default() }.sanitize();
        assert_eq!(s.display_hz, WF_MAX_HZ, "non-finite falls back to the default, not a clamped NaN");
        // An in-range value passes through untouched.
        let s = WfConfig { display_hz: 4_000.0, ..WfConfig::default() }.sanitize();
        assert_eq!(s.display_hz, 4_000.0);
    }

    #[test]
    fn display_hz_controls_emitted_column_count() {
        // At a fixed sample rate/FFT size, a wider display_hz must emit more
        // columns (bin_hz is unchanged -- only how many of them get included).
        let mut narrow = WfProcessor::new(
            WfConfig { fft_size: 2048, avg: 1, display_hz: WF_MIN_DISPLAY_HZ, ..WfConfig::default() },
        );
        let mut wide = WfProcessor::new(
            WfConfig { fft_size: 2048, avg: 1, display_hz: WF_MAX_DISPLAY_HZ, ..WfConfig::default() },
        );
        let samples = vec![0f32; narrow.samples_needed()];
        let (narrow_bins, _) = narrow.process_row(&samples, SR).unwrap();
        let (wide_bins, _) = wide.process_row(&samples, SR).unwrap();
        assert!(wide_bins.len() > narrow_bins.len());
    }

    #[test]
    fn denoise_defaults_off_preserving_existing_behavior() {
        assert!(!WfConfig::default().denoise, "NR must default off, matching Android's deNoise=false");
    }

    #[test]
    fn flatten_band_tilt_removes_a_linear_slope() {
        // A pure linear ramp (broadband tilt, no signal) should flatten to
        // (near-)zero everywhere once its local average is subtracted off.
        let n = 200;
        let mut dbs: Vec<f32> = (0..n).map(|i| i as f32 * 0.1).collect();
        flatten_band_tilt(&mut dbs);
        for (i, &v) in dbs.iter().enumerate() {
            assert!(v.abs() < 1.0, "bin {i}: expected near-zero after flattening, got {v}");
        }
    }

    #[test]
    fn flatten_band_tilt_preserves_a_narrow_spike_against_a_tilted_floor() {
        // A real signal (a few bins wide) riding on a broadband tilt must
        // still stand out after flattening -- NR shouldn't erase the thing
        // the operator is trying to see, only the slope under it.
        let n = 200;
        let mut dbs: Vec<f32> = (0..n).map(|i| i as f32 * 0.05).collect();
        let spike_at = 100;
        dbs[spike_at] += 30.0;
        let before_spike_prominence = dbs[spike_at] - dbs[spike_at - 20];
        flatten_band_tilt(&mut dbs);
        let after_spike_prominence = dbs[spike_at] - dbs[spike_at - 20];
        assert!(dbs[spike_at] > dbs[spike_at - 20] + 20.0, "spike must still stand out after flattening");
        // The spike's prominence relative to nearby bins shouldn't be destroyed.
        assert!(after_spike_prominence > before_spike_prominence * 0.5);
    }

    #[test]
    fn flatten_band_tilt_handles_degenerate_sizes() {
        // No panic on empty or tiny (smaller than the minimum 8-bin window) input.
        let mut empty: Vec<f32> = vec![];
        flatten_band_tilt(&mut empty);
        assert!(empty.is_empty());

        let mut tiny = vec![1.0f32, 2.0, 3.0];
        flatten_band_tilt(&mut tiny);
        assert_eq!(tiny.len(), 3);
    }

    #[test]
    fn denoise_toggle_changes_process_row_output() {
        // Same input, same everything else, only `denoise` differs -- the
        // emitted row must actually differ when there's real tilt to flatten,
        // proving the config flag is wired through process_row.
        let mut off = WfProcessor::new(WfConfig { denoise: false, ..WfConfig::default() });
        let mut on = WfProcessor::new(WfConfig { denoise: true, ..WfConfig::default() });
        let mut samples = vec![0f32; off.samples_needed()];
        // Synthesize a tilted-noise-ish signal: a sum of low-frequency content
        // so the spectrum isn't flat (a literally flat/silent input leaves
        // nothing for NR to flatten, which would make this test meaningless).
        for (i, s) in samples.iter_mut().enumerate() {
            let t = i as f32 / SR;
            *s = 0.01 * (2.0 * std::f32::consts::PI * 150.0 * t).sin()
                + 0.1 * (2.0 * std::f32::consts::PI * 900.0 * t).sin();
        }

        let (bins_off, _) = off.process_row(&samples, SR).unwrap();
        let (bins_on, _) = on.process_row(&samples, SR).unwrap();
        assert_ne!(bins_off, bins_on, "denoise=true must change the emitted row");
    }

    #[test]
    fn default_config_matches_legacy_constants() {
        let d = WfConfig::default();
        assert_eq!(d.window, WfWindow::Hann);
        assert_eq!(d.fft_size, 2048);
        assert_eq!(d.avg, 6);
        assert_eq!(d.step(), 1024);
        // The Hann table must match the old inline formula from engine.rs.
        let table = WfWindow::Hann.coeffs(2048);
        for &i in &[0usize, 1, 511, 1024, 2047] {
            let legacy =
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / 2048.0).cos();
            assert!((table[i] - legacy).abs() < 1e-7, "hann mismatch at {i}");
        }
    }
}
