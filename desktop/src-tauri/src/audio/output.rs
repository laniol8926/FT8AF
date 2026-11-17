//! TX audio playback: render a 12 kHz mono FT8 waveform to an output device,
//! resampling to the device rate and fanning the mono sample across channels.
//!
//! Playback is split into two phases so the engine can key the rig with minimal
//! dead air and stop a transmission mid-stream:
//!   * [`prepare`] does the heavy lifting — open the device, resample the whole
//!     waveform, build the (paused) output stream. The caller runs this *before*
//!     raising PTT so the rig isn't keyed during the resample (which is slow on
//!     weak CPUs).
//!   * [`PreparedTx::start`] is cheap: seek past any leading clip and unpause the
//!     stream. The caller runs it right after PTT settles.
//! The returned handle is polled (`is_done`/`timed_out`) and can be `abort`ed or
//! simply dropped to stop audio immediately (dropping the stream halts the cpal
//! callback). The `cpal::Stream` is `!Send` on Windows, so the handle must live
//! on the thread that created it (the engine thread).
//!
//! [`prepare_tune`]/[`TunePlayback`] are the simpler sibling for TUNE: no FT8
//! waveform, no two-phase split (nothing expensive to hide before PTT) --
//! just a looping steady tone, built and started in one call, stopped by
//! dropping the handle.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::channel_select;
use super::devices::find_output_device;
use super::resample::resample_buffer;
use crate::dsp::SAMPLE_RATE;

/// Slack added to the expected playback duration before the stuck-device
/// deadline trips and PTT is force-released.
const DEADLINE_MARGIN_SECS: f32 = 2.0;

/// Cursor (in device-rate mono samples) to start playback from, given a leading
/// clip of `skip_ms` of the original 12 kHz waveform. Clamped to `total` so an
/// over-long clip simply starts at the end (plays nothing) rather than panicking.
fn skip_cursor(skip_ms: i64, device_rate: u32, total: usize) -> usize {
    let skip = (skip_ms.max(0) * device_rate as i64 / 1000) as usize;
    skip.min(total)
}

/// Seconds of audio left to play from `cursor` to the end of a `total`-sample
/// device-rate buffer. Drives the stuck-device deadline.
fn remaining_secs(total: usize, cursor: usize, device_rate: u32) -> f32 {
    total.saturating_sub(cursor) as f32 / device_rate.max(1) as f32
}

/// A built-but-not-yet-playing TX waveform plus the shared state its cpal
/// callback drives. Hold it on the engine thread; poll [`is_done`]/[`timed_out`]
/// each loop tick and call [`finalize`-by-drop] (drop) when finished or stopping.
pub struct PreparedTx {
    // Kept alive for the life of the transmission; dropping it stops the audio.
    _stream: cpal::Stream,
    cursor: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    total: usize,
    device_rate: u32,
    /// Set when playback starts; bounds how long we'll hold PTT if the device
    /// callback never reaches the end (so a stalled stream can't key forever).
    deadline: Option<Instant>,
}

impl PreparedTx {
    /// Begin playback, skipping the first `skip_ms` of the (12 kHz) waveform.
    /// Cheap — meant to run immediately after PTT settles. Idempotent-safe.
    pub fn start(&mut self, skip_ms: i64) -> anyhow::Result<()> {
        let skip = skip_cursor(skip_ms, self.device_rate, self.total);
        self.cursor.store(skip, Ordering::Relaxed);
        self._stream.play()?;
        let remaining = remaining_secs(self.total, skip, self.device_rate);
        self.deadline = Some(Instant::now() + Duration::from_secs_f32(remaining + DEADLINE_MARGIN_SECS));
        Ok(())
    }

    /// True once the whole buffer has been clocked out (or playback was aborted).
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    /// True if playback has been running longer than its expected duration plus a
    /// safety margin — a stuck-device backstop so PTT is never held indefinitely.
    pub fn timed_out(&self) -> bool {
        matches!(self.deadline, Some(d) if Instant::now() > d)
    }

    /// Signal the callback to stop emitting samples (fills silence, marks done).
    /// Dropping the handle also stops audio; this just makes the intent explicit.
    pub fn abort(&self) {
        self.abort.store(true, Ordering::Relaxed);
    }
}

/// Apply the operator's TX `gain` and clamp every sample to the DAC's full-scale
/// range `[-1.0, 1.0]`.
///
/// The clamp is unconditional — deliberately **not** gated on `gain != 1.0`. FFT
/// resampling of the full-scale GFSK waveform (`synth_gfsk` peaks at exactly
/// ±1.0) interpolates a few samples slightly past full scale — measured ≈1.05
/// for 12 kHz→48 kHz — and the f32 output path hands samples to the driver
/// verbatim (`f32::from_sample` is identity) with no saturation net, unlike the
/// i16 cast which saturates. At `gain == 1.0` (TX level at 100%) those >1.0
/// samples would otherwise reach the soundcard unclamped and overdrive the rig's
/// input past 0 dBFS. Clamping here mirrors Android, which clamps unconditionally
/// on the output cast (`FT8TransmitSignal.float2Short` / `floatToInt16NoPad`),
/// kept separate from its gain step. For in-range samples the result is
/// byte-identical to a bare multiply, so the common `gain < 1.0` path is
/// unchanged.
fn apply_gain_and_clip(data: &mut [f32], gain: f32) {
    for s in data.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

/// Write one output frame (one sample per channel) from a mono `sample`.
///
/// `channel` (see [`channel_select`]) picks which channel carries the
/// signal. [`channel_select::BOTH`] (the historical default) fans the same
/// sample out to every channel. [`channel_select::LEFT`]/[`RIGHT`] --
/// `LEFT` needed to fully drive the QRP Labs QMX's modulator, see
/// [`crate::rig::needs_left_channel_tx_audio`] -- confine the signal to one
/// channel and silence every other, instead of leaving the mono-to-stereo
/// upmix to the USB Audio Class device. A mono (`frame.len() == 1`) device
/// has no second channel to silence, so the selection has no effect there.
fn write_frame<T: SizedSample + FromSample<f32>>(frame: &mut [T], sample: f32, channel: i32) {
    if frame.len() <= 1 {
        let v = T::from_sample(sample);
        for s in frame.iter_mut() {
            *s = v;
        }
        return;
    }
    match channel_select::clamp(channel) {
        channel_select::LEFT => {
            frame[0] = T::from_sample(sample);
            for s in &mut frame[1..] {
                *s = T::from_sample(0.0f32);
            }
        }
        channel_select::RIGHT => {
            for (i, s) in frame.iter_mut().enumerate() {
                *s = T::from_sample(if i == 1 { sample } else { 0.0f32 });
            }
        }
        _ => {
            let v = T::from_sample(sample);
            for s in frame.iter_mut() {
                *s = v;
            }
        }
    }
}

/// Render a 12 kHz mono buffer for `device_name`, applying `gain`, and build the
/// (paused) output stream. Does NOT key audio — call [`PreparedTx::start`] to
/// begin. This is the slow phase (device open + full-buffer resample); run it
/// before raising PTT. `channel` (see [`channel_select`]) picks which channel
/// carries the signal on a multi-channel device (see [`write_frame`]).
pub fn prepare(
    device_name: Option<&str>,
    samples_12k: &[f32],
    gain: f32,
    channel: i32,
) -> anyhow::Result<PreparedTx> {
    let device = find_output_device(device_name)
        .ok_or_else(|| anyhow::anyhow!("no output audio device available"))?;
    let supported = device.default_output_config()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();
    let channels = config.channels as usize;
    let device_rate = config.sample_rate.0;

    let mut data = resample_buffer(samples_12k, SAMPLE_RATE as u32, device_rate);
    apply_gain_and_clip(&mut data, gain);
    let total = data.len();
    let buffer = Arc::new(data);
    let cursor = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let abort = Arc::new(AtomicBool::new(false));

    let err_fn = |e| log::error!("audio output stream error: {e}");

    let stream = match sample_format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, channels, &buffer, &cursor, &done, &abort, channel, err_fn)?,
        SampleFormat::I16 => build_output::<i16>(&device, &config, channels, &buffer, &cursor, &done, &abort, channel, err_fn)?,
        SampleFormat::U16 => build_output::<u16>(&device, &config, channels, &buffer, &cursor, &done, &abort, channel, err_fn)?,
        other => anyhow::bail!("unsupported output sample format: {other:?}"),
    };

    Ok(PreparedTx {
        _stream: stream,
        cursor,
        done,
        abort,
        total,
        device_rate,
        deadline: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: &Arc<Vec<f32>>,
    cursor: &Arc<AtomicUsize>,
    done: &Arc<AtomicBool>,
    abort: &Arc<AtomicBool>,
    channel: i32,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let buffer = buffer.clone();
    let cursor = cursor.clone();
    let done = done.clone();
    let abort = abort.clone();
    let stream = device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            // Stop TX: emit silence and report done so the engine drops PTT.
            if abort.load(Ordering::Relaxed) {
                done.store(true, Ordering::Relaxed);
                out.fill(T::from_sample(0.0f32));
                return;
            }
            for frame in out.chunks_mut(channels) {
                let idx = cursor.load(Ordering::Relaxed);
                let sample = if idx < buffer.len() {
                    cursor.store(idx + 1, Ordering::Relaxed);
                    buffer[idx]
                } else {
                    done.store(true, Ordering::Relaxed);
                    0.0
                };
                write_frame(frame, sample, channel);
            }
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

/// A live TUNE transmission: a steady tone looped indefinitely until this
/// handle is dropped, which halts the cpal stream immediately (the same way
/// dropping a [`PreparedTx`] does). No `is_done`/`abort`/timeout here --
/// unlike a normal over there's nothing to finish or get stuck on; the
/// operator (or the engine's `finalize_tx`) decides when to stop.
///
/// Gain is a live `Arc<AtomicU32>` (an f32's bits), read by the cpal callback
/// on every sample -- unlike [`PreparedTx`]'s FT8 waveform, which bakes its
/// gain into the buffer once and is never touched again. Real gap, found
/// live: a Tune carrier is held for as long as it takes to ride an ATU or a
/// radio's ALC (e.g. an FT-857D, where TX audio level *is* the drive level)
/// or walk a manual tuner through its range -- all three need the level
/// adjustable while the carrier is already keyed, not "stop Tune, change a
/// setting, start Tune again," which would drop the carrier and restart
/// whatever the ATU/tuner was mid-cycle on. [`set_gain`](Self::set_gain)
/// updates it from the engine thread; [`build_tune_output`]'s callback reads
/// it fresh every sample, so a drag takes effect essentially instantly.
pub struct TunePlayback {
    _stream: cpal::Stream,
    gain: Arc<AtomicU32>,
}

impl TunePlayback {
    /// Change the live TX level while this Tune is still keyed. Clamped the
    /// same as the FT8 path's gain (`[0.0, 1.0]`) -- see `apply_gain_and_clip`'s
    /// doc comment for why the callback still clips every sample regardless.
    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// Build and immediately start a continuous steady-tone output stream at
/// `tone_hz`, for antenna tuning (no FT8 message, no auto-release).
/// `gain`/`channel` mirror [`prepare`]'s -- `gain` is just the *initial*
/// value here, live-adjustable afterward via [`TunePlayback::set_gain`].
/// Synthesized directly at the device's own sample rate -- no resampling,
/// unlike the FT8 waveform path, since there's no fixed-rate source to
/// match -- and trimmed to a whole number of tone cycles so the loop point
/// has no phase discontinuity (an audible click every wrap otherwise).
pub fn prepare_tune(
    device_name: Option<&str>,
    tone_hz: f32,
    gain: f32,
    channel: i32,
) -> anyhow::Result<TunePlayback> {
    let device = find_output_device(device_name)
        .ok_or_else(|| anyhow::anyhow!("no output audio device available"))?;
    let supported = device.default_output_config()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();
    let channels = config.channels as usize;
    let device_rate = config.sample_rate.0;

    // Unity-amplitude buffer -- the callback applies `gain` live on every
    // sample instead of it being baked in once here (see TunePlayback's own
    // doc comment). `tune_loop_buffer` is already a bare sine, inherently
    // within [-1.0, 1.0], so there's nothing to clip before gain is applied.
    let buffer = Arc::new(tune_loop_buffer(tone_hz.max(1.0), device_rate));
    let cursor = Arc::new(AtomicUsize::new(0));
    let gain = Arc::new(AtomicU32::new(gain.clamp(0.0, 1.0).to_bits()));

    let err_fn = |e| log::error!("audio output stream error: {e}");
    let stream = match sample_format {
        SampleFormat::F32 => {
            build_tune_output::<f32>(&device, &config, channels, &buffer, &cursor, &gain, channel, err_fn)?
        }
        SampleFormat::I16 => {
            build_tune_output::<i16>(&device, &config, channels, &buffer, &cursor, &gain, channel, err_fn)?
        }
        SampleFormat::U16 => {
            build_tune_output::<u16>(&device, &config, channels, &buffer, &cursor, &gain, channel, err_fn)?
        }
        other => anyhow::bail!("unsupported output sample format: {other:?}"),
    };
    stream.play()?;
    Ok(TunePlayback { _stream: stream, gain })
}

/// A buffer of whole tone cycles at `device_rate`, ~100 ms long: it loops
/// smoothly because it holds an exact integer number of periods of
/// `tone_hz`, so the sample at the end flows straight into the sample at the
/// start with no phase jump.
fn tune_loop_buffer(tone_hz: f32, device_rate: u32) -> Vec<f32> {
    let target_cycles = (tone_hz * 0.1).round().max(1.0);
    let n = ((target_cycles / tone_hz) * device_rate as f32).round().max(1.0) as usize;
    (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * tone_hz * i as f32 / device_rate as f32).sin())
        .collect()
}

/// Single-sample counterpart of `apply_gain_and_clip`, for Tune's per-sample
/// live gain (the whole buffer can't be pre-scaled once, since `gain` can
/// change on any later sample -- see TunePlayback's own doc comment).
/// Same unconditional clamp, same reasoning.
fn scale_and_clip_sample(raw: f32, gain: f32) -> f32 {
    (raw * gain).clamp(-1.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
fn build_tune_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: &Arc<Vec<f32>>,
    cursor: &Arc<AtomicUsize>,
    gain: &Arc<AtomicU32>,
    channel: i32,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let buffer = buffer.clone();
    let cursor = cursor.clone();
    let gain = gain.clone();
    let stream = device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            // Read fresh every callback (not cached across the closure's
            // lifetime) -- this is exactly what makes a live set_gain() call
            // actually reach the DAC. See TunePlayback's own doc comment.
            let g = f32::from_bits(gain.load(Ordering::Relaxed));
            for frame in out.chunks_mut(channels) {
                let idx = cursor.load(Ordering::Relaxed);
                let sample = scale_and_clip_sample(buffer[idx], g);
                cursor.store((idx + 1) % buffer.len(), Ordering::Relaxed);
                write_frame(frame, sample, channel);
            }
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tune_loop_buffer_holds_a_whole_number_of_cycles() {
        // The load-bearing property: a seamless loop needs the buffer to end
        // exactly where a fresh cycle would start, i.e. its length in
        // samples divided by the samples-per-cycle is (very nearly) a whole
        // number -- any fractional remainder is where an audible click would
        // come from.
        for &(tone_hz, device_rate) in &[(1500.0f32, 48_000u32), (200.0, 44_100), (2900.0, 48_000)] {
            let buf = tune_loop_buffer(tone_hz, device_rate);
            let samples_per_cycle = device_rate as f32 / tone_hz;
            let cycles = buf.len() as f32 / samples_per_cycle;
            assert!(
                (cycles - cycles.round()).abs() < 1e-3,
                "tone_hz={tone_hz} device_rate={device_rate}: {} buffered samples is {cycles} cycles, not a whole number",
                buf.len(),
            );
        }
    }

    #[test]
    fn tune_loop_buffer_is_never_empty() {
        // A zero-length buffer would make the cpal callback's `% buffer.len()`
        // divide by zero -- even a pathologically low tone must still
        // produce at least one sample.
        assert!(!tune_loop_buffer(1.0, 8_000).is_empty());
    }

    #[test]
    fn write_frame_fans_to_every_channel_by_default() {
        let mut frame = [0.0f32, 0.0, 0.0];
        write_frame(&mut frame, 0.5, channel_select::BOTH);
        assert_eq!(frame, [0.5, 0.5, 0.5]);
    }

    #[test]
    fn write_frame_confines_to_left_channel_when_requested() {
        let mut frame = [0.0f32, 0.0, 0.0];
        write_frame(&mut frame, 0.5, channel_select::LEFT);
        assert_eq!(frame, [0.5, 0.0, 0.0]);
    }

    #[test]
    fn write_frame_confines_to_right_channel_when_requested() {
        let mut frame = [0.0f32, 0.0, 0.0];
        write_frame(&mut frame, 0.5, channel_select::RIGHT);
        assert_eq!(frame, [0.0, 0.5, 0.0]);
    }

    #[test]
    fn write_frame_channel_selection_is_a_no_op_on_mono() {
        // No second channel to silence -- behaves the same as the fan-out path.
        let mut frame = [0.0f32];
        write_frame(&mut frame, 0.5, channel_select::LEFT);
        assert_eq!(frame, [0.5]);
        write_frame(&mut frame, 0.5, channel_select::RIGHT);
        assert_eq!(frame, [0.5]);
    }

    #[test]
    fn skip_cursor_converts_ms_to_device_samples() {
        // 100 ms at 48 kHz = 4800 samples.
        assert_eq!(skip_cursor(100, 48_000, 1_000_000), 4_800);
        // The normal case: no leading clip → start at sample 0.
        assert_eq!(skip_cursor(0, 48_000, 1_000_000), 0);
        // Negative is treated as zero, never a huge usize.
        assert_eq!(skip_cursor(-500, 48_000, 1_000_000), 0);
    }

    #[test]
    fn skip_cursor_clamps_to_total() {
        // A clip longer than the buffer starts at the end (plays silence), never
        // overruns the buffer.
        assert_eq!(skip_cursor(60_000, 48_000, 1_000), 1_000);
    }

    #[test]
    fn apply_gain_and_clip_clamps_overshoot_at_unity_gain() {
        // Regression: the old `if gain != 1.0` guard skipped the clamp at unity
        // gain, letting post-resample overshoot past ±1.0 reach the DAC. The
        // clamp must run at every gain, including 1.0 (TX level 100%).
        let mut d = vec![1.05_f32, -1.05, 0.5, -0.5, 1.0, -1.0];
        apply_gain_and_clip(&mut d, 1.0);
        assert_eq!(d, vec![1.0, -1.0, 0.5, -0.5, 1.0, -1.0]);
    }

    #[test]
    fn apply_gain_and_clip_scales_then_clamps() {
        let mut d = vec![2.0_f32, 1.0, -2.0, 0.4];
        apply_gain_and_clip(&mut d, 0.5);
        // 0.5*2=1.0, 0.5*1=0.5, 0.5*-2=-1.0, 0.5*0.4=0.2
        assert_eq!(d, vec![1.0, 0.5, -1.0, 0.2]);
    }

    #[test]
    fn apply_gain_and_clip_is_identity_for_in_range_unity_gain() {
        // Happy path: unity gain, already-in-range samples are untouched.
        let mut d = vec![0.0_f32, 0.25, -0.75, 1.0, -1.0];
        let expected = d.clone();
        apply_gain_and_clip(&mut d, 1.0);
        assert_eq!(d, expected);
    }

    #[test]
    fn resampled_full_scale_waveform_is_clamped_at_unity_gain() {
        // End-to-end: resampling a full-scale (±1.0) tone to a device rate
        // overshoots past 1.0; after the output stage at gain == 1.0 nothing
        // exceeds full scale. (Default gain 0.9 masks this, which is why the bug
        // survived — exercise the load-bearing unity-gain case.)
        let n = 12_000usize;
        let mut sig = Vec::with_capacity(n);
        let mut phi = 0.0f32;
        for _ in 0..n {
            phi += 2.0 * std::f32::consts::PI * 1500.0 / 12_000.0;
            sig.push(phi.sin());
        }
        let mut data = resample_buffer(&sig, SAMPLE_RATE as u32, 48_000);
        // Whether the resampler overshoots past full scale is an implementation
        // detail of rubato (filter params / version), so don't hard-fail on it —
        // the load-bearing property is that after the unity-gain output stage
        // nothing exceeds full scale, which holds either way.
        let overshot = data.iter().any(|&x| x.abs() > 1.0);
        apply_gain_and_clip(&mut data, 1.0);
        assert!(data.iter().all(|&x| (-1.0..=1.0).contains(&x)));
        // If this rubato build did overshoot, prove the clamp is what tamed it:
        // the offending samples must now sit pinned exactly at full scale.
        if overshot {
            assert!(
                data.iter().any(|&x| x.abs() == 1.0),
                "overshoot should have been clamped to exactly full scale"
            );
        }
    }

    #[test]
    fn remaining_secs_measures_unplayed_audio() {
        // 48000 samples left at 48 kHz = 1.0 s.
        assert_eq!(remaining_secs(96_000, 48_000, 48_000), 1.0);
        // Past the end clamps to zero rather than going negative.
        assert_eq!(remaining_secs(48_000, 96_000, 48_000), 0.0);
        // Zero device rate can't divide-by-zero.
        assert!(remaining_secs(48_000, 0, 0).is_finite());
    }

    #[test]
    fn scale_and_clip_sample_scales_then_clamps_like_the_buffer_version() {
        // Same contract as apply_gain_and_clip, just one sample at a time --
        // this is what lets Tune's gain change live, mid-carrier, instead of
        // only being baked in once before the stream starts.
        assert_eq!(scale_and_clip_sample(0.5, 1.0), 0.5);
        assert_eq!(scale_and_clip_sample(2.0, 0.5), 1.0);
        assert_eq!(scale_and_clip_sample(-2.0, 0.5), -1.0);
        assert_eq!(scale_and_clip_sample(1.0, 1.05), 1.0); // overshoot still clamps at unity gain
    }

    #[test]
    fn tune_playback_set_gain_is_live_and_clamped() {
        // TunePlayback can't be constructed here without a real output
        // device, so this exercises the same atomic-f32-bits mechanism
        // set_gain uses directly -- the contract that actually matters:
        // the callback's next read sees whatever was last stored, clamped
        // to [0.0, 1.0] regardless of what the caller passed.
        let gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let store = |g: f32| gain.store(g.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        let read = || f32::from_bits(gain.load(Ordering::Relaxed));

        assert_eq!(read(), 1.0);
        store(0.3);
        assert_eq!(read(), 0.3);
        store(-1.0); // a bad caller value clamps, doesn't underflow or panic
        assert_eq!(read(), 0.0);
        store(5.0);
        assert_eq!(read(), 1.0);
    }
}
