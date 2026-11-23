//! The runtime engine: a dedicated thread that owns audio + decoder + QSO state
//! machine + rig, runs the 15 s UTC cycle (ports `UtcTimer`), and turns QSO
//! decisions into encode + PTT + playback + DB writes. Communicates with the
//! Tauri layer over command/event channels so the cpal `Stream`s (which are
//! `!Send` on Windows) never leave this thread.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::adif_log_file;
use crate::audio::{output, AudioInput, SlotAccumulator};
use crate::bands;
use crate::clublog;
use crate::cloudlog;
use crate::db::{Db, QsoRecord};
use crate::dsp::encode;
use crate::dsp::{DecodedMessage, Decoder, SAMPLE_RATE};
use crate::dxcc;
use crate::eqsl;
use crate::hamqth;
use crate::hrdlog;
use crate::logged_state;
use crate::mode::Mode;
use crate::qmx_cat_tones as cat_tones;
use crate::qmx_meters;
use crate::qrz;
use crate::pskreporter_sender::{self, PskReporterSender};
use crate::qso::{QsoEngine, QsoOutcome, QsoStatus};
use crate::rig::{RigConfig, RigConnection};
use crate::tx_protection;
use crate::util;
use crate::wrl;
use crate::us_states;
use crate::wf::{WfConfig, WfProcessor, WfWindow};

// Cycle/TX-timing constants (15s/12.64s-audio FT8, 7.5s/5.04s-audio FT4) now
// live on `Mode` (cycle_ms/audio_slack_ms/tx_latest_ms/decode_at_ms) instead
// of as fixed consts here -- see mode.rs. PTT delay is rig-settle time, not
// FT8/FT4-specific, so it isn't on `Mode` either -- it's now a real,
// configurable `Engine::ptt_delay_ms` field (Settings -> Advanced's "PTT
// Delay"), not a fixed const.
// Run-loop tick while a QMX CAT-tone transmission is in flight (see
// `service_cat_tx`): short enough that each symbol goes out close to on time
// despite being polled rather than slept-to directly (no separate timer
// thread touches the rig -- see CatToneTx's doc comment for why), long
// enough not to burn the CPU busy-polling for the whole transmission.
const TX_CAT_TICK_MS: u64 = 10;
const DEFAULT_TX_AUDIO_HZ: i32 = 1_500;
/// How often a rig that drops CAT PTT on its own (the Xiegu G106 releases after
/// ~2 s) gets its PTT-on frame refreshed while keyed. Measured on a monotonic
/// clock so an NTP correction mid-over can't stall it past the rig's timeout.
const PTT_KEEPALIVE_INTERVAL: Duration = Duration::from_millis(1_000);
// Usable TX audio-offset passband (Hz). Real FT8 audio tones live well inside
// this; both the base-freq setter and an inbound WSJT-X Reply's requested `df`
// validate against it.
const MIN_TX_AUDIO_HZ: i32 = 200;
const MAX_TX_AUDIO_HZ: i32 = 3_000;
// Default TX output level (0.0–1.0). Slightly below full scale so a fresh
// install doesn't overdrive the soundcard/ALC before the operator sets it.
const DEFAULT_TX_GAIN: f32 = 0.9;

/// Clamp a requested TX gain into the valid 0.0–1.0 range (full scale).
///
/// Non-finite input falls back to the default rather than being clamped:
/// `f32::clamp` propagates NaN, and the result is both stored in the engine
/// and persisted to config, where `"NaN".parse::<f32>()` succeeds — so a
/// single NaN would poison the gain across restarts with no way back short
/// of editing the database.
fn clamp_tx_gain(g: f32) -> f32 {
    if !g.is_finite() {
        return DEFAULT_TX_GAIN;
    }
    g.clamp(0.0, 1.0)
}

/// Which gain actually drives Tune right now: the independent `tune_gain`
/// when that override is on, otherwise the normal `tx_gain` -- the same
/// choice `start_tune` makes when first keying, and `apply_live_tune_gain`
/// makes again on every live adjustment, so an already-running carrier
/// always tracks whichever slider the operator is actually on.
fn effective_tune_gain(tune_level_independent: bool, tune_gain: f32, tx_gain: f32) -> f32 {
    if tune_level_independent { tune_gain } else { tx_gain }
}

// Default RX input gain: unity (no change) -- a fresh install shouldn't
// alter whatever level the operator's soundcard/interface already provides.
const DEFAULT_RX_GAIN: f32 = 1.0;

/// Clamp a requested RX gain into 0.0–1.0, same range as TX gain. Widened to
/// 0.0-8.0 during testing to check the control had real effect (confirmed:
/// a clip warning at 800%, silence at 0%) -- but adjustment past 100% wasn't
/// doing anything practically useful, so finalized at 0-100% for finer
/// control resolution across the range that actually matters.
/// Non-finite input falls back to the default, for the same reason as
/// `clamp_tx_gain` — and worse here, since a NaN gain multiplies every
/// captured sample into NaN and silently kills RX entirely.
fn clamp_rx_gain(g: f32) -> f32 {
    if !g.is_finite() {
        return DEFAULT_RX_GAIN;
    }
    g.clamp(0.0, 1.0)
}

/// Matches Android's own `RX_LATENCY_MAX_MS` bound exactly.
const RX_LATENCY_MAX_MS: i64 = 3000;

fn clamp_rx_latency_ms(ms: i64) -> i64 {
    ms.clamp(0, RX_LATENCY_MAX_MS)
}

/// Format a wattage for the `powerWatts` config value -- ported from the
/// Android fork's `GeneralVariables.formatWatts` exactly: rounded to tenths,
/// with a trailing ".0" dropped (`4.5`, `12.4`, `100` not `100.0`). Keeping
/// the identical format (not just the identical config key) matters for a
/// future settings-import between the two apps.
fn format_watts(watts: f32) -> String {
    let tenths = (watts * 10.0).round() / 10.0;
    if tenths == tenths.trunc() {
        format!("{}", tenths as i64)
    } else {
        format!("{tenths:.1}")
    }
}

/// The TX audio channel to use for a newly selected rig: the operator's
/// sticky override if they've ever set one (`txAudioChannel`), else the
/// QMX-based auto-default (`LEFT` for a detected QMX, `BOTH` otherwise) --
/// see [`crate::rig::needs_left_channel_tx_audio`]. Pure and unit-tested
/// directly, separately from `select_rig` (which has a live rig connection
/// to make and no reason to be exercised just for this).
fn resolve_tx_audio_channel(override_: Option<i32>, backend: crate::rig::RigBackend, hamlib_model: i32) -> i32 {
    override_.unwrap_or_else(|| {
        if crate::rig::needs_left_channel_tx_audio(backend, hamlib_model) {
            crate::audio::channel_select::LEFT
        } else {
            crate::audio::channel_select::BOTH
        }
    })
}

/// Whether to turn QMX CAT tones on and persist that choice for a newly
/// selected rig -- ported from Android's own `shouldDefaultQmxCatTonesOn`
/// exactly, which is a **one-time latch**, not a per-connection auto-detect
/// like [`resolve_tx_audio_channel`]: the real Android description reads
/// "Turned on automatically the first time a QMX connects." The first time
/// ever a QMX is selected with no saved `qmxCatTones` row, this turns on and
/// *writes* the config row (see `select_rig`'s use of this); from then on
/// `cat_tones_config_key_exists` is permanently true, so this never fires
/// again even across switching away from and back to a QMX, and an explicit
/// operator "off" is never overridden either.
fn should_default_qmx_cat_tones_on(rig_is_qmx: bool, cat_tones_config_key_exists: bool) -> bool {
    rig_is_qmx && !cat_tones_config_key_exists
}

/// The TX audio offset (Hz) to adopt for an inbound WSJT-X Reply's requested
/// `df`, or `None` to keep the current offset. Bounds the untrusted UDP value to
/// the usable audio passband; a `0` (unspecified) or out-of-band `df` is ignored
/// so a malformed/garbled datagram can't push the TX tone off the band. Mirrors
/// the Android/iOS ports, which likewise honor a plausible `df` and drop the rest.
fn reply_tx_audio_hz(delta_freq: u32) -> Option<i32> {
    let df = i32::try_from(delta_freq).ok()?;
    (MIN_TX_AUDIO_HZ..=MAX_TX_AUDIO_HZ).contains(&df).then_some(df)
}

/// The core keying gate `maybe_transmit` enforces, factored out so it is pure and
/// unit-testable and shared with the run loop's boundary trigger. We may key iff:
///   * a QSO is active,
///   * a locked TX parity (set when answering a CQ) matches this slot's parity —
///     `None` leaves us eligible in any slot and locks on the first transmission,
///   * we have not already transmitted in this slot, and
///   * we are early enough in the cycle to fit the full waveform before the next
///     boundary (`into_cycle_ms <= tx_latest_ms`, the caller's current mode's
///     `Mode::tx_latest_ms()`); starting later clips the leading Costas array —
///     audible but undecodable.
fn tx_slot_eligible(
    active: bool,
    tx_parity: Option<i64>,
    slot_id: i64,
    txed_slot: i64,
    into_cycle_ms: i64,
    tx_latest_ms: i64,
) -> bool {
    if !active {
        return false;
    }
    if let Some(p) = tx_parity {
        if p != slot_id.rem_euclid(2) {
            return false;
        }
    }
    if txed_slot == slot_id {
        return false;
    }
    into_cycle_ms <= tx_latest_ms
}

/// Whether any decode batch is still outstanding, gating the boundary TX trigger.
/// Derived from the pending-decode *count* (not a single in-flight bool) so the
/// gate stays closed until every enqueued slot has returned — see
/// [`Engine::pending_decodes`].
fn awaiting_any_decode(pending_decodes: usize) -> bool {
    pending_decodes > 0
}

/// Whether the run loop should emit a new waterfall row this tick. The
/// display freezes while transmitting (covers both the audio and CAT-tone TX
/// paths, since both key `ptt`): during a CAT-tone over there's no audio
/// signal at all to show, and during an audio over the mic/line input is
/// usually TX leakage or silence either way, not anything useful to scroll
/// into view.
fn waterfall_row_due(decoding: bool, ptt: bool) -> bool {
    decoding && !ptt
}

/// Whether the run loop should key a queued transmission at this tick. Unlike an
/// operator command (which keys immediately), the per-tick boundary trigger waits
/// until the current slot's decodes have been processed (`!awaiting_decode`) so it
/// never keys a stale message ahead of a slow decode; once they are in,
/// `handle_decoded` has set the fresh `tx_message` this fires on. This is what lets
/// a reply computed early in the previous cycle (fast CPU) still go out early in
/// its slot — `maybe_transmit` is otherwise only reached on decode arrival, which
/// on a quick decode lands mid-slot, past the TX window, dropping the reply.
///
/// "Early in its slot" is the whole TX window, not a hard boundary: the gate only
/// requires `into_cycle_ms <= tx_latest_ms` (via [`tx_slot_eligible`]). In practice
/// the run loop ticks fast, so the first eligible tick after the decodes settle is
/// near the top of the slot — but a reply that becomes eligible a little later in
/// the window is still keyed rather than dropped.
///
/// The trigger is part of the decode-driven RX/TX cycle, so it is dormant while the
/// decoder is stopped (`decoding == false`) — matching the sibling per-tick actions
/// (the decode trigger and the waterfall row, both already gated on `self.decoding`).
/// Without this a QSO/CQ left `active` when the operator stops decoding would keep
/// auto-keying the rig every eligible slot with the receiver off — the radio would
/// transmit unattended, since `pending_decodes` drops to 0 on `StopDecode` so the
/// awaiting gate no longer holds it back.
/// Whether the run loop should hand this tick's slot audio off to the
/// decoder thread. Separate from `decoding` itself: `suspended` is the
/// narrower, temporary gate set by `EngineCommand::SetDecodeSuspended`
/// (currently only during a bulk ADIF import) that stops feeding the
/// CPU-heavy decode step without touching capture, the waterfall, or
/// anything else `decoding` gates -- see that command's doc comment.
fn decode_dispatch_due(
    decoding: bool,
    suspended: bool,
    rx_slot_id: i64,
    last_decoded_slot: i64,
    into_rx_cycle_ms: i64,
    decode_at_ms: i64,
) -> bool {
    decoding
        && !suspended
        && rx_slot_id != last_decoded_slot
        && into_rx_cycle_ms >= decode_at_ms
}

/// Shift `into_rx_cycle_ms` back by the configured RX audio latency, for
/// both the early-decode trigger and the slot-extraction math -- ported
/// from Android's `rxLatencyMs`/`FT8SignalListener.setRxLatencyMs`
/// ("Each slot's recording starts this much after the boundary, so decode
/// DTs read ~0 and the early pass doesn't cut off the end of every
/// signal"). A real, measured, QMX-specific USB-audio buffering delay
/// (confirmed in Android's own code comment: "~1 s with the QMX on the
/// motorola"), not a bug in this app's own timing math -- the decoder's
/// own symbol-sync detection is untouched; this only changes which
/// captured samples get treated as "this slot's audio so far" and when
/// that's considered a complete-enough waveform to decode early. Clamped
/// at 0 (a configured value can't make the decoder see into the future)
/// -- the upstream clamp to Android's own 0-3000ms range happens where the
/// setting is applied/persisted, same as every other bounded setting in
/// this file.
fn rx_latency_adjusted_into_cycle_ms(into_rx_cycle_ms: i64, rx_latency_ms: i64) -> i64 {
    (into_rx_cycle_ms - rx_latency_ms).max(0)
}

fn boundary_tx_ready(
    decoding: bool,
    active: bool,
    awaiting_decode: bool,
    tx_parity: Option<i64>,
    slot_id: i64,
    txed_slot: i64,
    into_cycle_ms: i64,
    tx_latest_ms: i64,
) -> bool {
    decoding
        && !awaiting_decode
        && tx_slot_eligible(active, tx_parity, slot_id, txed_slot, into_cycle_ms, tx_latest_ms)
}
// Live-waterfall FFT parameters (window/size/averaging + display constants)
// live in `crate::wf` and are runtime-configurable via SetWaterfallConfig.
// Input RMS at/below this (dBFS) counts as silence — no audio reaching the app.
// Real RX audio through a soundcard/DAX sits well above this even on a quiet band;
// a fully-routed-but-silent virtual device floors near -100 dBFS.
const SILENCE_DBFS: f32 = -75.0;

/// RMS level of a mono buffer in dBFS (0 dB = full scale). Empty or all-zero
/// input returns a large negative number rather than -inf, so the meter and the
/// silence test stay finite.
fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    (20.0 * (rms.max(1e-12)).log10()).max(-120.0) as f32
}

// RX input-level classification, ported from Android's AudioInputLevel.java
// (issue #405 there): the waterfall strip's RX meter uses these exact
// thresholds so the two apps' operators get the same gain-setting guidance.

/// Peak at/above this linear magnitude counts as clipping (~ -0.13 dBFS) --
/// matches `AudioInputLevel.CLIP_PEAK`.
const CLIP_PEAK: f32 = 0.985;
/// Peak dBFS at/above this is "hot" (clipping risk on transients) -- matches
/// `AudioInputLevel.HIGH_PEAK_DBFS`.
const HIGH_PEAK_DBFS: f32 = -3.0;
/// RMS dBFS below this (but above `SILENCE_DBFS`) is too quiet for reliable
/// weak-signal decode -- matches `AudioInputLevel.LOW_RMS_DBFS`.
const LOW_RMS_DBFS: f32 = -45.0;

/// Coarse RX input-level health, matching Android's `AudioInputLevel.Status`
/// -- drives the waterfall strip's RX meter color and status word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputLevelStatus {
    /// Essentially no input -- disconnected cable, wrong source, or muted radio.
    Silent,
    /// Audible but too quiet; weak signals will sit below the decode threshold.
    Low,
    /// In the healthy window -- what the operator should aim for.
    Good,
    /// Hot: peaks are close to full scale, raise nothing further.
    High,
    /// Peaks pinned at full scale; clipping distortion will block decodes.
    Clipping,
}

/// Largest sample magnitude in a mono buffer, linear (0..~1). Companion to
/// `rms_dbfs` -- together they feed `classify_input_level`.
fn peak_linear(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

/// RMS magnitude of a mono buffer, linear (0..~1) -- the same computation
/// `rms_dbfs` does internally, exposed separately for the meter bar's fill
/// fraction (which wants the linear value, not dBFS).
fn rms_linear(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Linear magnitude (>=0) to dBFS, with the same -120 floor as `rms_dbfs`.
fn to_dbfs(linear: f32) -> f32 {
    (20.0 * linear.max(1e-12).log10()).max(-120.0)
}

/// Classify a peak/RMS reading into a coarse status -- matches
/// `AudioInputLevel.classify` exactly: clipping is judged from the peak
/// (a lone hot transient should flag red even if the RMS looks fine),
/// "too quiet" from the RMS (tracks noise/signal energy, not a lone dip).
fn classify_input_level(peak: f32, peak_dbfs: f32, rms_dbfs: f32) -> InputLevelStatus {
    if peak >= CLIP_PEAK {
        InputLevelStatus::Clipping
    } else if peak_dbfs >= HIGH_PEAK_DBFS {
        InputLevelStatus::High
    } else if rms_dbfs < SILENCE_DBFS {
        InputLevelStatus::Silent
    } else if rms_dbfs < LOW_RMS_DBFS {
        InputLevelStatus::Low
    } else {
        InputLevelStatus::Good
    }
}

/// True once per cycle, on the first waterfall row at/after a UTC slot boundary
/// (15 s FT8, 7.5 s FT4 -- `cycle_ms`, the caller's current mode's
/// `Mode::cycle_ms()`), advancing `last_slot`. The decoder and waterfall use
/// this same fixed clock.
fn wf_boundary_row(now_ms: i64, last_slot: &mut i64, cycle_ms: i64) -> bool {
    let slot = now_ms.div_euclid(cycle_ms);
    if slot != *last_slot {
        *last_slot = slot;
        true
    } else {
        false
    }
}

/// The on-air text of a decode: the raw decoded line if we have it, else a
/// reconstruction from the parsed fields. Shared by the UI publish and the
/// WSJT-X UDP Decode broadcast so both show identical message text.
fn decode_text(m: &DecodedMessage) -> String {
    if m.raw_text.is_empty() {
        format!("{} {} {}", m.call_to, m.call_from, m.extra)
    } else {
        m.raw_text.clone()
    }
}

// --- messages crossing the channel boundary --------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AnswerArgs {
    pub call_from: String,
    #[serde(default)]
    pub grid: String,
    #[serde(default)]
    pub snr: i32,
    /// Audio offset (Hz) to answer on — WSJT-X's `df`, set by an inbound UDP
    /// Reply request. `0` (the default, and what the desktop UI's own "click to
    /// answer" sends) means "keep the current TX offset".
    #[serde(default)]
    pub delta_freq: u32,
    /// The target's own decoded-message timestamp (`UiMessage.utc_ms` /
    /// `SlotMessage.utcMs` on the frontend) -- lets us derive "opposite the
    /// DX" from the slot HE actually transmitted in, rather than whatever
    /// slot we happen to be in at click time. The two usually agree, but
    /// decode results land ~13s into a 15s slot, leaving the operator only
    /// a couple of seconds to react before the slot boundary; if the click
    /// (and this command's arrival) lands just past it, `self.cur_parity()`
    /// would be one slot later than his CQ -- i.e. the SAME parity as him,
    /// not opposite -- locking our whole QSO onto the wrong slot. Confirmed
    /// live: "my first call to him was in the correct slot but every
    /// transmission after that was in the wrong slot" / "he is transmitting
    /// in an odd slot but so am i". `None` (any caller that doesn't have a
    /// decode timestamp handy, e.g. a future UDP-driven Reply) falls back to
    /// the old click-time behavior.
    #[serde(default)]
    pub msg_utc_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum EngineCommand {
    StartDecode,
    StopDecode,
    /// Lightweight, UI-invisible pause on handing slots to the decoder
    /// thread -- *not* the same as `StopDecode`. Used only to free up CPU
    /// for a bulk ADIF import on CPU-constrained hardware (confirmed live:
    /// a 54,554-record QRZ export took 10+ minutes inside the running app
    /// but 34s in isolation with the decoder idle, on the same code/data).
    /// Unlike `StopDecode`, this does not drop `self.input` (audio capture
    /// keeps running), does not clear the UDP replay cache or notify
    /// companion apps, and does not touch `self.decoding` -- the waterfall
    /// keeps scrolling and the RX meter keeps reading, only the actual
    /// per-slot decode dispatch (`decode_dispatch_due`) is skipped. Sent by
    /// `import_adif` (true before the import, false after, via a guard that
    /// resumes even on error).
    SetDecodeSuspended(bool),
    /// Settings -> Advanced's "Decode Depth" toggle -- see `decode_deep`'s
    /// own doc comment. Re-spawns the decoder thread (same mechanism
    /// `switch_mode` already uses) since there's no live mid-stream config
    /// channel into it.
    SetDecodeDepth(bool),
    SetStation { call: String, grid: String },
    SetBand(u64),
    SetBaseFreq(i32),
    /// TX output level, 0.0–1.0 (drive into the soundcard/USB audio path).
    SetTxGain(f32),
    /// RX input gain, 0.0–1.0 (post-ADC software trim, applied live without
    /// restarting capture -- some bands are noisier than others). See
    /// `clamp_rx_gain` for why the range stops at unity.
    SetRxGain(f32),
    SetInputDevice(Option<String>),
    SetOutputDevice(Option<String>),
    /// Explicit RX channel-selection override (see
    /// [`crate::audio::channel_select`]), persisted under Android's own
    /// `rxAudioChannel` config key. Unlike TX, there is no QMX-based
    /// auto-default here (Android has none either) -- this always takes the
    /// value as given. Restarts capture if already decoding, same as
    /// `SetInputDevice`.
    SetRxAudioChannel(i32),
    /// Explicit TX channel-selection override (`txAudioChannel`), matching
    /// Android's own plain 3-way picker (no "reset to auto" option -- once
    /// set, it's just a persisted preference like any other). Applies
    /// immediately to the next transmission; does not require a rig
    /// reconnect. The *unset* (`None`/key-absent) state that falls back to
    /// the QMX auto-default is reached only by never calling this at all
    /// (see `tx_audio_channel_override`'s own doc comment) -- there is
    /// deliberately no "clear" command, since Android's UI has no such
    /// option either.
    SetTxAudioChannel(i32),
    /// Explicit QMX-CAT-tones override (`qmxCatTones`), matching Android's
    /// own plain boolean toggle -- same "no clear command" shape as
    /// `SetTxAudioChannel` above.
    SetQmxCatTones(bool),
    SelectRig(RigConfig),
    /// Drop the live rig connection for this session (CAT/PTT released). Leaves
    /// the saved rig config intact so Connect — or the next launch — reconnects.
    DisconnectRig,
    StartCq,
    Answer(AnswerArgs),
    /// Operator tapped a caller-queue chip -- see `qso.rs`'s `QueuedCaller`
    /// doc comment and `handle`'s own `AnswerQueuedCaller` arm.
    AnswerQueuedCaller { callsign: String },
    /// Operator manually selects which QSO message to transmit next.
    SetStage(crate::qso::TxStage),
    /// Operator's preferred TX slot parity for the *next* CQ/QSO start (0 =
    /// Even, 1 = Odd; `None` = no preference) -- matches Android's
    /// `TxPeriodControl`. Does not affect an already-locked in-progress QSO.
    SetTxPeriod(Option<i64>),
    /// Switch operating mode (FT8/FT4) -- matches Android's Band & Mode mode
    /// toggle. Stops any in-flight TX/QSO/tune first (same precaution as
    /// `DisconnectRig`): a mode's cycle length and tone encoding are baked
    /// into an in-flight transmission the moment it starts, so switching
    /// mid-transmission would desync the rig from what's actually being sent.
    SetMode(crate::mode::Mode),
    StopTx,
    /// Transmit a steady carrier at the TX offset for antenna tuning, with no
    /// FT8 message involved — see [`Engine::start_tune`].
    StartTune,
    /// Stop an in-flight tune (or any other transmission — shares
    /// [`Engine::finalize_tx`] with Stop TX).
    StopTune,
    FreeText(String),
    /// Re-emit the current rig + TX status. Lets a freshly-loaded UI sync up with
    /// state set before its event listener existed (e.g. the startup auto-reconnect).
    RefreshStatus,
    /// Apply a fresh NTP clock offset (ms to add to local time to get true UTC).
    SetClockOffset(i64),
    /// Kick off an immediate one-shot NTP re-sync (manual "Resync" button).
    ResyncTime,
    /// Apply new live-waterfall FFT parameters (developer knobs, issue #428).
    SetWaterfallConfig(WfConfig),
    /// "Width" on the waterfall strip -- the displayed audio span in Hz,
    /// merged into the current `WfConfig` so the operator can change it from
    /// either the strip's own control or Settings without needing to know
    /// the other waterfall knobs (unlike `SetWaterfallConfig`, which takes a
    /// complete config). Matches Android's `GeneralVariables.setSpectrumWidth`.
    SetWfDisplayHz(f32),
    /// "NR" on the waterfall strip -- same merge-into-current-config
    /// treatment as `SetWfDisplayHz`. Matches Android's `deNoise` toggle.
    SetWfDenoise(bool),
    /// Apply new WSJT-X UDP settings (enable/host/port/accept-requests). Rebinds
    /// the socket + listener live so the Settings screen takes effect at once.
    SetUdpConfig(crate::udp::UdpConfig),
    /// Re-broadcast this session's decodes over UDP (inbound WSJT-X Replay).
    UdpReplay,
    /// Settings -> Transmission -> TX Protection's SWR Halt toggle.
    SetSwrHaltEnabled(bool),
    /// SWR Halt's threshold, a plain ratio (e.g. `3.0`) -- see
    /// `tx_protection`'s module doc for why this isn't Android's normalized
    /// 0-255 scale.
    SetSwrHaltThreshold(f32),
    /// Operator dismisses the SWR lockout banner, allowing TX again --
    /// matches Android's `MeterProtectionController.clearSwrLockout`.
    ClearSwrLockout,
    /// Settings -> Transmission -> Tune's max-on-seconds cap.
    SetTuneMaxOnSeconds(i64),
    /// Settings -> Transmission -> Tune's method picker (Automatic/Internal/
    /// Low power tone) -- see `tx_protection::decide_tune_action`.
    SetTuneMethod(i32),
    /// Settings -> Transmission -> Tune's "independent tune level" toggle.
    SetTuneLevelIndependent(bool),
    /// The independent tune level itself (0.0-1.0, same scale as `tx_gain`).
    SetTuneLevel(f32),
    /// Settings -> Transmission's "TX Watchdog" ("Stop After" N unanswered
    /// attempts) -- Android's `noReplyLimit`, 0 = unlimited.
    SetTxWatchdog(u32),
    /// Settings -> Transmission's "Max 73 Sends" -- Android's `max73Sends`,
    /// 0 = Auto/off.
    SetMax73Sends(u32),
    /// Settings -> Transmission's "CQ Watchdog" -- genuinely new, no Android
    /// equivalent (see `QsoEngine::cq_no_reply_limit`'s doc comment). 0 =
    /// unlimited, the deliberate default (unattended/"robot" CQ operation).
    SetCqWatchdog(u32),
    /// Settings -> Transmission's "CQ Answer Attempts" -- genuinely new, no
    /// Android equivalent (see `QsoEngine::cq_answer_limit`'s doc comment).
    /// 0 = unconfigured, defers to "TX Watchdog".
    SetCqAnswerLimit(u32),
    /// Settings -> Auto-Sequence's "Auto-CQ after QSO" -- see `Engine::new`'s
    /// doc comment on why desktop's *default* differs from Android's own.
    SetAutoCqAfterQso(bool),
    /// Settings -> Transmission's "Clear decodes on band/mode change".
    SetClearOnBandModeChange(bool),
    /// RX audio capture latency compensation (ms) -- see
    /// `rx_latency_adjusted_into_cycle_ms`'s doc comment. Clamped to
    /// Android's own 0-3000ms range.
    SetRxLatencyMs(i64),
    /// Settings -> Advanced's "PTT Delay" (ms) -- Android's own `pttDelay`.
    /// Feeds both the real PTT-settle sleep and `Mode::tx_latest_ms`'s
    /// eligibility window; see `tx_protection::clamp_ptt_delay_ms`.
    SetPttDelayMs(i64),
    /// Settings -> Advanced's "Late-start Tolerance" (ms) -- see
    /// `tx_protection::clamp_late_start_tolerance_ms`'s doc comment for why
    /// desktop's default (`0`) deliberately diverges from Android's (2000).
    SetLateStartToleranceMs(i64),
    Shutdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiMessage {
    pub utc_ms: i64,
    pub call_to: String,
    pub call_from: String,
    pub extra: String,
    pub grid: String,
    pub snr: i32,
    pub freq_hz: f32,
    pub time_sec: f32,
    pub text: String,
    pub is_cq: bool,
    pub to_me: bool,
    /// The sender's DXCC entity prefix (`crate::dxcc`), or `None` for an
    /// unresolvable callsign -- computed once at decode time, matching
    /// Android's `CallsignDatabase.getMessagesLocation` stamping `fromDxcc`
    /// etc. onto each message rather than re-deriving it per render.
    pub dxcc_prefix: Option<String>,
    /// The sender's CQ zone, or `None` alongside `dxcc_prefix`.
    pub cq_zone: Option<i64>,
    /// The sender's US state, derived from its grid square (no callsign
    /// fallback here -- that richer logic is for worked-set aggregation
    /// from the log, see `crate::us_states::worked_state_for`; a live
    /// decode only ever has a grid to go on, matching Android's
    /// `CallsignDatabase` live-decode path).
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleTick {
    pub utc_ms: i64,
    pub sequential: i64,
    pub ms_into_cycle: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TxStateEvent {
    pub transmitting: bool,
    pub message: Option<String>,
    pub status: QsoStatus,
    /// Whether a TUNE (steady carrier, no FT8 message) is in flight — see
    /// [`Engine::start_tune`]. Drives the Tune button's on/off look.
    pub tuning: bool,
    /// Operator's preferred TX slot parity for the next CQ/QSO start (0 =
    /// Even, 1 = Odd, `None` = no preference) -- see `EngineCommand::SetTxPeriod`.
    pub preferred_tx_parity: Option<i64>,
    /// SWR Halt's lockout -- see `EngineCommand::ClearSwrLockout`. True
    /// blocks starting any new TX/Tune until explicitly cleared.
    pub swr_lockout: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RigStatusEvent {
    pub connected: bool,
    pub model: String,
    pub frequency_hz: u64,
    pub ptt: bool,
    /// The TX-audio-channel/CAT-tones values actually in effect right now
    /// (post `resolve_tx_audio_channel`/`resolve_qmx_cat_tones`) -- NOT the
    /// raw `txAudioChannel`/`qmxCatTones` config, which stays unset while an
    /// operator is relying on auto-detect. The Settings UI reads these (not
    /// the config key) so its checkbox/dropdown reflects what a detected QMX
    /// is actually doing, instead of always showing the override-unset
    /// default until the operator explicitly overrides it.
    pub tx_via_cat_tones: bool,
    pub tx_audio_channel: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClockSyncEvent {
    /// ms added to the local clock to get true UTC (negative = local is fast).
    pub offset_ms: i64,
    /// true once an NTP sync has succeeded this session (or a saved offset was restored).
    pub synced: bool,
    /// Retained for IPC compatibility; always zero since unsafe RX auto-calibration was disabled.
    pub rx_offset_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterfallFrame {
    /// Magnitude bins (0..=255) for the displayed audio band of one slot,
    /// downsampled server-side. `cols` bins per row, `rows` time rows.
    pub bins: Vec<u8>,
    pub rows: usize,
    pub cols: usize,
    pub hz_per_col: f32,
}

/// A single live spectrum row (emitted ~10×/s) for the scrolling waterfall.
#[derive(Debug, Clone, Serialize)]
pub struct WaterfallRow {
    pub bins: Vec<u8>,
    pub hz_per_col: f32,
    /// True on the first row at/after a UTC 15 s cycle boundary. The UI draws a grid line there so
    /// the spectrum lines up with decode DT instead of reading ahead of it.
    pub boundary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EngineEvent {
    Decoded(Vec<UiMessage>),
    Cycle(CycleTick),
    TxState(TxStateEvent),
    RigStatus(RigStatusEvent),
    ClockSync(ClockSyncEvent),
    QsoCompleted(QsoRecord),
    Waterfall(WaterfallFrame),
    WaterfallRow(WaterfallRow),
    /// Periodic RX input level (~1/s while decoding) so the UI can show a meter
    /// and warn when the captured audio is silent (e.g. the source/DAX channel
    /// isn't routed) instead of leaving a mysteriously blank waterfall.
    InputLevel(InputLevelEvent),
    /// SWR/output-power readings polled over CAT while transmitting (QMX
    /// only — see [`crate::rig::supports_meter_query`]). Either field is
    /// `None` when that particular query didn't return a valid reading.
    Meter(MeterEvent),
    /// Settings -> Transmission's "Clear decodes on band/mode change": the
    /// UI should wipe its own decode-row list, same as the operator
    /// pressing the manual "Clear the decode list" button.
    ClearDecodeList,
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct InputLevelEvent {
    /// RMS level of the recent capture in dBFS (≈ -100 for digital silence).
    pub db: f32,
    /// Peak sample magnitude of the recent capture, linear (0..~1) -- feeds
    /// the RX meter's peak tick and percent readout, matching Android's
    /// `InputLevelIndicator` (`peak`/`inputLevelPercent`).
    pub peak: f32,
    /// RMS sample magnitude, linear (0..~1) -- feeds the RX meter's bar fill
    /// (Android's `rms`/`inputLevelFraction`).
    pub rms: f32,
    /// Coarse health classification (matches Android's `AudioInputLevel.Status`)
    /// -- drives the meter's color and status word ("Low"/"Good"/"High"/"Clip").
    /// `Silent` is also the "no audio reaching the app" condition the old
    /// `silent: bool` field used to flag on its own.
    pub status: InputLevelStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeterEvent {
    pub swr_ratio: Option<f32>,
    pub power_watts: Option<f32>,
}

/// Thread-safe handle for sending commands to the engine. Wraps the `Sender` in
/// a mutex so it satisfies Tauri managed-state's `Sync` bound regardless of the
/// `mpsc::Sender` auto-trait situation.
pub struct EngineHandle {
    tx: parking_lot::Mutex<Sender<EngineCommand>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: EngineCommand) {
        let _ = self.tx.lock().send(cmd);
    }
}

/// Spawn the engine thread. Returns a command handle and the event receiver.
pub fn spawn(db: Arc<Db>) -> (EngineHandle, Receiver<EngineEvent>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (evt_tx, evt_rx) = std::sync::mpsc::channel();
    // Background NTP time sync: corrects the app's clock without admin rights or
    // external software. Feeds offsets back through the command channel.
    crate::timesync::spawn_loop(cmd_tx.clone(), db.clone());
    let engine_cmd_tx = cmd_tx.clone();
    std::thread::Builder::new()
        .name("ft8af-engine".into())
        .spawn(move || {
            let mut engine = Engine::new(db, evt_tx, engine_cmd_tx);
            engine.run(cmd_rx);
        })
        .expect("spawn engine thread");
    (EngineHandle { tx: parking_lot::Mutex::new(cmd_tx) }, evt_rx)
}

/// An in-flight QMX CAT-tone transmission: the 79 FT8 channel symbols plus
/// enough state to drive them out one `TA` command per symbol period,
/// serviced by [`Engine::service_cat_tx`] from the run loop's own tick
/// rather than a dedicated timer thread.
///
/// Why no separate thread (unlike the audio path's cpal stream, which does
/// run on its own realtime thread): `Hamlib`'s `RIG*` and loaded function
/// pointers are only ever touched from the engine thread (see the
/// `unsafe impl Send for Hamlib` comment in rig.rs) -- nothing in rig.rs
/// synchronizes concurrent access. Driving the CAT sends from a second
/// thread would need `self.rig` behind a lock shared across threads, a much
/// larger change for a feature whose timing tolerance doesn't actually need
/// it: FT8 tone spacing is 6.25 Hz over a 160 ms symbol period, and a
/// `TX_CAT_TICK_MS`-granularity poll keeps symbol lateness well under what
/// meaningfully hurts decodability (Android's own sleep-and-spin
/// implementation measured 0.3 ms worst case; polling at 10 ms trades a
/// little of that precision for keeping the existing single-threaded-rig
/// invariant intact).
///
/// `Copy` so `service_cat_tx` can take a plain owned local out of
/// `Engine::tx_cat`, mutate it, and make `self.rig`/`self.emit` calls
/// in between without fighting the borrow checker over a live reference
/// into `self`.
#[derive(Debug, Clone, Copy)]
struct CatToneTx {
    /// Fixed-size at `encode::FT4_NN` (105), the larger of the two supported
    /// modes' tone counts (FT8 uses 79) -- `len` below marks how many of
    /// these are actually in use for this transmission's mode. A fixed array
    /// (rather than a `Vec`) keeps `CatToneTx` `Copy`, see the struct's own
    /// doc comment for why that matters.
    tones: [u8; encode::FT4_NN],
    /// Number of `tones` entries actually populated (79 FT8, 105 FT4).
    len: usize,
    /// This transmission's mode's GFSK symbol period in seconds (`Mode::symbol_period()`,
    /// captured at TX start since `self.mode` could in principle change while
    /// this struct is still in flight) -- drives `service_cat_tx`'s per-symbol
    /// timing and `cat_tones::tone_hz`'s frequency spacing.
    symbol_period: f32,
    /// TX audio offset in Hz (`tx_audio_hz`, same number the audio path
    /// centers its waveform on) -- tone 0's CAT frequency.
    base_hz: f64,
    /// When this transmission's clock reference started (right after PTT
    /// settled) -- `Instant::now().elapsed()` since this maps directly to
    /// symbol position, the same way the audio path treats "now" as the
    /// start of whatever's left of the waveform after a late-start clip.
    start: Instant,
    /// Index of the next not-yet-sent symbol into `tones`.
    sent_through: usize,
    /// Symbols below this index were clipped by a late start (see
    /// [`cat_tones::symbols_to_skip`]) and are never sent -- `sent_through`
    /// starts here, not at 0.
    skip_until: usize,
}

/// A live TUNE transmission (steady carrier at the TX offset, for antenna
/// tuning — no FT8 message, no auto-release). One or the other depending on
/// [`Engine::tx_via_cat_tones`], same split as a normal over.
enum TuneHandle {
    /// Looping steady-tone cpal stream; dropping it halts the carrier.
    /// `apply_live_tune_gain` reads it to push a live TX-level change into
    /// the already-running carrier (see that method's own doc comment).
    Audio(output::TunePlayback),
    /// A single CAT `TA` command already sent; nothing more to do until
    /// key-up (`TA0;`), unlike a normal CAT-tone over's per-symbol clock.
    Cat,
}

struct Engine {
    db: Arc<Db>,
    evt: Sender<EngineEvent>,
    accum: Arc<SlotAccumulator>,
    slot_tx: Sender<Vec<f32>>,
    dec_rx: Receiver<Vec<DecodedMessage>>,
    /// Current operating mode (FT8/FT4) -- drives cycle length, TX
    /// encode/timing, and which decoder protocol `spawn_decoder_thread` runs.
    /// Changed only via `EngineCommand::SetMode` (see `switch_mode`), never
    /// written directly elsewhere, so the decoder thread and this field never
    /// disagree about which protocol is live.
    mode: Mode,
    txed_slot: i64,
    qso: QsoEngine,
    /// QRZ.com XML station-lookup cache + Logbook API upload -- see
    /// `qrz.rs`'s top doc comment for the three real features this backs
    /// (prefetch-on-target, auto-upload-on-log, and the manual Settings
    /// upload button, which reads `db` directly rather than going through
    /// this field).
    qrz_xml: qrz::QrzXmlClient,
    /// Uploads decoded spots to PSKReporter -- see `pskreporter_sender.rs`'s
    /// top doc comment. Gated on `enablePskReporter`, checked fresh on
    /// every slot (not cached) so a live Settings toggle takes effect
    /// immediately.
    psk_sender: PskReporterSender,
    rig: Option<RigConnection>,
    input: Option<AudioInput>,
    input_device: Option<String>,
    output_device: Option<String>,
    decoding: bool,
    /// Set via `EngineCommand::SetDecodeSuspended` -- a narrower, temporary
    /// pause than `decoding` itself. See that command's doc comment for why
    /// it exists and exactly what it does and doesn't touch.
    decode_suspended: bool,
    /// Settings -> Advanced's "Decode Depth" -- see `spawn_decoder_thread`'s
    /// doc comment. Only takes effect on the next decoder (re)spawn
    /// (mode switch or `EngineCommand::SetDecodeDepth`), same as `mode`
    /// itself -- the decoder thread has no live mid-stream config channel.
    decode_deep: bool,
    dial_hz: u64,
    tx_audio_hz: i32,
    /// TX output level (0.0–1.0) applied to the waveform before playback.
    tx_gain: f32,
    /// Which channel TX audio goes out on (see
    /// [`crate::audio::channel_select`]) -- resolved on every `select_rig`
    /// as `tx_audio_channel_override.unwrap_or_else(auto-detect)`, where
    /// auto-detect is `LEFT` for a QMX (see
    /// [`crate::rig::needs_left_channel_tx_audio`]) and `BOTH` otherwise.
    tx_audio_channel: i32,
    /// Sticky explicit override for `tx_audio_channel`, persisted under
    /// Android's own `txAudioChannel` config key -- `None` means "never set,
    /// use the QMX-based auto-default every `select_rig`" (re-resolved fresh
    /// on each connect, never written to config -- ported from Android's own
    /// `applyTxChannelDefaultForRig`/`defaultTxChannelForRig`, whose own doc
    /// comment says this is deliberate: switching to another rig goes back
    /// to `BOTH` instead of leaving it on a left-only TX never asked for).
    /// Set via `EngineCommand::SetTxAudioChannel`.
    tx_audio_channel_override: Option<i32>,
    /// Which channel RX audio is captured from. No QMX-based auto-default
    /// (Android has none for RX either) -- just a persisted preference, set
    /// via `EngineCommand::SetRxAudioChannel` under the `rxAudioChannel` key.
    rx_audio_channel: i32,
    /// Whether TX should go out as CAT tone commands instead of audio at all
    /// (currently just the QRP Labs QMX -- see
    /// [`crate::rig::needs_cat_tone_tx`]). `qmx_cat_tones_override.unwrap_or(false)`,
    /// refreshed each `select_rig` (see `should_default_qmx_cat_tones_on`).
    tx_via_cat_tones: bool,
    /// Sticky preference for `tx_via_cat_tones`, persisted under Android's
    /// own `qmxCatTones` config key -- unlike `tx_audio_channel_override`,
    /// this is a **one-time latch**, not a per-connection auto-default:
    /// ported from Android's own `applyCatTonesDefaultForRig`/
    /// `shouldDefaultQmxCatTonesOn`, the first time ever a QMX connects with
    /// this key unset, `select_rig` turns it on *and writes the config row*,
    /// so it never auto-resets again even across switching away from and
    /// back to a QMX. Set via `EngineCommand::SetQmxCatTones`.
    qmx_cat_tones_override: Option<bool>,
    /// Whether the connected rig answers the `SW;`/`PC;` SWR/power meter
    /// queries (currently just the QRP Labs QMX -- see
    /// [`crate::rig::supports_meter_query`]). Recomputed on every
    /// `select_rig`, same as `tx_via_cat_tones`.
    rig_supports_meters: bool,
    /// The in-flight CAT-tone transmission, if `tx_via_cat_tones` started one.
    /// Mutually exclusive with `tx_playback` -- `start_transmit` uses one path
    /// or the other. Serviced from the run loop (`service_cat_tx`), never from
    /// a second thread: see [`CatToneTx`]'s doc comment for why that matters.
    tx_cat: Option<CatToneTx>,
    /// The in-flight TUNE transmission, if any. Mutually exclusive with both
    /// `tx_playback` and `tx_cat` — see [`Engine::start_tune`].
    tx_tune: Option<TuneHandle>,
    /// RX input gain (0.0–1.0), applied live inside the capture callback.
    rx_gain: f32,
    /// UTC slot id most recently handed to the decode worker.
    /// Guards the once-per-slot early decode trigger in the run loop.
    last_decoded_slot: i64,
    /// Number of decode batches handed to the worker but not yet returned. A
    /// *count* rather than a single in-flight bool: the decoder returns exactly one
    /// batch per enqueued slot, so if it ever falls behind by more than one slot,
    /// the first returned batch must not clear the gate while later slots are still
    /// pending. Gates the run loop's boundary TX trigger (via [`awaiting_any_decode`])
    /// so it never keys a stale message ahead of a slow decode (the fresh one is
    /// keyed by `handle_decoded` when it arrives). See [`boundary_tx_ready`].
    pending_decodes: usize,
    /// FIFO of the slot id each outstanding dispatch in `pending_decodes` was
    /// actually captured for. Real bug, found live (CQ Watchdog set to 5,
    /// gave up late or not at all with dead silence the whole time): the
    /// decode worker runs on its own thread and the run loop only polls it
    /// opportunistically, so a result can surface on a LATER tick than the
    /// one that dispatched it -- if decoding takes long enough to cross a
    /// slot boundary, that tick's own freshly-recomputed `slot_id` is one
    /// slot newer than the audio actually was, flipping its parity and
    /// making `handle_decoded`'s own-tx-slot skip silently treat the
    /// partner's real silent slot as "ours," skipping the miss it should
    /// have counted. Pushed alongside every dispatch, popped (not read) for
    /// each drained result, so `handle_decoded` always gets the slot the
    /// audio was actually captured for, however late the result arrives.
    pending_decode_slots: VecDeque<i64>,
    /// Retained for event compatibility. This is fixed at zero: adjusting the
    /// receive clock after a decode can remove the next slot's leading sync block.
    rx_offset_ms: i64,
    last_tick_ms: i64,
    /// Monotonic time the PTT-on frame was last sent (None while unkeyed).
    /// Drives the CAT PTT keepalive independently of the NTP-corrected clock.
    last_ptt_on_sent: Option<Instant>,
    tx_parity: Option<i64>,
    /// Operator's preferred TX slot parity (0 = Even, 1 = Odd; `None` = no
    /// preference), set via `EngineCommand::SetTxPeriod` -- matches Android's
    /// `TxPeriodControl`/`onSelectTxPeriodAction`. Only takes effect the next
    /// time a CQ/QSO starts (`StartCq` seeds `tx_parity` from this instead of
    /// `None`); it never overrides an already-locked in-progress QSO's parity.
    preferred_tx_parity: Option<i64>,
    /// Captured just before `Answer` overwrites `preferred_tx_parity` to
    /// reply opposite a CQing station, so it can be put back once the
    /// excursion ends -- QSO completed (`QsoOutcome::Completed`) or a
    /// watchdog forced it to stop (`QsoOutcome::GaveUpTarget`). Real UX
    /// requirement, stated live: "should the slot setting only change for
    /// the duration of the QSO or potential QSO and then revert to its
    /// original setting" -- your own habitual CQ-calling slot is a personal
    /// collision-avoidance choice, unrelated to whichever slot a particular
    /// station happened to use, so it shouldn't silently drift to whoever
    /// you last answered.
    ///
    /// `None` = no restore owed (operating on our own habitual parity, not
    /// mid-answer). `Some(x)` = restore `preferred_tx_parity` to `x` once
    /// the excursion concludes. Deliberately NOT re-captured by a second
    /// `Answer` while already pending (e.g. manually abandoning one target
    /// to answer a different one) -- the ORIGINAL pre-excursion value must
    /// survive however many stations the excursion spans, not just the
    /// most recent one. Also deliberately NOT applied to `tx_parity`
    /// itself (the active lock) -- only to the *preference* that seeds the
    /// next fresh `StartCq`. `tx_parity` only ever changes via an explicit
    /// activation command, never automatically when a watchdog gives up
    /// (`give_up_target` flips `qso.rs`'s own stage back to Cq internally,
    /// without going through `EngineCommand::StartCq`) -- so a late reply
    /// reopening a just-abandoned target (the exact scenario flagged live:
    /// "what would happen if I've reverted to the message panel because
    /// I've given up but get a late reply from him") still finds
    /// `tx_parity` untouched and correct, never the reverted preference.
    /// An explicit `SetTxPeriod` cancels a pending restore outright (see
    /// its own handler) -- the operator manually changing their preference
    /// mid-excursion means "until I change it" has already happened.
    preferred_tx_parity_restore: Option<Option<i64>>,
    /// Whether this run has actually keyed a real over yet -- set `false` by
    /// every activation command (StartCq/Answer/SetStage/FreeText), `true`
    /// only inside `maybe_transmit`'s success branch. Real bug, found live
    /// (the operator: "the counter should increment after I call the cq not
    /// before the actual transmission" -- confirmed by a phantom TX-log entry
    /// and the CQ Watchdog undercounting real overs): `is_own_tx_slot`
    /// (`handle_decoded`) used to infer "haven't transmitted yet" from
    /// `tx_parity.is_none()`, but `tx_parity` is seeded from
    /// `preferred_tx_parity` the instant StartCq runs -- when the operator
    /// has a fixed TX Period set (not "Auto"), `tx_parity` is *never* `None`
    /// at all, even though the very first real over can still miss its
    /// slot's TX window exactly like the Auto case. A dedicated flag is the
    /// only thing that actually tracks "has a real over gone out," regardless
    /// of whether parity was locked early by a preference or only once
    /// transmission succeeds.
    has_transmitted_this_run: bool,
    clock_offset_ms: i64,
    time_synced: bool,
    /// Clone of the command sender so background helpers (NTP sync) can feed
    /// results back into this loop.
    cmd_tx: Sender<EngineCommand>,
    ptt: bool,
    /// In-flight TX playback, if any. Held on this thread (the cpal stream is
    /// `!Send`); polled each loop tick so completion drops PTT, and dropped to
    /// stop audio immediately on Stop TX.
    tx_playback: Option<output::PreparedTx>,
    // live waterfall FFT (plan + window table + floor state; rebuilt on config change)
    wf: WfProcessor,
    last_wf_slot: i64, // last UTC cycle slot marked on the live waterfall
    /// WSJT-X UDP interface (outbound broadcast + inbound request listener).
    udp: crate::udp::UdpService,
    /// UTC ms of the last UDP Heartbeat sent (rate-limits it to ~15 s).
    last_udp_heartbeat_ms: i64,
    /// Settings -> Transmission -> TX Protection: SWR Halt. A plain ratio
    /// threshold, not Android's normalized 0-255 (see `tx_protection`'s
    /// module doc). Checked in the periodic meter tick and at the end of a
    /// CAT-tone over (`finalize_tx`) via `note_swr_reading`.
    swr_halt_enabled: bool,
    swr_halt_threshold: f32,
    /// Session-only, never persisted (matches Android's own `swrLockout`
    /// LiveData) -- set by `note_swr_reading`, cleared only by an explicit
    /// `EngineCommand::ClearSwrLockout`. Blocks starting any new TX/Tune
    /// (`maybe_transmit`/`start_tune`) until cleared.
    swr_lockout: bool,
    /// Settings -> Transmission -> Tune. Hard cap on a single tune carrier
    /// (`TuneController.DEFAULT_MAX_ON_SECONDS`-equivalent) -- desktop's
    /// Tune had no timeout at all before this.
    tune_max_on_seconds: i64,
    /// Monotonic start of the in-flight tune, for the max-on timeout. `None`
    /// whenever `tx_tune` is `None` (set together in `start_tune`, cleared
    /// together in `finalize_tx`).
    tune_started_at: Option<Instant>,
    /// Settings -> Transmission -> Tune's method picker (Automatic/Internal/
    /// Rig tuner/Low power tone) -- see `tx_protection::decide_tune_action`'s
    /// doc comment for why `Internal`/`Automatic` currently behave alike on
    /// desktop (no rig backend here exposes an ATU-start CAT command yet).
    tune_method: i32,
    /// Settings -> Transmission -> Tune's "independent tune level" -- when
    /// true, Tune uses `tune_gain` instead of the normal `tx_gain` for its
    /// carrier (ported from Android's own separate tune-level slider: a
    /// tuner match often wants less power than a full FT8 over).
    tune_level_independent: bool,
    tune_gain: f32,
    /// Settings -> Transmission's "Clear decodes on band/mode change" --
    /// see `EngineCommand::SetBand`/`switch_mode`.
    clear_on_band_mode_change: bool,
    /// RX audio capture latency compensation (ms), Android's own
    /// `rxLatencyMs` key -- see `rx_latency_adjusted_into_cycle_ms`'s doc
    /// comment. Android computes this automatically (built on the
    /// self-sync-from-decode-DT machinery, deferred on desktop pending its
    /// own safety review); this is a manual value instead, a UI surface
    /// Android itself doesn't have, built because the auto-calibration
    /// isn't available yet. Default 0 -- unset, no shift, matches every
    /// build before this setting existed.
    rx_latency_ms: i64,
    /// Settings -> Advanced's "PTT Delay" (ms) -- Android's own `pttDelay`.
    /// Feeds both the real PTT-settle sleep (the 3 `thread::sleep` sites in
    /// `start_transmit`/`send_cat_transmission`/`start_tune`) and
    /// `Mode::tx_latest_ms`'s eligibility window via
    /// `Engine::effective_tx_latest_ms` -- the two must never disagree, see
    /// `Mode::tx_latest_ms`'s doc comment. Default matches the exact
    /// constant this replaces (`mode::DEFAULT_PTT_DELAY_MS`, 100ms), so
    /// every existing install's real-world timing is unchanged until the
    /// operator actually opens this new setting and changes it.
    ptt_delay_ms: i64,
    /// Settings -> Advanced's "Late-start Tolerance" (ms) -- see
    /// `tx_protection::clamp_late_start_tolerance_ms`'s doc comment for why
    /// desktop's default (`0`) deliberately diverges from Android's
    /// (2000ms): desktop's existing behavior already refuses to start a
    /// transmission too late to fit (never clipping), which is strictly
    /// safer than Android's "clip some leading audio and go anyway" -- this
    /// setting widens `tx_slot_eligible`'s window by this many ms, and the
    /// already-existing, already-tested clip math
    /// (`cat_tones::symbols_to_skip`/`audio::output::skip_cursor`) naturally
    /// engages for whatever lateness that widened window now lets through.
    late_start_tolerance_ms: i64,
}

/// Whether the keepalive should resend PTT-on now. `since_last_on` is the
/// monotonic time since the last PTT-on frame (None = never sent this over).
fn should_refresh_ptt(
    ptt_on: bool,
    rig_requires_keepalive: bool,
    since_last_on: Option<Duration>,
) -> bool {
    ptt_on
        && rig_requires_keepalive
        && since_last_on.map_or(true, |d| d >= PTT_KEEPALIVE_INTERVAL)
}

/// Spawn the decode worker: owns the (!Send) `Decoder` on its own thread so
/// the heavy per-slot DSP never blocks the engine loop (which drives the live
/// waterfall + UTC clock). Fed slot audio over the returned sender; decodes
/// come back over the returned receiver. `protocol` selects the decode
/// protocol (`Mode::ffi_protocol()`) -- called once from `Engine::new` and
/// again from `switch_mode` whenever the operating mode changes, since a
/// `Decoder` is built for one protocol for its whole lifetime.
fn spawn_decoder_thread(protocol: std::os::raw::c_int, deep: bool) -> (Sender<Vec<f32>>, Receiver<Vec<DecodedMessage>>) {
    let (slot_tx, slot_rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let (dec_tx, dec_rx) = std::sync::mpsc::channel::<Vec<DecodedMessage>>();
    std::thread::Builder::new()
        .name("ft8af-decoder".into())
        .spawn(move || {
            let mut decoder = Decoder::new(SAMPLE_RATE, protocol);
            decoder.set_deep(deep);
            while let Ok(slot) = slot_rx.recv() {
                decoder.feed_slot(&slot);
                let n = decoder.find_sync();
                let msgs = if n > 0 { decoder.decode_all() } else { Vec::new() };
                let _ = dec_tx.send(msgs);
            }
        })
        .expect("spawn decoder thread");
    (slot_tx, dec_rx)
}

impl Engine {
    fn new(db: Arc<Db>, evt: Sender<EngineEvent>, cmd_tx: Sender<EngineCommand>) -> Self {
        // Restore persisted station + band where present.
        let my_call = db.get_config("my_call").unwrap_or_default();
        let my_grid = db.get_config("my_grid").unwrap_or_default();
        let dial_hz = db
            .get_config("dial_hz")
            .and_then(|s| s.parse().ok())
            .unwrap_or(bands::default_band().dial_hz);
        let input_device = db.get_config("input_device").filter(|s| !s.is_empty());
        let output_device = db.get_config("output_device").filter(|s| !s.is_empty());
        let tx_audio_hz = db
            .get_config("base_freq")
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_TX_AUDIO_HZ);
        let tx_gain = db
            .get_config("tx_gain")
            .and_then(|s| s.parse::<f32>().ok())
            .map(clamp_tx_gain)
            .unwrap_or(DEFAULT_TX_GAIN);
        let rx_gain = db
            .get_config("rx_gain")
            .and_then(|s| s.parse::<f32>().ok())
            .map(clamp_rx_gain)
            .unwrap_or(DEFAULT_RX_GAIN);
        // Sticky overrides (Android's own `txAudioChannel`/`qmxCatTones` key
        // names/encoding -- see audio::channel_select's doc comment): absent
        // key means "never set," preserved as `None` so `select_rig` keeps
        // falling back to the QMX auto-detect exactly as before this existed.
        let tx_audio_channel_override: Option<i32> = db
            .get_config("txAudioChannel")
            .and_then(|s| s.trim().parse::<i32>().ok())
            .map(crate::audio::channel_select::clamp);
        let qmx_cat_tones_override: Option<bool> =
            db.get_config("qmxCatTones").map(|s| s == "1");
        // No auto-default for RX (Android has none either) -- just a plain
        // persisted preference, defaulting to BOTH like every build before
        // this feature existed.
        let rx_audio_channel = crate::audio::channel_select::parse(db.get_config("rxAudioChannel").as_deref());
        // Restore the last NTP offset so DT is roughly right immediately, before
        // the first fresh sync of this session lands. Treated as already-synced.
        let saved_offset: Option<i64> = db.get_config("clock_offset_ms").and_then(|s| s.parse().ok());
        // Versions through 0.1.1 persisted an auto-calibrated RX offset. Applying
        // that offset changes which samples are treated as the slot start and can
        // remove the leading Costas sync block. Clear every legacy value and keep
        // receive windows anchored to UTC.
        let rx_offset_ms = 0;
        let _ = db.set_config("rx_offset_ms", "0");
        let mode = Mode::from_config(&db.get_config("operating_mode").unwrap_or_default());
        // Settings -> Advanced's "Decode Depth": more LDPC iterations + a
        // lower sync-score threshold (see `Decoder::set_deep`'s own doc
        // comment) -- a real, already-ported-but-never-wired knob found
        // while researching Advanced, not new decode logic. Own config key
        // (`decodeDeep`, plain boolean) rather than Android's `decodeDepth`
        // (0/1/2 Fast/Balanced/Deep): desktop only has this one FAST/DEEP
        // distinction, no separate BALANCED tier, so reusing Android's 3-
        // value encoding would be a false equivalence.
        let decode_deep = db.get_config("decodeDeep").as_deref() == Some("1");
        let mut qso = QsoEngine::new(my_call, my_grid);
        qso.band = bands::band_for_freq_hz(dial_hz).to_string();
        qso.freq_mhz = bands::freq_mhz_string(dial_hz);
        qso.mode = mode.display_name().to_string();
        // Settings -> Transmission's TX Watchdog / Max 73 Sends -- Android's
        // own key names, plain integer counts, 0 = unlimited/Auto (Android's
        // own defaults).
        qso.no_reply_limit = db.get_config("noReplyLimit").and_then(|s| s.parse().ok()).unwrap_or(0);
        qso.max_73_sends = db.get_config("max73Sends").and_then(|s| s.parse().ok()).unwrap_or(0);
        qso.cq_no_reply_limit = db.get_config("cqNoReplyLimit").and_then(|s| s.parse().ok()).unwrap_or(0);
        qso.cq_answer_limit = db.get_config("cqAnswerLimit").and_then(|s| s.parse().ok()).unwrap_or(0);
        // Settings -> Auto-Sequence's "Auto-CQ after QSO" (Android's
        // `autoCQAfterQSO`, default **false**). Desktop's own default here
        // is deliberately kept at **true** when the key is unset -- this
        // field already existed, hardcoded true, before this Settings pass
        // (every completed QSO has always auto-chained back into CQ on
        // desktop); wiring it to Android's real default would silently
        // change that behavior for every existing install the first time
        // this build runs. An operator who wants Android's own default can
        // just switch the new toggle off.
        qso.auto_return_to_cq = db.get_config("autoCQAfterQSO").map(|s| s == "1").unwrap_or(true);

        // Restore persisted waterfall FFT knobs (issue #428); defaults match
        // the historical hard-coded pipeline (Hann, 2048, 6-segment Welch).
        let wf_defaults = WfConfig::default();
        let wf_cfg = WfConfig {
            window: db
                .get_config("wf_window")
                .map(|s| WfWindow::parse(&s))
                .unwrap_or(wf_defaults.window),
            fft_size: db
                .get_config("wf_fft_size")
                .and_then(|s| s.parse().ok())
                .unwrap_or(wf_defaults.fft_size),
            avg: db
                .get_config("wf_avg")
                .and_then(|s| s.parse().ok())
                .unwrap_or(wf_defaults.avg),
            display_hz: db
                .get_config("wf_display_hz")
                .and_then(|s| s.parse().ok())
                .unwrap_or(wf_defaults.display_hz),
            denoise: db
                .get_config("wf_denoise")
                .map(|s| s == "true")
                .unwrap_or(wf_defaults.denoise),
        }
        .sanitize();
        let wf = WfProcessor::new(wf_cfg);

        // Settings -> Transmission -> TX Protection/Tune/Clear-on-change.
        // Thresholds/levels use Android's exact defaults where this is a
        // direct equivalent (tune max-on seconds); SWR halt uses a plain
        // ratio, not Android's normalized 0-255 scale (see tx_protection's
        // module doc).
        let swr_halt_enabled = db.get_config("swrHaltEnabled").map(|s| s == "1").unwrap_or(false);
        let swr_halt_threshold = db
            .get_config("swrHaltThreshold")
            .and_then(|s| s.parse().ok())
            .unwrap_or(tx_protection::DEFAULT_SWR_HALT_THRESHOLD);
        let tune_max_on_seconds = tx_protection::clamp_tune_max_on_seconds(
            db.get_config("tuneMaxOnSeconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(tx_protection::DEFAULT_TUNE_MAX_ON_SECONDS),
        );
        let tune_method = tx_protection::clamp_tune_method(
            db.get_config("tuneMethod").and_then(|s| s.parse().ok()).unwrap_or(0),
        );
        let tune_level_independent =
            db.get_config("tuneLevelIndependent").map(|s| s == "1").unwrap_or(false);
        let tune_gain = db
            .get_config("tuneLevel")
            .and_then(|s| s.parse::<f32>().ok())
            .map(clamp_tx_gain)
            .unwrap_or(tx_gain);
        let clear_on_band_mode_change =
            db.get_config("clearOnBandModeChange").map(|s| s == "1").unwrap_or(true);
        let rx_latency_ms = db
            .get_config("rxLatencyMs")
            .and_then(|s| s.parse().ok())
            .map(clamp_rx_latency_ms)
            .unwrap_or(0);
        let ptt_delay_ms = db
            .get_config("pttDelay")
            .and_then(|s| s.parse().ok())
            .map(tx_protection::clamp_ptt_delay_ms)
            .unwrap_or(tx_protection::DEFAULT_PTT_DELAY_MS);
        // Default 0, deliberately not Android's 2000 -- see
        // `late_start_tolerance_ms`'s own doc comment.
        let late_start_tolerance_ms = db
            .get_config("lateStartTolerance")
            .and_then(|s| s.parse().ok())
            .map(tx_protection::clamp_late_start_tolerance_ms)
            .unwrap_or(0);

        let (slot_tx, dec_rx) = spawn_decoder_thread(mode.ffi_protocol(), decode_deep);

        let psk_sender = PskReporterSender::new();
        {
            let flush_sender = psk_sender.clone();
            let flush_db = db.clone();
            psk_sender.start(move || {
                let rig = flush_sender.rig_name();
                pskreporter_sender::StationInfo {
                    my_call: flush_db.get_config("my_call").unwrap_or_default(),
                    my_grid: flush_db.get_config("my_grid").unwrap_or_default(),
                    software: pskreporter_sender::build_software_string(env!("CARGO_PKG_VERSION"), rig.as_deref()),
                    antenna: flush_db.get_config("antenna").unwrap_or_default(),
                    rig: rig.unwrap_or_default(),
                }
            });
        }

        let engine = Engine {
            db,
            decode_deep,
            evt,
            accum: Arc::new(SlotAccumulator::new()),
            slot_tx,
            dec_rx,
            mode,
            txed_slot: -1,
            qso,
            qrz_xml: qrz::QrzXmlClient::new(),
            psk_sender,
            rig: None,
            input: None,
            input_device,
            output_device,
            decoding: false,
            decode_suspended: false,
            dial_hz,
            tx_audio_hz,
            tx_gain,
            tx_audio_channel: crate::audio::channel_select::BOTH,
            tx_audio_channel_override,
            rx_audio_channel,
            tx_via_cat_tones: false,
            qmx_cat_tones_override,
            rig_supports_meters: false,
            tx_cat: None,
            tx_tune: None,
            rx_gain,
            last_decoded_slot: -1,
            pending_decodes: 0,
            pending_decode_slots: VecDeque::new(),
            rx_offset_ms,
            last_tick_ms: 0,
            last_ptt_on_sent: None,
            tx_parity: None,
            preferred_tx_parity: None,
            preferred_tx_parity_restore: None,
            has_transmitted_this_run: false,
            clock_offset_ms: saved_offset.unwrap_or(0),
            time_synced: saved_offset.is_some(),
            udp: crate::udp::UdpService::new(cmd_tx.clone()),
            cmd_tx,
            ptt: false,
            tx_playback: None,
            wf,
            last_wf_slot: -1,
            last_udp_heartbeat_ms: 0,
            swr_halt_enabled,
            swr_halt_threshold,
            swr_lockout: false,
            tune_max_on_seconds,
            tune_started_at: None,
            tune_method,
            tune_level_independent,
            tune_gain,
            clear_on_band_mode_change,
            rx_latency_ms,
            ptt_delay_ms,
            late_start_tolerance_ms,
        };

        // Catch up on any QSO that failed to auto-upload to QRZ in a
        // previous session (e.g. no internet at the time it was logged) --
        // real gap, found live: the per-QSO auto-upload (log_completed_qso,
        // below) is a single attempt with no retry, and the one UI control
        // that looks like it should retry a backlog ("Sync to logging
        // services") is a stale stub predating that feature entirely,
        // unconnected to it. Spawned on a background thread, same reasoning
        // as the per-QSO upload -- a slow/blocked network call must never
        // stall engine construction. Each record stays `synced_qrz = 0`
        // until it actually succeeds, so a failed attempt here (e.g.
        // network not up yet this early in boot) just tries again next
        // launch, same as it would have before this existed.
        //
        // Requires a real API key, not just the enable toggle -- stated
        // live: "the qrz upload should only run at startup if the qrz api
        // key is not empty because an empty key means a station does not
        // subscribe to the qrz auto upload." `upload_pending` itself
        // already no-ops on an empty key, but checking here too avoids
        // spawning a thread (and a DB scan for pending rows) for a station
        // that isn't using this feature at all.
        let qrz_api_key = engine.db.get_config(qrz::CONFIG_API_KEY).unwrap_or_default();
        if engine.db.get_config(qrz::CONFIG_ENABLE).as_deref() == Some("1") && !qrz_api_key.trim().is_empty() {
            let db_for_upload = engine.db.clone();
            std::thread::spawn(move || {
                qrz::upload_pending(&db_for_upload, &qrz_api_key);
            });
        }

        // Same startup catch-up, for Cloudlog/Wavelog/Nextlog and World
        // Radio League -- see the QRZ block above for the reasoning
        // (never block engine construction, gate on a real credential so
        // a station not using the feature doesn't even spawn the thread).
        let cloudlog_api_key = engine.db.get_config(cloudlog::CONFIG_API_KEY).unwrap_or_default();
        if engine.db.get_config(cloudlog::CONFIG_ENABLE).as_deref() == Some("1") && !cloudlog_api_key.trim().is_empty()
        {
            let db_for_upload = engine.db.clone();
            std::thread::spawn(move || {
                cloudlog::upload_pending(&db_for_upload);
            });
        }
        let wrl_api_key = engine.db.get_config(wrl::CONFIG_API_KEY).unwrap_or_default();
        if engine.db.get_config(wrl::CONFIG_ENABLE).as_deref() == Some("1") && !wrl_api_key.trim().is_empty() {
            let db_for_upload = engine.db.clone();
            std::thread::spawn(move || {
                wrl::upload_pending(&db_for_upload);
            });
        }
        // Same startup catch-up, for HamQTH, Club Log and HRDLog.net.
        let hamqth_password = engine.db.get_config(hamqth::CONFIG_PASSWORD).unwrap_or_default();
        if engine.db.get_config(hamqth::CONFIG_ENABLE).as_deref() == Some("1") && !hamqth_password.is_empty() {
            let db_for_upload = engine.db.clone();
            let callsign = engine.db.get_config("my_call").unwrap_or_default();
            std::thread::spawn(move || {
                hamqth::upload_pending(&db_for_upload, &callsign, &hamqth_password);
            });
        }
        let clublog_token = engine.db.get_config(clublog::CONFIG_TOKEN).unwrap_or_default();
        if engine.db.get_config(clublog::CONFIG_ENABLE).as_deref() == Some("1") && !clublog_token.trim().is_empty() {
            let db_for_upload = engine.db.clone();
            std::thread::spawn(move || {
                clublog::upload_pending(&db_for_upload, &clublog_token);
            });
        }
        let hrdlog_code = engine.db.get_config(hrdlog::CONFIG_CODE).unwrap_or_default();
        if engine.db.get_config(hrdlog::CONFIG_ENABLE).as_deref() == Some("1") && !hrdlog_code.trim().is_empty() {
            let db_for_upload = engine.db.clone();
            let callsign = engine.db.get_config("my_call").unwrap_or_default();
            std::thread::spawn(move || {
                hrdlog::upload_pending(&db_for_upload, &callsign, &hrdlog_code);
            });
        }
        // Upload side only -- eQSL's download/import is a manual Settings
        // action, same as LoTW's (`LotwDialog`'s "Download new"/"Download
        // all" buttons, never run automatically on startup).
        let eqsl_password = engine.db.get_config(eqsl::CONFIG_PASSWORD).unwrap_or_default();
        if engine.db.get_config(eqsl::CONFIG_ENABLE).as_deref() == Some("1") && !eqsl_password.is_empty() {
            let db_for_upload = engine.db.clone();
            let username = engine.db.get_config(eqsl::CONFIG_USERNAME).unwrap_or_default();
            std::thread::spawn(move || {
                eqsl::upload_pending(&db_for_upload, &username, &eqsl_password);
            });
        }

        engine
    }

    /// Load the persisted WSJT-X UDP settings into a config struct. Falls back to
    /// the WSJT-X-compatible defaults (disabled, 127.0.0.1:2237) for missing keys.
    fn read_udp_config(&self) -> crate::udp::UdpConfig {
        let d = crate::udp::UdpConfig::default();
        crate::udp::UdpConfig {
            enabled: self.db.get_config("udp_enabled").as_deref() == Some("true"),
            host: self.db.get_config("udp_host").filter(|s| !s.is_empty()).unwrap_or(d.host),
            port: self
                .db
                .get_config("udp_port")
                .and_then(|s| s.parse().ok())
                .unwrap_or(d.port),
            accept_requests: self.db.get_config("udp_accept_requests").as_deref() == Some("true"),
        }
    }

    fn now(&self) -> i64 {
        util::now_unix_ms() + self.clock_offset_ms
    }

    fn run(&mut self, cmd_rx: Receiver<EngineCommand>) {
        // Bring up the WSJT-X UDP interface from persisted settings (binds the
        // socket + inbound listener if enabled). Non-fatal — a bad host/port just
        // surfaces an error and leaves the feature off.
        let udp_cfg = self.read_udp_config();
        match self.udp.apply(udp_cfg) {
            Ok(msg) => self.emit(EngineEvent::Info(msg)),
            Err(e) => self.emit(EngineEvent::Error(e)),
        }

        // Reconnect the last-used rig on startup (non-fatal if it can't).
        // Retries a few times, a beat apart, before giving up: a USB-serial
        // rig's device node isn't always enumerated yet at the exact instant
        // the app launches (confirmed live: a QMX reconnect lost this race
        // once, leaving the rig silently "offline" with no further attempt
        // until the operator noticed and pressed Connect manually in
        // Settings) -- a single one-shot try had no way to recover from
        // that. Each failed attempt still emits its own `select_rig` error
        // (so the operator can see retries happening, not just silence),
        // and a later success clears it the normal way via
        // `publish_rig_status`.
        if let Some(json) = self.db.get_config("rig_config") {
            if let Ok(cfg) = serde_json::from_str::<RigConfig>(&json) {
                if cfg.backend != crate::rig::RigBackend::None {
                    const STARTUP_RECONNECT_ATTEMPTS: u32 = 3;
                    const STARTUP_RECONNECT_DELAY: Duration = Duration::from_millis(1500);
                    for attempt in 1..=STARTUP_RECONNECT_ATTEMPTS {
                        self.select_rig(cfg.clone());
                        if self.rig.is_some() {
                            break;
                        }
                        if attempt < STARTUP_RECONNECT_ATTEMPTS {
                            std::thread::sleep(STARTUP_RECONNECT_DELAY);
                        }
                    }
                }
            }
        }

        loop {
            // Drain pending commands.
            loop {
                match cmd_rx.try_recv() {
                    Ok(EngineCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                    Ok(cmd) => self.handle(cmd),
                    Err(TryRecvError::Empty) => break,
                }
            }

            let now = self.now();
            let ms_into = now.rem_euclid(self.mode.cycle_ms());
            let slot_id = now.div_euclid(self.mode.cycle_ms());

            // 1 Hz UI clock.
            if now - self.last_tick_ms >= 1000 {
                self.last_tick_ms = now;
                self.emit(EngineEvent::Cycle(CycleTick {
                    utc_ms: now,
                    sequential: slot_id.rem_euclid(2),
                    ms_into_cycle: ms_into,
                }));
                // WSJT-X UDP Status every tick (so companions track dial/TX/decode
                // state), Heartbeat every ~15 s (how they discover our address).
                self.broadcast_status();
                if now - self.last_udp_heartbeat_ms >= 15_000 {
                    self.last_udp_heartbeat_ms = now;
                    self.broadcast_heartbeat();
                }
                // RX input level + silence warning, once we're capturing. Surfaces
                // a dead source (e.g. DAX channel not routed) that otherwise looks
                // like a broken waterfall.
                if self.decoding {
                    let recent = self.accum.peek_recent(SAMPLE_RATE as usize / 2); // ~0.5 s
                    let db = rms_dbfs(&recent);
                    let peak = peak_linear(&recent);
                    let rms = rms_linear(&recent);
                    let peak_dbfs = to_dbfs(peak);
                    let status = classify_input_level(peak, peak_dbfs, db);
                    self.emit(EngineEvent::InputLevel(InputLevelEvent { db, peak, rms, status }));
                }

                // QMX SWR/power meters, read over CAT -- ported from Android's
                // MeterProtectionController/QmxMeterReplies polling (there,
                // driven by the rig's own showAlert() callback; here, the
                // same ~1/s cadence as this tick block). Only while actually
                // keyed (there's no RF to measure otherwise) and only for a
                // QMX, the one rig this firmware extension exists on.
                //
                // Deliberately excludes `tx_cat` (an in-flight CAT-tone
                // transmission): its per-symbol `TA` commands share this same
                // blocking serial connection on this same engine thread, paced
                // by this run loop's own Instant-based clock (see
                // service_cat_tx). A query+reply round-trip inserted here
                // would add real, if brief, latency to that thread right
                // before service_cat_tx runs later in this same iteration --
                // a correctness risk to exact symbol timing this project has
                // put a lot of care into (see docs/qmx-transmit.md) that
                // isn't worth it for a once-a-second meter read. Audio-based
                // TX and Tune (a steady, non-symbol-paced carrier) have no
                // such timing dependency, so polling there is safe; a CAT-tone
                // transmission instead gets one final reading right as it
                // ends (see finalize_tx). A failed query just means no
                // reading this tick -- the meter display simply holds its
                // last value, same as Android's.
                let meter_safe_to_poll = self.tx_playback.is_some() || self.tx_tune.is_some();
                if meter_safe_to_poll && self.rig_supports_meters {
                    let swr = self.poll_meters();
                    self.note_swr_reading(swr);
                    // SWR Halt just fired on a reading taken mid-transmission
                    // (tx_playback) or mid-tune (tx_tune) -- abort it right
                    // now rather than waiting for the over to finish on its
                    // own, matching Android's `haltForSwr` calling
                    // `setActivated(false)` immediately.
                    if self.swr_lockout && (self.tx_playback.is_some() || self.tx_tune.is_some()) {
                        self.finalize_tx();
                    }
                }
            }

            // CAT PTT keepalive (G106). Checked every loop pass against a
            // monotonic clock, not on the 1 Hz UI tick, which runs on the
            // NTP-corrected clock and can stall when the offset steps backwards.
            self.refresh_ptt_keepalive();

            // Tune max-on timeout: force-stop a tune that's run past its
            // configured cap even if the operator does nothing -- a
            // continuous carrier is the most dangerous thing this app can
            // emit, so this is checked every loop pass against the same
            // monotonic clock as the PTT keepalive above (immune to the
            // NTP-corrected `now`'s clock-jump caveat), not gated on the
            // 1 Hz UI tick.
            if let Some(started) = self.tune_started_at {
                if tx_protection::tune_expired(started.elapsed().as_millis() as i64, self.tune_max_on_seconds) {
                    self.emit(EngineEvent::Info(format!("Tune stopped after {}s", self.tune_max_on_seconds)));
                    self.finalize_tx();
                }
            }

            // Decode stays anchored to the UTC slot. Do not shift this from a
            // decoded message's DT: doing so changes the sample chosen as the slot
            // start and can cut off the next message's leading Costas sync block.
            // We fire once per slot as soon as the 12.64 s waveform is captured
            // (~13.2 s in) rather than at the 15 s boundary, so decodes land a slot
            // earlier and the operator can reply on the next cycle (not skip one).
            let rx_slot_id = now.div_euclid(self.mode.cycle_ms());
            // Shifted back by rx_latency_ms (0 for anyone who hasn't set it,
            // same as every build before this setting existed) -- see
            // rx_latency_adjusted_into_cycle_ms's own doc comment.
            let into_rx_cycle = rx_latency_adjusted_into_cycle_ms(
                now.rem_euclid(self.mode.cycle_ms()),
                self.rx_latency_ms,
            );
            if decode_dispatch_due(
                self.decoding,
                self.decode_suspended,
                rx_slot_id,
                self.last_decoded_slot,
                into_rx_cycle,
                self.mode.decode_at_ms(),
            ) {
                self.last_decoded_slot = rx_slot_id;
                // The slot's audio so far = the most recent `into_rx_cycle` of it,
                // front-aligned (signal at sample 0) with the tail zero-padded.
                let elapsed = (into_rx_cycle * SAMPLE_RATE as i64 / 1000) as usize;
                let slot = self.accum.take_slot_from_start(elapsed);
                if self.slot_tx.send(slot).is_ok() {
                    // Hold off the boundary TX trigger until this slot's decodes
                    // come back, so it can't key a stale message ahead of a slow
                    // decode (the fresh one is keyed by handle_decoded on arrival).
                    // Count it: if the decoder is already behind, the gate must
                    // stay closed until every outstanding slot has returned.
                    self.pending_decodes += 1;
                    // Remember which slot this one's actually for -- see
                    // `pending_decode_slots`'s own doc comment.
                    self.pending_decode_slots.push_back(rx_slot_id);
                }
            }

            // Act on any decodes the worker has finished (off-loop DSP). This is
            // where steady-state TX is driven: each slot's decodes advance the QSO
            // and then `maybe_transmit` keys up if it's our turn. Immediate keying
            // on an operator tap is handled separately, in the command handlers.
            while let Ok(msgs) = self.dec_rx.try_recv() {
                // One batch settled — decrement, not reset, so a still-pending
                // later slot keeps the boundary gate closed. saturating_sub guards
                // against a stray batch arriving after a StopDecode reset.
                self.pending_decodes = self.pending_decodes.saturating_sub(1);
                // The slot this batch was actually captured for -- NOT this
                // tick's own `slot_id`, which is freshly recomputed from
                // `now` every loop pass and can have already ticked over to
                // the next slot by the time a slow decode's result surfaces.
                // Falls back to `slot_id` only if the queue and the count
                // above have ever drifted apart (should never happen; see
                // `pending_decode_slots`'s doc comment).
                let decoded_slot_id = self.pending_decode_slots.pop_front().unwrap_or(slot_id);
                self.handle_decoded(msgs, decoded_slot_id);
            }

            // Boundary TX trigger: key a queued reply/CQ early in its slot (anywhere
            // within the TX window), even when this slot's decodes were processed
            // earlier in the previous cycle. `maybe_transmit` is otherwise only
            // reached on decode arrival, which on a fast decode lands mid-slot — past
            // the TX window — so the queued message would never go out (auto-sequence
            // stalls; CQ never retransmits). Gated on `!awaiting_any_decode` so a
            // stale message is never keyed before every outstanding slot's decodes
            // are in; `maybe_transmit` re-checks eligibility and is idempotent per
            // slot (the `txed_slot` guard).
            if boundary_tx_ready(
                self.decoding,
                self.qso.active,
                awaiting_any_decode(self.pending_decodes),
                self.tx_parity,
                slot_id,
                self.txed_slot,
                now.rem_euclid(self.mode.cycle_ms()),
                self.effective_tx_latest_ms(),
            ) {
                self.maybe_transmit();
            }

            // Drop PTT as soon as the TX waveform finishes clocking out (or a
            // stalled device trips the deadline). Polling here — rather than
            // blocking inside `transmit` — is what lets Stop TX take effect
            // mid-transmission instead of at the end of the cycle.
            if let Some(pb) = self.tx_playback.as_ref() {
                if pb.is_done() || pb.timed_out() {
                    self.finalize_tx();
                }
            }

            // QMX CAT-tone TX: send any symbols now due, finish the over once
            // the last one has held its full period. No-op when not active.
            self.service_cat_tx();

            // Live scrolling waterfall: one spectrum row per ~100 ms tick,
            // frozen while transmitting -- see waterfall_row_due.
            if waterfall_row_due(self.decoding, self.ptt) {
                self.emit_waterfall_row();
            }

            // A CAT-tone transmission needs a tighter tick so its 160 ms-spaced
            // symbols go out close to on time (see TX_CAT_TICK_MS and
            // CatToneTx's doc comment); otherwise the normal, cheaper cadence.
            let tick_ms = if self.tx_cat.is_some() { TX_CAT_TICK_MS } else { 100 };
            std::thread::sleep(Duration::from_millis(tick_ms));
        }
    }

    /// Handle one slot's decodes (from the worker): publish them, advance the QSO
    /// state machine, then key up if it's our turn this slot. Runs once per slot
    /// (the decoder returns a batch — possibly empty — every cycle), so this is
    /// what drives CQ/auto-sequence retransmits.
    fn handle_decoded(&mut self, decoded: Vec<DecodedMessage>, slot_id: i64) {
        let slot_start_ms = slot_id * self.mode.cycle_ms();
        self.publish_decodes(&decoded, slot_start_ms);
        self.broadcast_decodes(&decoded);

        // PSKReporter upload: enqueue this slot's decodes (gated on
        // enablePskReporter inside `enqueue` itself), and keep the sender's
        // rig-name snapshot in sync -- see `psk_sender`'s own doc comment
        // for why this is the handoff point rather than every individual
        // connect/disconnect call site.
        self.psk_sender.set_rig_name(self.rig.as_ref().map(|r| r.name()));
        let psk_enabled = self.db.get_config(pskreporter_sender::CONFIG_ENABLE).map(|v| v != "0").unwrap_or(true);
        self.psk_sender.enqueue(
            &decoded,
            psk_enabled,
            &self.qso.my_call,
            self.dial_hz,
            self.mode,
            slot_start_ms / 1000,
            self.now(),
        );

        // Own-slot skip: on a slot assigned to OUR transmit parity, the
        // partner structurally can't be replying (FT8 QSOs alternate by
        // parity -- half of every full round trip is "our turn," where
        // nobody else transmits). Real bug, found live: without this,
        // every real round trip counted as TWO misses instead of one
        // (one for the partner's genuine silence, one for our own
        // trivially-empty transmit slot), so the TX Watchdog/"Stop
        // After" (no_reply_count) and the CQ Watchdog (cq_no_reply_count)
        // both gave up at roughly HALF their configured limit -- e.g.
        // "3" produced only ~1-2 real retries, not 3. Mirrors Android's
        // own equivalent guard (`parseMessageToFunction`'s own-slot
        // early-out, `msgList.get(0).getSequence() == sequential`).
        //
        // We haven't transmitted at all yet this run -- which includes a
        // brief real window, not just "before the operator clicks":
        // StartCq/SetStage/FreeText try to key the very slot they're issued
        // in, and that attempt often misses the ~2.26s TX window (an
        // operator's click lands at an arbitrary point in the cycle, not at
        // its top), leaving the first real over to go out on the NEXT
        // slot's boundary trigger instead. A slot where we haven't
        // transmitted *at all* yet is just as structurally nothing-to-
        // reply-to as our own transmit slot is, so it must be skipped the
        // same way, not treated as a real round trip. Real bug, found live
        // (CQ Watchdog set to 3, only 2 CQs actually went out; separately,
        // the operator: "the counter should increment after I call the cq
        // not before the actual transmission") -- `tx_parity.is_none()`
        // alone isn't a reliable proxy for this: StartCq seeds `tx_parity`
        // from `preferred_tx_parity` immediately, so with a fixed TX Period
        // set (not "Auto"), `tx_parity` is *never* `None` even though the
        // very first real over can still miss its slot. `has_transmitted_
        // this_run` (set only inside `maybe_transmit`'s success branch) is
        // the actual, unambiguous signal -- see its own doc comment.
        let is_own_tx_slot = !self.has_transmitted_this_run
            || self.tx_parity == Some(slot_id.rem_euclid(2));
        // Caller-queue (pileup-while-busy): a SEPARATE pass over this same
        // decoded batch from process_rx below, called unconditionally
        // every cycle regardless of stage/own-slot -- see
        // QsoEngine::offer_callers's own doc comment for why this can't be
        // folded into process_rx (which early-returns at Bye73 before ever
        // looking at `decoded` for a third party, the exact live bug this
        // closes).
        self.qso.offer_callers(&decoded, slot_start_ms, self.mode.cycle_ms());
        let qso_outcome = if is_own_tx_slot { None } else { self.qso.process_rx(&decoded) };

        match qso_outcome {
            Some(QsoOutcome::Completed(record)) => {
                self.restore_preferred_tx_parity_if_owed();
                self.log_completed_qso(record);
            }
            Some(QsoOutcome::GaveUpTarget) => self.restore_preferred_tx_parity_if_owed(),
            None => {}
        }
        self.maybe_transmit();
        self.publish_tx_state();
    }

    /// Fill in everything `qso.rs` deliberately builds a bare `QsoRecord`
    /// without (it stays pure, no DB/config access -- see each fill's own
    /// comment below for why that line lives here instead of there), then
    /// log it: DB insert, ADIF mirror, QRZ upload, the `QsoCompleted` event.
    /// Called from both `handle_decoded` (completing on a *received*
    /// confirmation) and `maybe_transmit` (completing the instant RR73 is
    /// *sent* -- see `QsoEngine::notify_transmitted`'s own doc comment) --
    /// extracted so a QSO logs identically regardless of which side of the
    /// exchange actually finished it.
    fn log_completed_qso(&mut self, mut record: QsoRecord) {
        // The operator's current power setting at the moment the QSO
        // completes (ADIF TX_PWR) -- matches Android's own QSLTable.tx_pwr.
        record.tx_pwr = self.db.get_config("powerWatts").unwrap_or_default();
        // Auto comment, ported from Android's `QSLRecord` constructor
        // exactly: "Distance: <N> <mi|km>, QSO by FT8AF" when both grids
        // are known, else just "QSO by FT8AF". `distanceInMiles` (Settings
        // -> Decode & Filters, default on) is read fresh each time rather
        // than cached, matching every other per-QSO config read in this
        // function.
        let miles = self.db.get_config("distanceInMiles").as_deref() != Some("0");
        record.comment = match util::maidenhead_distance_km(&record.my_gridsquare, &record.gridsquare) {
            Some(km) => {
                let dist = if miles { km * 0.621371 } else { km };
                format!("Distance: {:.0} {}, QSO by FT8AF", dist, if miles { "mi" } else { "km" })
            }
            None => "QSO by FT8AF".to_string(),
        };
        // Fork: fill COUNTRY (DXCC prefix lookup) and STATE (QRZ's
        // registered state for the partner, gated by `logged_state`'s
        // portable/grid-match rules) when the QSO itself didn't already
        // carry one -- ported from Android's `DatabaseOpr.fillPlace`.
        // See `logged_state.rs`'s doc comment for a real, disclosed gap
        // (desktop has no POTA/SIG tracking on a QSO yet, so only the
        // callsign-suffix half of "portable" is checked).
        if record.country.is_empty() {
            if let Some(m) = dxcc::lookup(&record.call) {
                if !m.country_name.is_empty() {
                    record.country = m.country_name;
                }
            }
        }
        if record.state.is_empty() {
            let us_or_unknown = record.country.is_empty()
                || record.country.to_uppercase().starts_with("UNITED STATES");
            if us_or_unknown {
                let grid_state = us_states::state_for_grid(&record.gridsquare);
                let qrz = self.qrz_xml.cached(&record.call);
                let portable = logged_state::is_portable_call(&record.call);
                if let Some(picked) = logged_state::pick_logged_state(
                    grid_state.as_deref(),
                    qrz.as_ref().map(|q| q.state.as_str()),
                    qrz.as_ref().map(|q| q.grid.as_str()),
                    Some(record.gridsquare.as_str()),
                    portable,
                ) {
                    record.state = picked;
                }
            }
        }
        match self.db.insert_qso(&record) {
            Ok(id) => {
                // Real bug, found live (user asked to audit the sync flags
                // specifically): `record.id` was left `None` here --
                // `upload_and_mark_one`'s success path is `if let Some(id) =
                // q.id { mark_synced(id, "qrz") }`, so every auto-upload-on-
                // log QRZ submission actually reached QRZ (it uploads by
                // content, not by id) but could never mark the local row
                // `synced_qrz = 1` -- it silently no-op'd on `None` every
                // time. The row only ever got fixed if the operator later
                // ran a manual "Sync to logging services" pass, which
                // re-uploads it (QRZ replies "duplicate," harmless) and
                // *that* path's record came from a DB query that already has
                // a real id. Filling it in here, right after insert, closes
                // the gap at the source instead of relying on that manual
                // catch-up.
                record.id = Some(id);
                self.broadcast_qso(&record);
                // Running ADIF log mirror (ft8af_log.adi in Downloads),
                // ported from Android's `AdifLogFile.java` -- gated on
                // `enableAdifExport`, defaulting to on (Android's own
                // default) since this is new desktop functionality with
                // no prior behavior to preserve. Swallows its own errors.
                let adif_log_enabled = self.db.get_config("enableAdifExport").map(|v| v != "0").unwrap_or(true);
                adif_log_file::log_qso(adif_log_enabled, &crate::db::adif_record(&record));
                // Fork: auto-upload to QRZ's Logbook the moment this QSO is
                // logged (gated on `enableQRZ`), matching
                // `MainViewModel.java`'s post-insert background thread --
                // spawned so a slow/blocked network call never stalls the
                // engine's own run loop.
                if self.db.get_config(qrz::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let api_key = self.db.get_config(qrz::CONFIG_API_KEY).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        qrz::upload_one_logged(&db, &api_key, &record_for_upload);
                    });
                }
                // Same auto-upload-on-log hook, for Cloudlog/Wavelog/Nextlog
                // and World Radio League -- see `cloudlog.rs`/`wrl.rs`'s own
                // top doc comments.
                if self.db.get_config(cloudlog::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let address = self.db.get_config(cloudlog::CONFIG_ADDRESS).unwrap_or_default();
                    let api_key = self.db.get_config(cloudlog::CONFIG_API_KEY).unwrap_or_default();
                    let station_id = self.db.get_config(cloudlog::CONFIG_STATION_ID).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        cloudlog::upload_one_logged(&db, &address, &api_key, &station_id, &record_for_upload);
                    });
                }
                if self.db.get_config(wrl::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let api_key = self.db.get_config(wrl::CONFIG_API_KEY).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        wrl::upload_one_logged(&db, &api_key, &record_for_upload);
                    });
                }
                // Same auto-upload-on-log hook, for HamQTH, Club Log and
                // HRDLog.net -- see `hamqth.rs`/`clublog.rs`/`hrdlog.rs`'s
                // own top doc comments.
                if self.db.get_config(hamqth::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let password = self.db.get_config(hamqth::CONFIG_PASSWORD).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        hamqth::upload_one_logged(&db, &record_for_upload.station_callsign, &password, &record_for_upload);
                    });
                }
                if self.db.get_config(clublog::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let token = self.db.get_config(clublog::CONFIG_TOKEN).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        clublog::upload_one_logged(&db, &token, &record_for_upload);
                    });
                }
                if self.db.get_config(hrdlog::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let code = self.db.get_config(hrdlog::CONFIG_CODE).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        hrdlog::upload_one_logged(&db, &record_for_upload.station_callsign, &code, &record_for_upload);
                    });
                }
                if self.db.get_config(eqsl::CONFIG_ENABLE).as_deref() == Some("1") {
                    let db = self.db.clone();
                    let username = self.db.get_config(eqsl::CONFIG_USERNAME).unwrap_or_default();
                    let password = self.db.get_config(eqsl::CONFIG_PASSWORD).unwrap_or_default();
                    let record_for_upload = record.clone();
                    std::thread::spawn(move || {
                        eqsl::upload_one_logged(&db, &username, &password, &record_for_upload);
                    });
                }
                self.emit(EngineEvent::QsoCompleted(record));
            }
            Err(e) => self.emit(EngineEvent::Error(format!("log QSO failed: {e}"))),
        }
    }

    /// `Mode::tx_latest_ms` (the configured PTT delay's eligibility window)
    /// widened by the configured "Late-start Tolerance" -- the single value
    /// `tx_slot_eligible`'s boundary check should use everywhere, so the
    /// late-start clip math downstream (already correct, already tested --
    /// see `late_start_tolerance_ms`'s own doc comment) is the only thing
    /// that changes when this setting is non-zero, not the eligibility
    /// logic itself.
    fn effective_tx_latest_ms(&self) -> i64 {
        self.mode.tx_latest_ms(self.ptt_delay_ms) + self.late_start_tolerance_ms
    }

    /// Key up this slot's QSO message if we're armed, it's our turn, and the
    /// 12.64 s waveform still fits before the next boundary. Idempotent per slot
    /// (`txed_slot` guard), so calling it from both `handle_decoded` and a command
    /// handler in the same slot transmits at most once.
    fn maybe_transmit(&mut self) {
        // SWR Halt's lockout blocks every new transmission -- CQ, a QSO
        // reply, or a courtesy 73 alike -- until explicitly cleared. Gating
        // here (the one place every activation path funnels through, see
        // StartCq/Answer/SetStage/FreeText) rather than at each of those
        // call sites: the operator's click still arms `qso` normally (the
        // QSO panel shows what it's waiting to send), it just never keys
        // until the lockout clears -- matches Android's
        // `setActivated(false)` in spirit without losing the click.
        if self.swr_lockout {
            return;
        }
        // ALWAYS the true, freshly-computed current slot -- never a caller-
        // supplied one. Real bug, confirmed live with two screenshots and a
        // full diagnostic trace: `handle_decoded`'s tail call used to pass
        // its OWN `slot_id` parameter straight through (correct for ITS
        // purpose -- decode/watchdog attribution of a specific slot's
        // audio, which must stay accurate even when decode processing
        // falls behind). But the CAT-tone TX path (`qmxCatTones`) blocks
        // this same thread for the FULL ~12.64s of a transmission, one
        // `rig_send_raw` per symbol -- so by the time a queued command or a
        // backlogged decode result finally gets processed, real time can
        // already be a full slot (or more) past the one it was labeled
        // for. `tx_slot_eligible`'s `into_cycle_ms` check can't tell the
        // difference: it's a bare modulo, so "106ms into slot 511" looks
        // exactly like "106ms into slot 510" -- it would let a transmission
        // key up believing it's still inside slot 510's window, while the
        // radio actually keys up during REAL slot 511, which can be the
        // SAME parity as the DX instead of opposite. Always deriving the
        // slot fresh here closes that gap: a stale caller value can at
        // worst cause a missed/retried check, never a mistimed key-up.
        let slot_id = self.cur_slot();
        // All the keying rules — active QSO, locked-parity alternation, once per
        // slot, and early enough in the cycle to fit the waveform — live in the pure
        // `tx_slot_eligible` so the run loop's boundary trigger shares them exactly.
        let into_cycle_ms_now = self.now().rem_euclid(self.mode.cycle_ms());
        let eligible = tx_slot_eligible(
            self.qso.active,
            self.tx_parity,
            slot_id,
            self.txed_slot,
            into_cycle_ms_now,
            self.effective_tx_latest_ms(),
        );
        if !eligible {
            return;
        }
        // Answering a CQ pins the parity to the operator's click slot so we always
        // reply opposite the DX; a CQ / free-text start locks it on this first TX.
        self.tx_parity = Some(slot_id.rem_euclid(2));
        self.txed_slot = slot_id;
        if let Some(msg) = self.qso.tx_message().map(|s| s.to_string()) {
            if self.start_transmit(&msg) {
                // The ONLY reliable "a real over has actually gone out this
                // run" signal -- see `has_transmitted_this_run`'s own doc
                // comment for why `tx_parity.is_none()` wasn't enough.
                self.has_transmitted_this_run = true;
                // Tell the sequencer the message actually went out. This is what
                // lets a courtesy "73" wrap the QSO up only after it's on the air;
                // if start_transmit bailed (busy/encode error) the QSO stays armed
                // and retransmits next slot instead of clobbering the 73. It also
                // returns a loggable outcome the instant RR73 is confirmed sent
                // (see QsoEngine::notify_transmitted's own doc comment) --
                // logged the same way as any other completion.
                if let Some(QsoOutcome::Completed(record)) = self.qso.notify_transmitted() {
                    self.restore_preferred_tx_parity_if_owed();
                    self.log_completed_qso(record);
                }
                // Publish the instant a real over actually keys -- matches
                // Android's `transmitSignal.mutableIsTransmitting.postValue(true)`,
                // called synchronously right here, not left to whichever
                // caller happens to publish next. Real gap, found live: of
                // maybe_transmit's three call sites, only two (StartCq's own
                // immediate attempt, handle_decoded's tail) already publish
                // afterward -- the one that fires most *real* CQ keyings (the
                // run loop's boundary trigger, used whenever the operator's
                // click misses the current slot's window and the first over
                // goes out on the next one instead) never did. The frontend's
                // TX log (activeQsoPanel.ts's decideTxLog) is keyed off the
                // rising edge of `transmitting`, so without this, that edge
                // could be missed or arrive late enough to look like it
                // belonged to a different slot than the one the carrier
                // actually keyed in.
                self.publish_tx_state();
            }
        }
    }

    /// The TX slot index we're currently in, by real UTC.
    fn cur_slot(&self) -> i64 {
        self.now().div_euclid(self.mode.cycle_ms())
    }

    /// Parity (0/1) of the slot we're currently in.
    fn cur_parity(&self) -> i64 {
        self.cur_slot().rem_euclid(2)
    }

    /// Put `preferred_tx_parity` back to its pre-excursion value if
    /// `Answer` left one pending -- call on every `QsoOutcome::Completed`
    /// or `QsoOutcome::GaveUpTarget`. See `preferred_tx_parity_restore`'s
    /// own doc comment for why this never touches `tx_parity` itself.
    fn restore_preferred_tx_parity_if_owed(&mut self) {
        if let Some(original) = self.preferred_tx_parity_restore.take() {
            self.preferred_tx_parity = original;
        }
    }

    /// Switch operating mode (FT8/FT4) -- see `EngineCommand::SetMode`.
    ///
    /// **Rejected outright while actually transmitting** (audio playing, a
    /// CAT-tone over in flight, or tuning) -- matches Android's
    /// `MainViewModel.setOperatingMode`, which refuses mid-transmit rather
    /// than interrupting: a mode's cycle length and tone encoding are baked
    /// into a transmission from the moment it starts (the CAT-tone path in
    /// particular captures `symbol_period` once into `CatToneTx`), so
    /// switching mid-transmission would desync the rig from what's actually
    /// still being clocked out -- and cutting the transmission off instead
    /// is its own hazard (a half-sent message on the air). An *armed but not
    /// currently keyed* QSO/CQ (`qso.active` true, waiting between slots) is
    /// fine to interrupt and is stopped as part of the switch, same as
    /// Android's `ft8TransmitSignal.setActivated(false)`. A no-op (but still
    /// safe to call) when `mode` is already the current mode.
    ///
    /// Retunes within the **same band** to the new mode's dial frequency
    /// (`bands::mode_band_freq`) -- never changes the band itself. Matches
    /// Android's own documented radio-safety rule: jumping to a band the
    /// operator hasn't tuned (antenna/ATU) could present high SWR and damage
    /// the PA, so a band with no dial in the new mode (160m/60m have no FT4
    /// allocation) is left exactly as-is rather than guessing a frequency.
    ///
    /// The decode worker thread is built for one protocol for its whole
    /// lifetime (see `spawn_decoder_thread`), so this tears the old one down
    /// (dropping `self.slot_tx` ends its `recv()` loop) and spawns a fresh
    /// one for the new mode. Scheduling state tied to slot boundaries
    /// (`last_decoded_slot`, `pending_decodes`, `txed_slot`, `last_wf_slot`)
    /// is reset too, since FT8's and FT4's slot grids don't line up --
    /// carrying over a stale slot id from the old cycle length could skip a
    /// decode or key a transmission on a bogus "already sent this slot" check.
    fn switch_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        if self.tx_playback.is_some() || self.tx_cat.is_some() || self.tx_tune.is_some() {
            self.emit(EngineEvent::Error(
                "can't switch mode while transmitting -- try again after this over".into(),
            ));
            return;
        }
        let was_active = self.qso.active;
        self.qso.stop();
        self.tx_parity = None;

        let (slot_tx, dec_rx) = spawn_decoder_thread(mode.ffi_protocol(), self.decode_deep);
        self.slot_tx = slot_tx;
        self.dec_rx = dec_rx;

        self.mode = mode;
        self.qso.mode = mode.display_name().to_string();
        self.last_decoded_slot = -1;
        self.pending_decodes = 0;
        self.pending_decode_slots.clear();
        self.txed_slot = -1;
        self.last_wf_slot = -1;

        // Retune within the same band only -- see doc comment above.
        let wave_length = bands::band_for_freq_hz(self.dial_hz);
        if let Some(new_hz) = bands::mode_band_freq(wave_length, mode) {
            if new_hz != self.dial_hz {
                self.dial_hz = new_hz;
                self.qso.band = bands::band_for_freq_hz(new_hz).to_string();
                self.qso.freq_mhz = bands::freq_mhz_string(new_hz);
                let _ = self.db.set_config("dial_hz", &new_hz.to_string());
                if let Some(rig) = self.rig.as_mut() {
                    let _ = rig.set_frequency(new_hz);
                }
            }
        }

        let _ = self.db.set_config("operating_mode", mode.display_name());
        self.emit(EngineEvent::Info(format!("switched to {}", mode.display_name())));
        self.clear_decodes_and_target(was_active);
        self.publish_rig_status();
    }

    /// Settings -> Transmission's "Clear decodes on band/mode change": wipe
    /// the UI's decode list and reset the TX target back to CQ, mirroring
    /// Android's `MainViewModel.clearDecodesAndTarget`/`resetToCQ`. `was_active`
    /// is the QSO engine's `active` flag from *before* the band/mode change
    /// (callers capture it before touching anything) -- CQ is only re-armed
    /// when something was actually running, so moving the dial while idle
    /// doesn't start transmitting out of nowhere. No-op (beyond publishing
    /// state) when the setting is off. Always publishes TX state so the UI's
    /// QSO panel reflects a cleared or re-armed target immediately.
    fn clear_decodes_and_target(&mut self, was_active: bool) {
        if self.clear_on_band_mode_change {
            if was_active {
                self.tx_parity = self.preferred_tx_parity;
                self.qso.start_cq();
            }
            self.emit(EngineEvent::ClearDecodeList);
        }
        self.publish_tx_state();
    }

    /// Begin transmitting `message`, returning immediately. Playback runs off the
    /// loop (in `tx_playback`) so the engine keeps draining commands — that's what
    /// makes Stop TX able to interrupt mid-transmission. PTT is dropped later, when
    /// the run loop sees the playback finish (or on Stop TX).
    ///
    /// Returns `true` if a transmission was actually started (PTT keyed, playback
    /// armed), `false` if it bailed out (already transmitting, or an encode /
    /// device / playback error). The caller uses this to tell the sequencer the
    /// message really went out.
    fn start_transmit(&mut self, message: &str) -> bool {
        // FT2 has no encoder anywhere (see Mode's doc comment) -- refuse before
        // touching PTT or the rig, rather than letting generate_signal silently
        // produce an unverified waveform. Covers both the audio path below and
        // start_transmit_cat_tones (QMX CAT-tone path), which this falls through
        // to. TUNE (a steady unmodulated carrier, no message encoding) is exempt
        // -- it's mode-independent and Android's own FT2 screen leaves it enabled.
        if !self.mode.can_transmit() {
            self.emit(EngineEvent::Error(format!(
                "{} is receive-only -- switch mode to transmit",
                self.mode.display_name()
            )));
            return false;
        }
        // Never key a second transmission over a live one, or over an active tune.
        if self.tx_playback.is_some() || self.tx_cat.is_some() || self.tx_tune.is_some() {
            return false;
        }
        if self.tx_via_cat_tones {
            return self.start_transmit_cat_tones(message);
        }
        let signal = match crate::dsp::encode::generate_signal(message, self.tx_audio_hz as f32, SAMPLE_RATE, self.mode) {
            Some(s) => s,
            None => {
                self.emit(EngineEvent::Error(format!("cannot encode '{message}'")));
                return false;
            }
        };

        // Do the slow work — device open + full-buffer resample — BEFORE keying
        // the rig, so PTT isn't held through it. On a weak CPU this resample is
        // hundreds of ms; running it after PTT is what produced the long
        // keyed-but-silent gap reported on the Inovato Quadra.
        let mut prepared = match output::prepare(
            self.output_device.as_deref(),
            &signal,
            self.tx_gain,
            self.tx_audio_channel,
        ) {
            Ok(p) => p,
            Err(e) => {
                self.emit(EngineEvent::Error(format!("playback failed: {e}")));
                return false;
            }
        };

        // Key up, let the rig's T/R relay settle, then start clocking samples.
        self.set_ptt(true);
        std::thread::sleep(Duration::from_millis(self.ptt_delay_ms as u64));

        // Clip leading audio only if we'd overrun the cycle (AGENTS.md gotcha:
        // ms_late = max(0, into_cycle - 2360), NOT into_cycle % 15000). Computed
        // here, right before audio starts, so it reflects the real lateness after
        // the PTT settle — `maybe_transmit`'s window check keeps this at 0 in
        // normal operation.
        let into_cycle = self.now().rem_euclid(self.mode.cycle_ms());
        let ms_late = (into_cycle - self.mode.audio_slack_ms()).max(0);
        if let Err(e) = prepared.start(ms_late) {
            self.emit(EngineEvent::Error(format!("playback failed: {e}")));
            self.set_ptt(false);
            return false;
        }
        self.tx_playback = Some(prepared);
        self.publish_tx_state();
        true
    }

    /// The QMX CAT-tone TX path: no audio at all, just a `TA` command per
    /// symbol sent from [`Engine::service_cat_tx`] on the run loop's own
    /// tick. See [`CatToneTx`] and [`crate::qmx_cat_tones`].
    fn start_transmit_cat_tones(&mut self, message: &str) -> bool {
        let payload = match encode::pack77(message) {
            Some(p) => p,
            None => {
                self.emit(EngineEvent::Error(format!("cannot encode '{message}'")));
                return false;
            }
        };
        let tones_vec = encode::encode_tones_mode(&payload, self.mode);
        let symbol_period = self.mode.symbol_period();
        let mut tones = [0u8; encode::FT4_NN];
        tones[..tones_vec.len()].copy_from_slice(&tones_vec);

        // Key up, let the rig's T/R relay settle -- same as the audio path.
        self.set_ptt(true);
        std::thread::sleep(Duration::from_millis(self.ptt_delay_ms as u64));

        // Same lateness math as the audio path (AGENTS.md gotcha: ms_late =
        // max(0, into_cycle - audio_slack_ms), NOT into_cycle % cycle_ms), just
        // converted to whole symbols instead of samples.
        let into_cycle = self.now().rem_euclid(self.mode.cycle_ms());
        let ms_late = (into_cycle - self.mode.audio_slack_ms()).max(0);
        let skip_until = cat_tones::symbols_to_skip(ms_late, symbol_period, tones_vec.len());

        self.tx_cat = Some(CatToneTx {
            tones,
            len: tones_vec.len(),
            symbol_period,
            base_hz: self.tx_audio_hz as f64,
            start: Instant::now(),
            sent_through: skip_until,
            skip_until,
        });
        self.publish_tx_state();
        true
    }

    /// Start transmitting a steady carrier at the TX offset for antenna
    /// tuning — no FT8 message, no auto-release; the operator (or the Tune
    /// button going off) stops it explicitly via `finalize_tx`. Refuses to
    /// start over a live transmission or an already-active tune, the same
    /// reentrancy rule `start_transmit` applies to itself.
    fn start_tune(&mut self) {
        if self.tx_playback.is_some() || self.tx_cat.is_some() || self.tx_tune.is_some() {
            return;
        }
        // SWR Halt's lockout blocks a new Tune exactly like a new TX --
        // matches Android's own `FT8TransmitSignal.startTune` refusing while
        // `isSwrLocked()` (a high-SWR fault doesn't go away just because the
        // operator wants to try tuning again; they must clear it first).
        if self.swr_lockout {
            self.emit(EngineEvent::Error(
                "SWR protection lockout is active -- clear it in Settings before tuning".into(),
            ));
            return;
        }
        // Tune method "Internal tuner": refuse rather than key a surprise
        // carrier when no rig ATU-start path is available -- see
        // `tx_protection::decide_tune_action`'s doc comment on why
        // `atu_available` is always `false` here today.
        if tx_protection::decide_tune_action(self.tune_method, false) == tx_protection::TuneAction::Unavailable {
            self.emit(EngineEvent::Error(
                "Tune method is set to \"Internal tuner\", but no connected rig exposes one over CAT yet".into(),
            ));
            return;
        }
        self.set_ptt(true);
        std::thread::sleep(Duration::from_millis(self.ptt_delay_ms as u64));
        let tune_gain = effective_tune_gain(self.tune_level_independent, self.tune_gain, self.tx_gain);
        if self.tx_via_cat_tones {
            if let Some(rig) = self.rig.as_mut() {
                if let Err(e) = rig.send_raw_cat(&cat_tones::ta_command(self.tx_audio_hz as f64)) {
                    self.emit(EngineEvent::Error(format!("tune failed: {e}")));
                    self.set_ptt(false);
                    return;
                }
            }
            self.tx_tune = Some(TuneHandle::Cat);
        } else {
            match output::prepare_tune(self.output_device.as_deref(), self.tx_audio_hz as f32, tune_gain, self.tx_audio_channel) {
                Ok(h) => self.tx_tune = Some(TuneHandle::Audio(h)),
                Err(e) => {
                    self.emit(EngineEvent::Error(format!("tune failed: {e}")));
                    self.set_ptt(false);
                    return;
                }
            }
        }
        self.tune_started_at = Some(Instant::now());
        self.publish_tx_state();
    }

    /// Push the operator's current TX level to an already-running audio Tune
    /// (a no-op if Tune isn't active, or is active via CAT tones -- the QMX's
    /// CAT-tone drive has no analog "volume" to adjust, confirmed by the
    /// operator: "on the QMX it is always set to 100%"). Real gap, found
    /// live: riding an FT-857D's ALC, walking a manual tuner, or letting an
    /// ATU cycle all need the level adjustable *while the carrier is already
    /// keyed* -- stopping Tune to change a Settings value and starting it
    /// again would drop the carrier mid-cycle. Mirrors `start_tune`'s own
    /// `tune_gain` selection exactly (independent tune level vs. the normal
    /// TX gain), so whichever one the operator is actually adjusting is the
    /// one that reaches the rig, regardless of which slider they're on.
    fn apply_live_tune_gain(&mut self) {
        if let Some(TuneHandle::Audio(tp)) = &self.tx_tune {
            tp.set_gain(effective_tune_gain(self.tune_level_independent, self.tune_gain, self.tx_gain));
        }
    }

    /// Send any CAT-tone symbols whose scheduled time has arrived, and finish
    /// the over (key up, release PTT) once the last one has been held for its
    /// full period. A no-op when no CAT-tone transmission is in flight. Called
    /// every run-loop tick; see [`CatToneTx`]'s doc comment for why this runs
    /// here rather than on a dedicated timer thread.
    fn service_cat_tx(&mut self) {
        let Some(mut cat) = self.tx_cat else { return };
        let elapsed = cat.start.elapsed();
        while cat.sent_through < cat.len {
            let since_start = cat.sent_through - cat.skip_until;
            let due_at = Duration::from_secs_f32(cat.symbol_period) * since_start as u32;
            if elapsed < due_at {
                break;
            }
            let idx = cat.sent_through;
            cat.sent_through += 1;
            let hz = cat_tones::tone_hz(cat.base_hz, cat.tones[idx], cat.symbol_period);
            if let Some(rig) = self.rig.as_mut() {
                if let Err(e) = rig.send_raw_cat(&cat_tones::ta_command(hz)) {
                    self.emit(EngineEvent::Error(format!("CAT tone send failed: {e}")));
                }
            }
        }
        // Hold the final tone for its full symbol period before keying up,
        // instead of cutting it short the instant it's sent -- matches the
        // audio path, whose waveform already ends with a full-length symbol.
        let total_since_start =
            Duration::from_secs_f32(cat.symbol_period) * (cat.len - cat.skip_until) as u32;
        let finished = cat.sent_through >= cat.len && elapsed >= total_since_start;
        self.tx_cat = Some(cat);
        if finished {
            self.finalize_tx();
        }
    }

    /// Stop whatever's transmitting (if anything) — a normal over (audio or
    /// CAT-tone) or a TUNE: dropping an audio handle halts its cpal stream
    /// immediately; a CAT-tone over or a CAT tune gets an explicit key-up
    /// (`TA0;`) since there's no stream to drop. PTT goes down either way.
    /// Safe to call when nothing is transmitting. Shared by Stop TX and the
    /// Tune button's "off" side.
    fn finalize_tx(&mut self) {
        let had_audio = self.tx_playback.take().is_some();
        let had_cat = self.tx_cat.take().is_some();
        let had_tune = self.tx_tune.take();
        self.tune_started_at = None;
        let tune_was_cat = matches!(had_tune, Some(TuneHandle::Cat));
        if had_cat || tune_was_cat {
            // A CAT-tone transmission doesn't get polled live (see the 1Hz
            // tick block's comment on why) -- take one last reading here
            // instead, while RF from the final symbol is still live, just
            // before the key-up command below silences it. Only sets the
            // lockout for the *next* over (see note_swr_reading) -- this one
            // is already ending, so there's nothing left here to abort.
            if had_cat && self.rig_supports_meters {
                let swr = self.poll_meters();
                self.note_swr_reading(swr);
            }
            if let Some(rig) = self.rig.as_mut() {
                // Best-effort: a failed key-up here has already failed every
                // other CAT call this over (send_raw_cat marks the rig
                // disconnected on error), so there's nothing more to retry.
                let _ = rig.send_raw_cat(cat_tones::KEY_UP);
            }
        }
        // had_tune, if TuneHandle::Audio, drops its TunePlayback here (end of
        // scope), halting the looping carrier.
        if had_audio || had_cat || had_tune.is_some() {
            // The courtesy 73's transmission just finished (audio or
            // CAT-tone, never Tune -- Tune never touches qso.rs at all) --
            // wrap the QSO up right now rather than waiting for the next
            // decoded slot's process_rx to notice. Real UX bug, found live:
            // that wait cost the operator 2-3 missed decode cycles after
            // every single QSO. A no-op unless we're genuinely still at
            // Bye73 with the message confirmed sent, so this is harmless
            // after any other kind of transmission.
            if had_audio || had_cat {
                self.qso.finish_if_bye73_sent();
            }
            self.set_ptt(false);
            self.publish_tx_state();
        }
    }

    /// One SWR/power reading over CAT (QMX only -- gated by
    /// `rig_supports_meters`, checked by callers). Emits a `Meter` event only
    /// if at least one of the two queries returned a valid reading; a failed
    /// or unparseable reply just means no event this call, not an error.
    ///
    /// A valid power reading also updates the operator's `powerWatts`
    /// Station setting -- ported from the Android fork's
    /// `updateOperatorPowerFromRig`/`persistOperatorPowerIfChanged`: for a
    /// radio whose power meter this app reads (just the QMX today), the
    /// configured power follows the rig's own measurement instead of
    /// standing at whatever the operator once typed in Settings. Desktop
    /// polls once per over (right here, just before key-up) rather than
    /// continuously while keyed, so there is no separate "unkey" moment to
    /// defer the save to -- this reading *is* that moment, and comparing
    /// against the persisted value (via `set_power_watts_if_changed`)
    /// provides the same "don't rewrite on every over if the reading is
    /// stable" dedup Android's own save-on-unkey gets from a different angle.
    fn poll_meters(&mut self) -> Option<f32> {
        let Some(rig) = self.rig.as_mut() else { return None };
        let swr = rig
            .send_raw_cat_query(qmx_meters::SWR_QUERY, qmx_meters::REPLY_TERM)
            .ok()
            .and_then(|r| qmx_meters::parse_swr(&r));
        let power = rig
            .send_raw_cat_query(qmx_meters::POWER_QUERY, qmx_meters::REPLY_TERM)
            .ok()
            .and_then(|r| qmx_meters::parse_power_watts(&r));
        if swr.is_some() || power.is_some() {
            self.emit(EngineEvent::Meter(MeterEvent { swr_ratio: swr, power_watts: power }));
        }
        if let Some(watts) = power {
            if watts > 0.0 {
                let text = format_watts(watts);
                if self.db.get_config("powerWatts").as_deref() != Some(text.as_str()) {
                    let _ = self.db.set_config("powerWatts", &text);
                }
            }
        }
        swr
    }

    /// Check an SWR reading against the halt threshold and set the lockout
    /// if it fires -- ported from Android's `MeterProtectionController.
    /// onMeterUpdate`'s SWR half (ALC sampling is handled separately, see
    /// `tx_protection`'s module doc on why desktop has nothing to feed it
    /// yet). Deliberately does **not** itself stop any transmission: the
    /// periodic meter tick aborts a live `tx_playback`/`tx_tune` right after
    /// calling this (see that call site); the `finalize_tx` call site (a
    /// CAT-tone over's final reading) only needs the lockout set for the
    /// *next* over, since that over is already ending -- and calling
    /// `finalize_tx` again from in here, while already inside it, would be
    /// real re-entrancy.
    fn note_swr_reading(&mut self, swr: Option<f32>) {
        if self.swr_lockout {
            return; // already locked out; nothing new to do until cleared
        }
        let tuning = self.tx_tune.is_some();
        if tx_protection::should_halt_for_swr(swr, self.swr_halt_enabled, self.swr_halt_threshold, tuning) {
            self.swr_lockout = true;
            // Matches Android's `haltForSwr` calling `setActivated(false)`:
            // a high-SWR fault ends the whole auto-sequence (CQ or QSO),
            // not just this one over -- the operator must explicitly clear
            // the lockout and re-arm, not just wait for the next slot.
            self.qso.stop();
            self.tx_parity = None;
            self.emit(EngineEvent::Error(format!(
                "SWR protection: TX halted (SWR {:.1}, threshold {:.1}) -- clear the lockout in Settings to resume",
                swr.unwrap_or(0.0),
                self.swr_halt_threshold
            )));
            self.publish_tx_state();
        }
    }

    fn set_ptt(&mut self, on: bool) {
        if let Some(rig) = self.rig.as_mut() {
            if let Err(e) = rig.set_ptt(on) {
                self.emit(EngineEvent::Error(format!("PTT failed: {e}")));
            }
        }
        self.ptt = on;
        self.last_ptt_on_sent = if on { Some(Instant::now()) } else { None };
        self.publish_rig_status();
    }

    fn refresh_ptt_keepalive(&mut self) {
        let required = self
            .rig
            .as_ref()
            .map(|rig| rig.requires_ptt_keepalive())
            .unwrap_or(false);
        let since_last_on = self.last_ptt_on_sent.map(|t| t.elapsed());
        if !should_refresh_ptt(self.ptt, required, since_last_on) {
            return;
        }

        let result = self.rig.as_mut().map(|rig| rig.set_ptt(true));
        self.last_ptt_on_sent = Some(Instant::now());
        if let Some(Err(e)) = result {
            // The rig will drop PTT on its own within ~2 s, so don't keep
            // playing audio into an unkeyed radio or re-error every second:
            // end this over (stops playback, sends a best-effort PTT-off).
            self.emit(EngineEvent::Error(format!("PTT keepalive failed: {e}")));
            self.finalize_tx();
            if self.ptt {
                self.set_ptt(false);
            }
        }
    }

    /// Compute one windowed-FFT spectrum row from the most recent audio and emit
    /// it for the scrolling waterfall. Color is auto-scaled relative to a smoothed
    /// noise-floor estimate so the floor stays blue regardless of input gain.
    fn emit_waterfall_row(&mut self) {
        let samples = self.accum.peek_recent(self.wf.samples_needed());
        let Some((bins, hz_per_col)) = self.wf.process_row(&samples, SAMPLE_RATE as f32) else {
            return; // still warming up
        };
        // Mark the first row at/after each UTC cycle boundary, matching the
        // fixed clock used for decoder windows.
        let boundary = wf_boundary_row(self.now(), &mut self.last_wf_slot, self.mode.cycle_ms());
        self.emit(EngineEvent::WaterfallRow(WaterfallRow { bins, hz_per_col, boundary }));
    }

    /// `slot_utc_ms` is the START of the slot this decode was actually
    /// captured for (`slot_id * cycle_ms`, using the `pending_decode_slots`
    /// -corrected `slot_id` -- see its own doc comment), NOT `self.now()`
    /// at publish time. Real bug, confirmed live with two screenshots: the
    /// decode worker runs on its own thread and is only polled
    /// opportunistically, so a result landing late enough to cross a slot
    /// boundary used to get stamped with whatever time it happened to be
    /// *picked up*, one slot later than when it was actually transmitted
    /// -- "the message box shows him calling CQ at the :00 time slot but
    /// he actually called in the previous :45 slot when looking at the
    /// waterfall." This `UiMessage.utc_ms` is also what `AnswerArgs::
    /// msg_utc_ms` derives the reply parity from -- a stale timestamp here
    /// would have silently defeated that fix too.
    fn publish_decodes(&self, decoded: &[DecodedMessage], slot_utc_ms: i64) {
        let now = slot_utc_ms;
        // Callsign/grid cache for the (not yet built) Map screen: any decode
        // that actually resolved a grid (m.grid is only non-empty when
        // dsp/decoder.rs's looks_like_grid passed -- never a signal report,
        // RRR, or 73) is worth recording, regardless of who it's addressed
        // to. Live-heard data always wins over ADIF/FCC backfill, see
        // Db::upsert_callsign_qth_live's doc comment.
        for m in decoded {
            if !m.grid.is_empty() {
                self.db.upsert_callsign_qth_live(&m.call_from, &m.grid);
            }
        }
        let ui: Vec<UiMessage> = decoded
            .iter()
            .map(|m| {
                let dxcc = crate::dxcc::lookup(&m.call_from);
                UiMessage {
                    utc_ms: now,
                    call_to: m.call_to.clone(),
                    call_from: m.call_from.clone(),
                    extra: m.extra.clone(),
                    grid: m.grid.clone(),
                    snr: m.snr,
                    freq_hz: m.freq_hz,
                    time_sec: m.time_sec,
                    text: decode_text(m),
                    is_cq: crate::callsign::is_cq_callsign(&m.call_to),
                    to_me: crate::callsign::is_my_callsign(&m.call_to, &self.qso.my_call),
                    dxcc_prefix: dxcc.as_ref().map(|d| d.dxcc_prefix.clone()),
                    cq_zone: dxcc.as_ref().map(|d| d.cq_zone),
                    state: crate::us_states::state_for_grid(&m.grid),
                }
            })
            .collect();
        self.emit(EngineEvent::Decoded(ui));
    }

    // --- WSJT-X UDP broadcast (outbound) ------------------------------------

    /// Assemble a WSJT-X Status snapshot from current engine state.
    fn udp_status(&self) -> crate::udp::codec::Status {
        let st = self.qso.status();
        crate::udp::codec::Status {
            dial_freq_hz: self.dial_hz,
            mode: "FT8".to_string(),
            dx_call: st.target.clone().unwrap_or_default(),
            report: st.report_sent.map(|n| format!("{n:+03}")).unwrap_or_default(),
            tx_mode: "FT8".to_string(),
            tx_enabled: st.active,
            transmitting: self.tx_playback.is_some(),
            decoding: self.decoding,
            rx_df: self.tx_audio_hz.max(0) as u32,
            tx_df: self.tx_audio_hz.max(0) as u32,
            de_call: self.qso.my_call.clone(),
            de_grid: self.qso.my_grid.clone(),
            dx_grid: String::new(),
            tx_watchdog: false,
            sub_mode: String::new(),
            fast_mode: false,
            special_op_mode: 0,
            freq_tolerance: 0xFFFF_FFFF,
            tr_period: 15,
            config_name: "Default".to_string(),
            tx_message: st.tx_message.clone().unwrap_or_default(),
        }
    }

    fn broadcast_status(&self) {
        if self.udp.is_enabled() {
            self.udp.send(&crate::udp::codec::status(&self.udp_status()));
        }
    }

    fn broadcast_heartbeat(&self) {
        if self.udp.is_enabled() {
            self.udp
                .send(&crate::udp::codec::heartbeat(env!("CARGO_PKG_VERSION"), ""));
        }
    }

    /// Broadcast each of a slot's decodes as a WSJT-X Decode message. The `Time`
    /// field is ms-since-UTC-midnight; `Frequency` is the audio offset in Hz.
    fn broadcast_decodes(&mut self, decoded: &[DecodedMessage]) {
        if !self.udp.is_enabled() {
            return;
        }
        let time_ms = self.now().rem_euclid(86_400_000) as u32;
        for m in decoded {
            self.udp.send_decode(crate::udp::codec::Decode {
                is_new: true,
                time_ms,
                snr: m.snr,
                delta_time: m.time_sec as f64,
                delta_freq: m.freq_hz.max(0.0) as u32,
                mode: "FT8".to_string(),
                message: decode_text(m),
                low_confidence: false,
                off_air: false,
            });
        }
    }

    /// Broadcast a logged QSO as both the structured QSO-Logged message and the
    /// Logged-ADIF message (loggers consume one or the other).
    fn broadcast_qso(&mut self, record: &QsoRecord) {
        if !self.udp.is_enabled() {
            return;
        }
        use crate::udp::codec::{qso_logged, DateTime, QsoLogged};
        // Actual TX RF frequency = dial + audio offset (what WSJT-X reports).
        let tx_freq_hz = self.dial_hz + self.tx_audio_hz.max(0) as u64;
        let q = QsoLogged {
            time_off: DateTime::from_adif(&record.qso_date_off, &record.time_off),
            dx_call: record.call.clone(),
            dx_grid: record.gridsquare.clone(),
            tx_freq_hz,
            mode: record.mode.clone(),
            report_sent: record.rst_sent.clone(),
            report_received: record.rst_rcvd.clone(),
            tx_power: String::new(),
            comments: record.comment.clone(),
            name: String::new(),
            time_on: DateTime::from_adif(&record.qso_date, &record.time_on),
            operator_call: record.station_callsign.clone(),
            my_call: record.station_callsign.clone(),
            my_grid: record.my_gridsquare.clone(),
            exchange_sent: String::new(),
            exchange_received: String::new(),
            prop_mode: String::new(),
        };
        self.udp.send(&qso_logged(&q));
        self.udp
            .send(&crate::udp::codec::logged_adif(&crate::db::adif_record(record)));
    }

    fn publish_tx_state(&self) {
        self.emit(EngineEvent::TxState(TxStateEvent {
            // Derived from the live handles so the badge can't get stuck:
            // it's TX exactly while a transmission (any kind, including a
            // tune) is in flight.
            transmitting: self.tx_playback.is_some() || self.tx_cat.is_some() || self.tx_tune.is_some(),
            message: self.qso.tx_message().map(|s| s.to_string()),
            status: self.qso.status(),
            tuning: self.tx_tune.is_some(),
            preferred_tx_parity: self.preferred_tx_parity,
            swr_lockout: self.swr_lockout,
        }));
        // Mirror the state change out over WSJT-X UDP so companion apps see TX
        // start/stop and QSO progress without waiting for the next 1 Hz tick.
        self.broadcast_status();
    }

    /// Sanitize, persist, and apply a new `WfConfig` -- shared by
    /// `SetWaterfallConfig` (a complete config from the Settings editor) and
    /// `SetWfDisplayHz`/`SetWfDenoise` (the waterfall strip's own Width/NR
    /// controls, which merge a single field into `self.wf.config()` first
    /// so the operator doesn't need to know the other knobs to change one).
    /// Rebuilds the FFT plan + window table; the noise-floor EMA restarts
    /// and reconverges within a couple of seconds.
    fn apply_wf_config(&mut self, cfg: WfConfig) {
        let cfg = cfg.sanitize();
        let _ = self.db.set_config("wf_window", cfg.window.as_str());
        let _ = self.db.set_config("wf_fft_size", &cfg.fft_size.to_string());
        let _ = self.db.set_config("wf_avg", &cfg.avg.to_string());
        let _ = self.db.set_config("wf_display_hz", &cfg.display_hz.to_string());
        let _ = self.db.set_config("wf_denoise", if cfg.denoise { "true" } else { "false" });
        self.wf = WfProcessor::new(cfg);
        self.emit(EngineEvent::Info(format!(
            "waterfall: {} {} avg {} width {}Hz{}",
            cfg.window.as_str(),
            cfg.fft_size,
            cfg.avg,
            cfg.display_hz as i32,
            if cfg.denoise { " NR" } else { "" },
        )));
    }

    fn publish_rig_status(&self) {
        let (connected, model) = match &self.rig {
            Some(r) => (r.connected(), r.name()),
            None => (false, "None".to_string()),
        };
        self.emit(EngineEvent::RigStatus(RigStatusEvent {
            connected,
            model,
            frequency_hz: self.dial_hz,
            ptt: self.ptt,
            tx_via_cat_tones: self.tx_via_cat_tones,
            tx_audio_channel: self.tx_audio_channel,
        }));
    }

    fn publish_clock_sync(&self) {
        self.emit(EngineEvent::ClockSync(ClockSyncEvent {
            offset_ms: self.clock_offset_ms,
            synced: self.time_synced,
            rx_offset_ms: self.rx_offset_ms,
        }));
    }

    fn emit(&self, e: EngineEvent) {
        let _ = self.evt.send(e);
    }

    /// Adopt an inbound WSJT-X Reply's requested `df` as the **session** TX audio
    /// offset — in-memory only. The value arrives on an untrusted UDP socket, so
    /// this deliberately does *not* write through to the persisted `base_freq`
    /// config: only explicit operator actions (SetBaseFreq / the UI) change the
    /// saved offset, and a datagram can't survive a restart. A `0`/unspecified or
    /// out-of-band `df` is ignored (current offset kept). Returns whether the
    /// session offset changed.
    fn apply_reply_df(&mut self, delta_freq: u32) -> bool {
        match reply_tx_audio_hz(delta_freq) {
            Some(hz) => {
                self.tx_audio_hz = hz;
                true
            }
            None => false,
        }
    }

    /// Shared tail of `EngineCommand::Answer` and `AnswerQueuedCaller`:
    /// apply the requested TX offset, derive reply parity, update the
    /// parity preference/restore bookkeeping, and hand the target to
    /// `qso.answer()` or `qso.answer_caller()`. `msg_utc_ms` is the DX's
    /// own decoded-slot timestamp (see `AnswerArgs::msg_utc_ms`'s doc
    /// comment for why this matters more than it looks like it should) --
    /// `None` falls back to the current slot. `from_caller_queue` selects
    /// which `qso.rs` entry point applies: `answer()` is for breaking into
    /// someone ELSE's CQ bulletin (we have nothing from them yet, so we
    /// send our own grid first); `answer_caller()` is for a station that
    /// already called US (a caller-queue chip), which already told us
    /// what we need to reply with a signal report immediately -- see
    /// `answer_caller`'s own doc comment for the live bug this fixes.
    fn answer_message(&mut self, msg: DecodedMessage, delta_freq: u32, msg_utc_ms: Option<i64>, from_caller_queue: bool) {
        // Honor the audio tone the companion/caller asked us to answer on
        // (WSJT-X's `df`) for *this session* when it carries a plausible
        // one, so we key up on the requested offset rather than the
        // current TX tone — matching the Android/iOS ports. This is an
        // in-memory-only change: it must never rewrite the operator's
        // persisted `base_freq` (only SetBaseFreq / the UI do that, and
        // the saved offset survives a restart). The desktop UI's own QSO
        // Sheet "Call" button sends df=0 (offset unchanged) when "Hold TX
        // freq" is on (the default, matching every build before that
        // setting existed), or the target's own decoded frequency when
        // off.
        self.apply_reply_df(delta_freq);
        // Reply opposite the DX. Prefer deriving this from his OWN
        // decoded slot (see `AnswerArgs::msg_utc_ms`'s doc comment --
        // this is the live bug fix for "every transmission after the
        // first was in the wrong slot" / "he is transmitting in an
        // odd slot but so am i") -- only fall back to "whatever slot
        // we're in right now" when no timestamp was supplied.
        let reply_parity = match msg_utc_ms {
            Some(his_ms) => {
                let his_parity = his_ms.div_euclid(self.mode.cycle_ms()).rem_euclid(2);
                1 - his_parity
            }
            None => self.cur_parity(),
        };
        self.tx_parity = Some(reply_parity);
        // Also update the *preference* (not just this run's lock), so a
        // CQ started mid-excursion (e.g. manually abandoning this target
        // to answer a different one) keeps using whichever slot that
        // required -- reverted back to the pre-excursion value once this
        // excursion actually ends (QSO completed or a watchdog gives up),
        // see `preferred_tx_parity_restore`'s own doc comment. Don't
        // recapture the restore point if one's already pending: the
        // ORIGINAL value has to survive however many stations this
        // excursion spans, not just the most recent one.
        if self.preferred_tx_parity_restore.is_none() {
            self.preferred_tx_parity_restore = Some(self.preferred_tx_parity);
        }
        self.preferred_tx_parity = Some(reply_parity);
        self.has_transmitted_this_run = false;
        if from_caller_queue {
            self.qso.answer_caller(&msg);
        } else {
            self.qso.answer(&msg);
        }
        // Fork: look the partner up on QRZ now, so its state is
        // cached by the time the QSO is logged. Mirrors
        // `FT8TransmitSignal.java`'s one real call site.
        self.qrz_xml.prefetch(&self.db, &msg.call_from);
        self.maybe_transmit();
        self.publish_tx_state();
    }

    fn handle(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::StartDecode => self.start_decode(),
            EngineCommand::StopDecode => {
                self.decoding = false;
                self.input = None;
                // No slot is in flight once capture stops; clear the count so a
                // later CQ can key at its slot boundary without being blocked. Any
                // batch still in the channel decrements from 0 via saturating_sub.
                self.pending_decodes = 0;
                self.pending_decode_slots.clear();
                // Tell companion apps to wipe their decode band and forget the
                // replay history — this is a genuine reset, not a per-cycle churn.
                if self.udp.is_enabled() {
                    self.udp.send(&crate::udp::codec::clear());
                }
                self.udp.clear_replay_cache();
                self.broadcast_status();
                self.emit(EngineEvent::Info("decode stopped".into()));
            }
            EngineCommand::SetDecodeSuspended(suspended) => {
                self.decode_suspended = suspended;
            }
            EngineCommand::SetDecodeDepth(deep) => {
                self.decode_deep = deep;
                let _ = self.db.set_config("decodeDeep", if deep { "1" } else { "0" });
                let (slot_tx, dec_rx) = spawn_decoder_thread(self.mode.ffi_protocol(), deep);
                self.slot_tx = slot_tx;
                self.dec_rx = dec_rx;
            }
            EngineCommand::SetStation { call, grid } => {
                // Persist the operator's full-precision grid (e.g. an 8-char
                // extended square) for display/distance purposes -- matches
                // Android's GeneralVariables.setMyMaidenheadGrid, which never
                // truncates the stored value. Only the live FT8 exchange
                // needs the 4-char locator (the protocol's fixed-width grid
                // field), so qso.my_grid alone is truncated via grid4;
                // on restart, QsoEngine::new re-derives the same truncation
                // from this same full-precision config value.
                let grid_full = grid.to_uppercase();
                self.qso.my_call = call.to_uppercase();
                self.qso.my_grid = util::grid4(&grid_full);
                let _ = self.db.set_config("my_call", &self.qso.my_call);
                let _ = self.db.set_config("my_grid", &grid_full);
            }
            EngineCommand::SetBand(hz) => {
                self.dial_hz = hz;
                self.qso.band = bands::band_for_freq_hz(hz).to_string();
                self.qso.freq_mhz = bands::freq_mhz_string(hz);
                let _ = self.db.set_config("dial_hz", &hz.to_string());
                if let Some(rig) = self.rig.as_mut() {
                    let _ = rig.set_frequency(hz);
                }
                // New band = new decode context: clear companions' band + replay
                // history and push the fresh dial frequency in a Status.
                if self.udp.is_enabled() {
                    self.udp.send(&crate::udp::codec::clear());
                }
                self.udp.clear_replay_cache();
                self.broadcast_status();
                self.publish_rig_status();
                self.clear_decodes_and_target(self.qso.active);
            }
            EngineCommand::SetBaseFreq(hz) => {
                self.tx_audio_hz = hz.clamp(MIN_TX_AUDIO_HZ, MAX_TX_AUDIO_HZ);
                let _ = self.db.set_config("base_freq", &self.tx_audio_hz.to_string());
            }
            EngineCommand::SetTxGain(g) => {
                self.tx_gain = clamp_tx_gain(g);
                let _ = self.db.set_config("tx_gain", &self.tx_gain.to_string());
                // Live while actively tuning, same reasoning as SetTuneLevel
                // below -- see apply_live_tune_gain's own doc comment.
                self.apply_live_tune_gain();
            }
            EngineCommand::SetRxGain(g) => {
                self.rx_gain = clamp_rx_gain(g);
                let _ = self.db.set_config("rx_gain", &self.rx_gain.to_string());
                // Applied live -- no capture restart, unlike changing the
                // device itself. Some bands are noisier than others, so this
                // is expected to be adjusted often while decoding.
                if let Some(input) = &self.input {
                    input.set_gain(self.rx_gain);
                }
            }
            EngineCommand::SetWaterfallConfig(cfg) => self.apply_wf_config(cfg),
            EngineCommand::SetWfDisplayHz(hz) => {
                let mut cfg = self.wf.config();
                cfg.display_hz = hz;
                self.apply_wf_config(cfg);
            }
            EngineCommand::SetWfDenoise(denoise) => {
                let mut cfg = self.wf.config();
                cfg.denoise = denoise;
                self.apply_wf_config(cfg);
            }
            EngineCommand::SetUdpConfig(cfg) => {
                // Persist the four settings, then rebind live.
                let _ = self.db.set_config("udp_enabled", if cfg.enabled { "true" } else { "false" });
                let _ = self.db.set_config("udp_host", &cfg.host);
                let _ = self.db.set_config("udp_port", &cfg.port.to_string());
                let _ = self.db.set_config(
                    "udp_accept_requests",
                    if cfg.accept_requests { "true" } else { "false" },
                );
                match self.udp.apply(cfg) {
                    Ok(msg) => {
                        // Prime companions immediately on enable.
                        self.broadcast_heartbeat();
                        self.broadcast_status();
                        self.emit(EngineEvent::Info(msg));
                    }
                    Err(e) => self.emit(EngineEvent::Error(e)),
                }
            }
            EngineCommand::UdpReplay => self.udp.replay(),
            EngineCommand::SetSwrHaltEnabled(enabled) => {
                self.swr_halt_enabled = enabled;
                let _ = self.db.set_config("swrHaltEnabled", if enabled { "1" } else { "0" });
            }
            EngineCommand::SetSwrHaltThreshold(threshold) => {
                self.swr_halt_threshold = threshold;
                let _ = self.db.set_config("swrHaltThreshold", &threshold.to_string());
            }
            EngineCommand::ClearSwrLockout => {
                self.swr_lockout = false;
                self.publish_tx_state();
            }
            EngineCommand::SetTuneMaxOnSeconds(seconds) => {
                self.tune_max_on_seconds = tx_protection::clamp_tune_max_on_seconds(seconds);
                let _ = self.db.set_config("tuneMaxOnSeconds", &self.tune_max_on_seconds.to_string());
            }
            EngineCommand::SetTuneMethod(method) => {
                self.tune_method = tx_protection::clamp_tune_method(method);
                let _ = self.db.set_config("tuneMethod", &self.tune_method.to_string());
            }
            EngineCommand::SetTuneLevelIndependent(independent) => {
                self.tune_level_independent = independent;
                let _ = self.db.set_config("tuneLevelIndependent", if independent { "1" } else { "0" });
                // Flipping this switches which value is actually driving an
                // already-running Tune -- re-apply right away so the audio
                // doesn't silently keep using whichever one was active before
                // the toggle, until the operator happens to nudge a slider.
                self.apply_live_tune_gain();
            }
            EngineCommand::SetTuneLevel(gain) => {
                self.tune_gain = clamp_tx_gain(gain);
                let _ = self.db.set_config("tuneLevel", &self.tune_gain.to_string());
                self.apply_live_tune_gain();
            }
            EngineCommand::SetTxWatchdog(limit) => {
                self.qso.no_reply_limit = limit;
                let _ = self.db.set_config("noReplyLimit", &limit.to_string());
            }
            EngineCommand::SetMax73Sends(cap) => {
                self.qso.max_73_sends = cap;
                let _ = self.db.set_config("max73Sends", &cap.to_string());
            }
            EngineCommand::SetCqWatchdog(limit) => {
                self.qso.cq_no_reply_limit = limit;
                let _ = self.db.set_config("cqNoReplyLimit", &limit.to_string());
            }
            EngineCommand::SetCqAnswerLimit(limit) => {
                self.qso.cq_answer_limit = limit;
                let _ = self.db.set_config("cqAnswerLimit", &limit.to_string());
            }
            EngineCommand::SetAutoCqAfterQso(enabled) => {
                self.qso.auto_return_to_cq = enabled;
                let _ = self.db.set_config("autoCQAfterQSO", if enabled { "1" } else { "0" });
            }
            EngineCommand::SetClearOnBandModeChange(enabled) => {
                self.clear_on_band_mode_change = enabled;
                let _ = self.db.set_config("clearOnBandModeChange", if enabled { "1" } else { "0" });
            }
            EngineCommand::SetRxLatencyMs(ms) => {
                self.rx_latency_ms = clamp_rx_latency_ms(ms);
                let _ = self.db.set_config("rxLatencyMs", &self.rx_latency_ms.to_string());
            }
            EngineCommand::SetPttDelayMs(ms) => {
                self.ptt_delay_ms = tx_protection::clamp_ptt_delay_ms(ms);
                let _ = self.db.set_config("pttDelay", &self.ptt_delay_ms.to_string());
            }
            EngineCommand::SetLateStartToleranceMs(ms) => {
                self.late_start_tolerance_ms = tx_protection::clamp_late_start_tolerance_ms(ms);
                let _ = self.db.set_config("lateStartTolerance", &self.late_start_tolerance_ms.to_string());
            }
            EngineCommand::SetInputDevice(name) => {
                self.input_device = name.clone();
                let _ = self.db.set_config("input_device", name.as_deref().unwrap_or(""));
                if self.decoding {
                    self.start_decode(); // restart capture on the new device
                }
            }
            EngineCommand::SetOutputDevice(name) => {
                self.output_device = name.clone();
                let _ = self.db.set_config("output_device", name.as_deref().unwrap_or(""));
            }
            EngineCommand::SetRxAudioChannel(channel) => {
                self.rx_audio_channel = crate::audio::channel_select::clamp(channel);
                let _ = self.db.set_config("rxAudioChannel", &self.rx_audio_channel.to_string());
                if self.decoding {
                    self.start_decode(); // restart capture with the new selection
                }
            }
            EngineCommand::SetTxAudioChannel(channel) => {
                let v = crate::audio::channel_select::clamp(channel);
                self.tx_audio_channel_override = Some(v);
                let _ = self.db.set_config("txAudioChannel", &v.to_string());
                self.tx_audio_channel = v; // apply immediately, no reconnect needed
                self.publish_rig_status();
            }
            EngineCommand::SetQmxCatTones(enabled) => {
                self.qmx_cat_tones_override = Some(enabled);
                let _ = self.db.set_config("qmxCatTones", if enabled { "1" } else { "0" });
                self.tx_via_cat_tones = enabled; // apply immediately, no reconnect needed
                self.publish_rig_status();
            }
            EngineCommand::SelectRig(cfg) => {
                if let Ok(json) = serde_json::to_string(&cfg) {
                    let _ = self.db.set_config("rig_config", &json);
                }
                self.select_rig(cfg);
            }
            EngineCommand::DisconnectRig => {
                // Stop any in-flight TX and drop PTT *before* dropping the rig:
                // set_ptt() only sends the CAT un-key when the rig is still
                // connected, so tearing the transport down first could leave the
                // radio keyed. Also stop CQ/QSO so we don't try to re-key a
                // disconnected rig on the next slot.
                self.qso.stop();
                self.tx_parity = None;
                self.finalize_tx();
                self.publish_tx_state();
                // Now drop the connection (runs the rig's close/cleanup); keep the
                // saved config so reconnecting is one click away.
                self.rig = None;
                self.ptt = false;
                self.emit(EngineEvent::Info("rig disconnected".into()));
                self.publish_rig_status();
            }
            EngineCommand::SetTxPeriod(parity) => {
                // The operator explicitly changing their own preference mid-
                // excursion IS "until i change it" happening -- cancel any
                // pending answer-excursion restore outright, rather than
                // later clobbering this deliberate choice back to whatever
                // it was before the excursion began.
                self.preferred_tx_parity_restore = None;
                self.preferred_tx_parity = parity;
                self.publish_tx_state();
            }
            EngineCommand::SetMode(mode) => self.switch_mode(mode),
            EngineCommand::StartCq => {
                self.tx_parity = self.preferred_tx_parity;
                self.has_transmitted_this_run = false;
                self.qso.start_cq();
                // Start CQ this very slot if we're still inside the window;
                // otherwise the first transmission lands at the next free slot.
                self.maybe_transmit();
                self.publish_tx_state();
            }
            EngineCommand::Answer(args) => {
                let msg = DecodedMessage {
                    call_from: args.call_from,
                    grid: args.grid,
                    snr: args.snr,
                    ..Default::default()
                };
                self.answer_message(msg, args.delta_freq, args.msg_utc_ms, false);
            }
            EngineCommand::AnswerQueuedCaller { callsign } => {
                // Caller-queue (pileup-while-busy): the operator tapped a
                // chip for a station that called us while busy with a
                // DIFFERENT target -- see `qso.rs`'s `QueuedCaller` doc
                // comment. Dequeuing (rather than a plain lookup) means a
                // re-tap or a stale chip the UI hasn't refreshed yet can't
                // double-answer the same caller.
                //
                // Uses `qso.answer_caller()`, not plain `answer()` -- live
                // bug, found live: this used to send "HISCALL MYCALL
                // MYGRID" (answer()'s always-Grid-stage reply, correct only
                // for breaking into someone ELSE's CQ bulletin) instead of
                // "HISCALL MYCALL <report>", even though this caller
                // already told us everything needed to reply with a report
                // immediately -- see `answer_caller`'s own doc comment.
                // `extra` must be passed through (not dropped) so
                // `classify()` inside `answer_caller` can tell a grid-
                // shaped message from a report from an RR73, etc.
                //
                // Reusing `answer_caller()`'s "abandon the old target,
                // don't log it" behavior for the OLD QSO -- the SAME thing
                // a manual Answer on a different station already does
                // today (see `qso.rs`'s `manually_answering_someone_else_
                // mid_retry_does_not_relog_the_first_contact` test). This
                // is narrower than Android's `forceLogAndMoveOn`, which
                // can force-log a QSO that reached only a signal report
                // (no RR73 yet) -- porting that needs new record-building
                // logic in `qso.rs` this pass deliberately doesn't add
                // (see this feature's plan doc's "Scope decision"). The
                // one case that matters for the original reported bug --
                // tapping while the old QSO is already logged (Rr73/
                // Bye73) -- already loses nothing, since `notify_transmitted`
                // logged it the moment our own closing message was
                // confirmed sent.
                if let Some(caller) = self.qso.dequeue_specific(&callsign) {
                    let msg = DecodedMessage {
                        call_from: caller.callsign,
                        grid: caller.grid,
                        snr: caller.snr,
                        freq_hz: caller.freq_hz,
                        extra: caller.extra,
                        ..Default::default()
                    };
                    self.answer_message(msg, 0, Some(caller.last_heard_ms), true);
                }
            }
            EngineCommand::SetStage(stage) => {
                self.tx_parity = None;
                self.has_transmitted_this_run = false;
                self.qso.set_stage(stage);
                self.maybe_transmit();
                self.publish_tx_state();
            }
            EngineCommand::StopTx => {
                self.qso.stop();
                self.tx_parity = None;
                // Halt audio + drop PTT now, not at the end of the cycle.
                self.finalize_tx();
                self.publish_tx_state();
            }
            EngineCommand::StartTune => self.start_tune(),
            EngineCommand::StopTune => self.finalize_tx(),
            EngineCommand::FreeText(text) => {
                self.tx_parity = None;
                self.has_transmitted_this_run = false;
                self.qso.set_free_text(&text);
                self.maybe_transmit();
                self.publish_tx_state();
            }
            EngineCommand::RefreshStatus => {
                self.publish_rig_status();
                self.publish_tx_state();
                self.publish_clock_sync();
            }
            EngineCommand::SetClockOffset(offset_ms) => {
                self.clock_offset_ms = offset_ms;
                self.time_synced = true;
                let _ = self.db.set_config("clock_offset_ms", &offset_ms.to_string());
                self.emit(EngineEvent::Info(format!(
                    "time synced: clock offset {offset_ms:+} ms"
                )));
                self.publish_clock_sync();
            }
            EngineCommand::ResyncTime => {
                crate::timesync::spawn_once(self.cmd_tx.clone(), self.db.get_config("ntpServer"));
            }
            EngineCommand::Shutdown => {}
        }
    }

    fn start_decode(&mut self) {
        // Release any existing capture stream FIRST, before opening the next
        // one. `self.input = Some(new)` alone would evaluate `AudioInput::
        // start(...)` (the new open) before dropping the old `self.input` --
        // on Linux, a direct-hardware PCM (no dmix/dsnoop) only accepts one
        // open handle per direction, so switching from e.g. "default" to a
        // named device routed to the *same* hardware (confirmed on a QMX:
        // both "default" and "plughw:CARD=Transceiver,DEV=0" are the one USB
        // audio interface) would otherwise find its own old stream still
        // holding the hardware and fail to open the new one.
        self.input = None;
        match AudioInput::start(self.input_device.as_deref(), self.accum.clone(), self.rx_gain, self.rx_audio_channel) {
            Ok(input) => {
                self.emit(EngineEvent::Info(format!(
                    "capturing from '{}' @ {} Hz",
                    input.device_name, input.device_rate
                )));
                self.input = Some(input);
                self.decoding = true;
            }
            Err(e) => {
                // self.input is already None (dropped above) -- reflect
                // that honestly rather than leaving `decoding` true with
                // no actual capture running.
                self.decoding = false;
                self.emit(EngineEvent::Error(format!("audio start failed: {e}")));
            }
        }
    }

    fn select_rig(&mut self, cfg: RigConfig) {
        // Close any existing connection FIRST. Some CAT servers (notably
        // SmartSDR CAT) accept only one client, so opening the new session while
        // the old one is still open makes the new handshake time out. Dropping
        // here runs the old rig's close/cleanup before we reconnect.
        self.rig = None;
        self.tx_audio_channel = resolve_tx_audio_channel(self.tx_audio_channel_override, cfg.backend, cfg.hamlib_model);
        if should_default_qmx_cat_tones_on(
            crate::rig::needs_cat_tone_tx(cfg.backend, cfg.hamlib_model),
            self.qmx_cat_tones_override.is_some(),
        ) {
            self.qmx_cat_tones_override = Some(true);
            let _ = self.db.set_config("qmxCatTones", "1");
            self.emit(EngineEvent::Info(
                "QMX: turned on CAT tones (first connect, no prior user choice)".into(),
            ));
        }
        self.tx_via_cat_tones = self.qmx_cat_tones_override.unwrap_or(false);
        self.rig_supports_meters = crate::rig::supports_meter_query(cfg.backend, cfg.hamlib_model);
        match RigConnection::connect(&cfg) {
            Ok(mut rig) => {
                let _ = rig.set_frequency(self.dial_hz);
                let name = rig.name();
                self.rig = Some(rig);
                self.emit(EngineEvent::Info(format!("rig connected: {name}")));
                if self.tx_via_cat_tones {
                    // CAT tones replace audio outright, so the left-channel fix
                    // (an audio-path workaround) wouldn't do anything -- only
                    // announce whichever one is actually in effect.
                    self.emit(EngineEvent::Info(
                        "QMX: TX via CAT tone commands (no audio) for full drive".into(),
                    ));
                } else if self.tx_audio_channel != crate::audio::channel_select::BOTH {
                    self.emit(EngineEvent::Info(
                        "QMX: TX audio confined to the left channel for full drive".into(),
                    ));
                }
            }
            Err(e) => {
                self.rig = None;
                self.emit(EngineEvent::Error(format!("rig connect failed: {e}")));
            }
        }
        self.publish_rig_status();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_rx_gain, clamp_tx_gain, effective_tune_gain, format_watts, reply_tx_audio_hz, rms_dbfs,
        should_default_qmx_cat_tones_on, should_refresh_ptt, resolve_tx_audio_channel,
        waterfall_row_due, wf_boundary_row, AnswerArgs, CatToneTx, Engine, EngineCommand, EngineEvent,
        TuneHandle, CLIP_PEAK, DEFAULT_RX_GAIN, DEFAULT_TX_GAIN, HIGH_PEAK_DBFS, LOW_RMS_DBFS,
        MAX_TX_AUDIO_HZ, MIN_TX_AUDIO_HZ, PTT_KEEPALIVE_INTERVAL, SILENCE_DBFS,
    };
    use crate::audio::channel_select;
    use crate::db::Db;
    use crate::qso::{QsoOutcome, TxStage};
    use crate::dsp::{encode, DecodedMessage};
    use crate::qmx_cat_tones as cat_tones;
    use crate::mode::{Mode, DEFAULT_PTT_DELAY_MS};
    use crate::rig::{RigBackend, QRPLABS_QMX_HAMLIB_MODEL};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn tx_audio_channel_auto_detects_left_for_a_qmx_when_unset() {
        assert_eq!(
            resolve_tx_audio_channel(None, RigBackend::Hamlib, QRPLABS_QMX_HAMLIB_MODEL),
            channel_select::LEFT
        );
    }

    #[test]
    fn tx_audio_channel_auto_detects_both_for_a_non_qmx_when_unset() {
        assert_eq!(resolve_tx_audio_channel(None, RigBackend::Flrig, 0), channel_select::BOTH);
        assert_eq!(resolve_tx_audio_channel(None, RigBackend::Hamlib, 1), channel_select::BOTH);
    }

    #[test]
    fn tx_audio_channel_override_wins_regardless_of_rig_type() {
        // Forces RIGHT even for a detected QMX, and forces BOTH even though
        // that's already the non-QMX default (proves the override is
        // actually consulted, not just coincidentally matching auto-detect).
        assert_eq!(
            resolve_tx_audio_channel(Some(channel_select::RIGHT), RigBackend::Hamlib, QRPLABS_QMX_HAMLIB_MODEL),
            channel_select::RIGHT
        );
        assert_eq!(
            resolve_tx_audio_channel(Some(channel_select::BOTH), RigBackend::Flrig, 0),
            channel_select::BOTH
        );
    }

    #[test]
    fn qmx_cat_tones_default_on_the_first_time_a_qmx_connects() {
        assert!(should_default_qmx_cat_tones_on(true, false));
    }

    #[test]
    fn qmx_cat_tones_never_defaults_for_a_non_qmx() {
        assert!(!should_default_qmx_cat_tones_on(false, false));
    }

    #[test]
    fn qmx_cat_tones_never_redefaults_once_a_choice_is_saved() {
        // Covers both an explicit "on" and an explicit "off" -- once the
        // config row exists at all, this never fires again, even for a QMX.
        assert!(!should_default_qmx_cat_tones_on(true, true));
    }

    #[test]
    fn format_watts_drops_a_trailing_zero_tenth() {
        assert_eq!(format_watts(5.0), "5");
        assert_eq!(format_watts(100.0), "100");
    }

    #[test]
    fn format_watts_keeps_a_real_tenth() {
        assert_eq!(format_watts(4.5), "4.5");
        assert_eq!(format_watts(12.4), "12.4");
    }

    #[test]
    fn format_watts_rounds_to_the_nearest_tenth() {
        // The QMX's PC; reply is already in tenths, but float noise (e.g. from
        // an upstream *0.1 conversion) must not leak extra decimal places in.
        assert_eq!(format_watts(4.449), "4.4");
        assert_eq!(format_watts(4.46), "4.5");
    }

    #[test]
    fn ptt_keepalive_only_runs_for_keyed_radios_that_require_it() {
        let due = Some(PTT_KEEPALIVE_INTERVAL);
        assert!(should_refresh_ptt(true, true, due));
        assert!(!should_refresh_ptt(false, true, due));
        assert!(!should_refresh_ptt(true, false, due));
        assert!(!should_refresh_ptt(false, false, due));
    }

    #[test]
    fn ptt_keepalive_waits_for_its_monotonic_interval() {
        // Not yet due: the PTT-on frame went out less than the interval ago.
        assert!(!should_refresh_ptt(true, true, Some(Duration::from_millis(400))));
        // Due once the interval has elapsed, and well inside the G106's ~2 s release.
        assert!(should_refresh_ptt(true, true, Some(Duration::from_millis(1_000))));
        assert!(PTT_KEEPALIVE_INTERVAL < Duration::from_millis(2_000));
        // Never sent this over (e.g. the initial key-up raced a reconnect): send now.
        assert!(should_refresh_ptt(true, true, None));
    }

    #[test]
    fn unkeying_clears_the_keepalive_timestamp() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        engine.set_ptt(true);
        assert!(engine.last_ptt_on_sent.is_some());
        engine.set_ptt(false);
        assert!(engine.last_ptt_on_sent.is_none());
        // Unkeyed: the keepalive is a no-op and must not re-key.
        engine.refresh_ptt_keepalive();
        assert!(!engine.ptt);
        assert!(engine.last_ptt_on_sent.is_none());
    }

    #[test]
    fn preferred_tx_period_seeds_the_lock_on_cq_start() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);
        // Pin the clock mid-cycle, safely past tx_latest_ms: StartCq calls
        // maybe_transmit immediately, and without this the test is flaky --
        // whenever the real wall clock happens to land inside the TX window
        // (~15% of the time for FT8's 2260ms/15000ms), tx_parity locks to
        // the real current slot's parity instead of staying None, failing
        // the assertions below. This test is about tx_parity's *initial*
        // value from the preference, not about whether a real transmission
        // window is open right now.
        let cycle_ms = engine.mode.cycle_ms();
        let into_cycle = crate::util::now_unix_ms().rem_euclid(cycle_ms);
        engine.clock_offset_ms = cycle_ms / 2 - into_cycle;

        // No preference set: CQ starts free to key in any slot's parity (the
        // existing default behavior), locking only once it actually transmits.
        engine.handle(EngineCommand::StartCq);
        assert_eq!(engine.tx_parity, None);
        engine.handle(EngineCommand::StopTx);

        // Operator picks Odd (1) as their preferred TX period before calling CQ:
        // the preference is stored immediately...
        engine.handle(EngineCommand::SetTxPeriod(Some(1)));
        assert_eq!(engine.preferred_tx_parity, Some(1));
        // ...and the next CQ start locks tx_parity to it right away, restricting
        // eligibility to Odd slots from the very first transmission instead of
        // whichever slot happened to be current.
        engine.handle(EngineCommand::StartCq);
        assert_eq!(engine.tx_parity, Some(1));

        // Clearing the preference (back to "no preference") and starting a new
        // CQ frees tx_parity again.
        engine.handle(EngineCommand::StopTx);
        engine.handle(EngineCommand::SetTxPeriod(None));
        engine.handle(EngineCommand::StartCq);
        assert_eq!(engine.tx_parity, None);
    }

    #[test]
    fn answering_a_station_updates_the_tx_period_preference_too() {
        // Real UX requirement, stated live: "if i answer a station that is
        // calling cq the odd/even slot setting is [to] get adjusted to what
        // is needed to reply to that station and it should remain as the
        // selected slot until i change it or choose to answer a station in
        // a different time slot." Answering only ever locked the transient
        // `tx_parity` for *this* run -- `preferred_tx_parity` (what the next
        // CQ start, e.g. via Auto-CQ after this QSO, seeds from) was left
        // alone, so the very next CQ could silently revert to whatever
        // period was set before this QSO, not the one this QSO actually
        // used.
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);
        assert_eq!(engine.preferred_tx_parity, None, "no preference set yet");

        let parity_before = engine.cur_parity();
        engine.handle(EngineCommand::Answer(AnswerArgs {
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            snr: -5,
            delta_freq: 0,
            msg_utc_ms: None,
        }));
        assert_eq!(engine.tx_parity, Some(parity_before));
        assert_eq!(
            engine.preferred_tx_parity,
            Some(parity_before),
            "answering must also update the lasting preference, not just this run's lock"
        );

        // A later CQ (e.g. Auto-CQ after this QSO ends) keeps using that
        // same period, not whatever was set before this QSO.
        engine.qso.stop();
        engine.handle(EngineCommand::StartCq);
        assert_eq!(engine.tx_parity, Some(parity_before));
    }

    #[test]
    fn answering_derives_parity_from_the_targets_own_slot_not_click_time() {
        // Live bug: "my first call to him was in the correct slot but every
        // transmission after that was in the wrong slot" / "he is
        // transmitting in an odd slot but so am i". Root cause: the operator
        // reacting to a decode a little late (decode results land ~13s into
        // a 15s slot) can click Answer one slot after the DX's own CQ --
        // `self.cur_parity()` at that moment is then the SAME parity as the
        // DX, not opposite. `msg_utc_ms` (the decode's own timestamp) lets us
        // derive the reply parity from the DX's actual slot instead.
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        let cycle_ms = engine.mode.cycle_ms();
        let his_slot = engine.cur_slot() + 10; // any slot, far from "now"
        let his_parity = his_slot.rem_euclid(2);
        let his_decode_ms = his_slot * cycle_ms + (cycle_ms - 2000); // decoded late in his slot

        engine.handle(EngineCommand::Answer(AnswerArgs {
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            snr: -5,
            delta_freq: 0,
            msg_utc_ms: Some(his_decode_ms),
        }));

        let expected_reply_parity = 1 - his_parity;
        assert_eq!(engine.tx_parity, Some(expected_reply_parity));
        assert_eq!(engine.preferred_tx_parity, Some(expected_reply_parity));
    }

    #[test]
    fn handle_decoded_queues_a_third_station_calling_while_busy() {
        // The reported live bug: a third station calling while mid-QSO with
        // someone else must be visible, including on the exact slot where
        // process_rx itself would early-return (Bye73) -- see qso.rs's
        // offer_callers doc comment.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        engine.tx_parity = Some(1);
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(
            vec![
                DecodedMessage {
                    call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
                    extra: "-12".into(), snr: -8, ..Default::default()
                },
                DecodedMessage {
                    call_to: "K0XYZ".into(), call_from: "IT9XYZ".into(),
                    grid: "JN54".into(), snr: 7, ..Default::default()
                },
            ],
            0,
        );
        let queue = engine.qso.status().caller_queue;
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].callsign, "IT9XYZ");
    }

    #[test]
    fn answer_queued_caller_switches_target_without_touching_the_old_qsos_log() {
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        engine.tx_parity = Some(1);
        engine.has_transmitted_this_run = true;
        // K1ABC reaches RR73 (still not logged -- that needs
        // notify_transmitted, same as the sibling QSO-completion tests
        // above) while IT9XYZ calls directly.
        engine.handle_decoded(
            vec![
                DecodedMessage {
                    call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
                    extra: "-12".into(), snr: -8, ..Default::default()
                },
                DecodedMessage {
                    call_to: "K0XYZ".into(), call_from: "IT9XYZ".into(),
                    grid: "JN54".into(), snr: 7, ..Default::default()
                },
            ],
            0,
        );
        assert_eq!(engine.qso.status().caller_queue.len(), 1);

        engine.handle(EngineCommand::AnswerQueuedCaller { callsign: "IT9XYZ".into() });

        assert_eq!(engine.qso.status().target.as_deref(), Some("IT9XYZ"));
        // Report stage, not Grid -- IT9XYZ's own captured message already
        // gave us his grid, so the reply is a signal report immediately
        // (answer_caller's whole point), not another grid exchange.
        assert_eq!(engine.qso.status().stage, TxStage::Report);
        assert_eq!(engine.qso.status().tx_message.as_deref(), Some("IT9XYZ K0XYZ +07"));
        assert!(engine.qso.status().caller_queue.is_empty(), "IT9XYZ was dequeued");
        // K1ABC's exchange was abandoned (never reached notify_transmitted),
        // so nothing was logged for it either -- matches plain answer()'s
        // existing behavior for abandoning an unlogged QSO, deliberately
        // not Android's force-log-a-partial-exchange behavior (see this
        // feature's plan doc's "Scope decision").
        assert_eq!(engine.db.list_qsos(10, 0).len(), 0);
        // Same parity bookkeeping as a normal Answer -- reusing the shared
        // answer_message helper, not a parallel implementation.
        assert!(engine.preferred_tx_parity_restore.is_some());
    }

    #[test]
    fn answer_queued_caller_is_a_no_op_for_an_unknown_or_already_dequeued_callsign() {
        let mut engine = test_engine();
        let target_before = engine.qso.status().target.clone();
        engine.handle(EngineCommand::AnswerQueuedCaller { callsign: "GHOST".into() });
        assert_eq!(engine.qso.status().target, target_before, "nothing to dequeue -- must not touch the QSO");
    }

    #[test]
    fn tx_watchdog_give_up_restores_the_pre_answer_tx_period_preference() {
        // Real UX requirement, stated live: "should the slot setting only
        // change for the duration of the QSO or potential QSO and then
        // revert to its original setting" -- confirmed for the exception
        // path specifically ("a normal end of QSO is not where the problem
        // would be it's what happens on the exceptions"): the TX Watchdog
        // forcing an end to a silent target must put the operator's own
        // habitual CQ-calling parity back, not leave it pinned to whatever
        // the abandoned station needed.
        let mut engine = test_engine();
        engine.handle(EngineCommand::SetTxPeriod(Some(0))); // home preference: Even
        engine.qso.no_reply_limit = 2;

        // A deterministic msg_utc_ms (an even his_slot -> reply Odd) --
        // not `None`, which would derive the excursion parity from real
        // wall-clock time at whatever instant this test happens to run,
        // making the test flaky.
        let cycle_ms = engine.mode.cycle_ms();
        let his_slot = (engine.cur_slot() + 10) / 2 * 2; // force even
        engine.handle(EngineCommand::Answer(AnswerArgs {
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            snr: -5,
            delta_freq: 0,
            msg_utc_ms: Some(his_slot * cycle_ms),
        }));
        engine.has_transmitted_this_run = true;
        assert_eq!(engine.tx_parity, Some(1), "reply derived as Odd for this excursion");
        assert_eq!(engine.preferred_tx_parity, Some(1), "excursion value, not yet reverted");

        engine.handle_decoded(vec![], 0); // ours -- must not count
        engine.handle_decoded(vec![], 1); // theirs, silent -- 1st real miss
        assert_eq!(engine.preferred_tx_parity, Some(1), "not given up yet, still mid-excursion");
        engine.handle_decoded(vec![], 2); // ours -- must not count
        engine.handle_decoded(vec![], 3); // theirs, silent -- 2nd real miss -> give up
        assert_eq!(engine.qso.status().stage, TxStage::Cq);
        assert_eq!(
            engine.preferred_tx_parity,
            Some(0),
            "watchdog give-up must restore the pre-answer home preference"
        );

        // Crucially, the ACTIVE lock used for any late reply from the
        // abandoned station is untouched by the restore -- see
        // `preferred_tx_parity_restore`'s own doc comment for why this
        // matters: "what would happen if I've reverted to the message
        // panel because I've given up but get a late reply from him."
        assert_eq!(engine.tx_parity, Some(1), "active lock stays correct for a late reply, unlike the preference");
    }

    #[test]
    fn qso_completion_restores_the_pre_answer_tx_period_preference() {
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.handle(EngineCommand::SetTxPeriod(Some(0))); // home preference: Even

        // Deterministic msg_utc_ms, same reasoning as the watchdog test above.
        let cycle_ms = engine.mode.cycle_ms();
        let his_slot = (engine.cur_slot() + 10) / 2 * 2; // force even -> reply Odd
        engine.handle(EngineCommand::Answer(AnswerArgs {
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            snr: -5,
            delta_freq: 0,
            msg_utc_ms: Some(his_slot * cycle_ms),
        }));
        assert_eq!(engine.tx_parity, Some(1));
        assert_eq!(engine.preferred_tx_parity, Some(1));

        // Drive the QSO to completion via the real engine entry point.
        engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }]); // -> Report
        engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), snr: -8, ..Default::default()
        }]); // -> Bye73
        engine.has_transmitted_this_run = true;
        engine.txed_slot = -999;
        if let Some(QsoOutcome::Completed(record)) = engine.qso.notify_transmitted() {
            engine.restore_preferred_tx_parity_if_owed();
            engine.log_completed_qso(record);
        } else {
            panic!("expected the RR73-send to complete and log the QSO");
        }

        assert_eq!(
            engine.preferred_tx_parity,
            Some(0),
            "QSO completion must restore the pre-answer home preference"
        );
    }

    #[test]
    fn switch_mode_retunes_within_the_same_band() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        // 40m has an FT4 dial (7.0475 MHz) -- switching retunes to it, same band.
        engine.dial_hz = 7_074_000;
        engine.handle(EngineCommand::SetMode(Mode::Ft4));
        assert_eq!(engine.mode, Mode::Ft4);
        assert_eq!(engine.dial_hz, 7_047_500);
        assert_eq!(engine.qso.band, "40m");
        assert_eq!(db.get_config("dial_hz").as_deref(), Some("7047500"));

        // Switching back to FT8 retunes to 40m's FT8 dial.
        engine.handle(EngineCommand::SetMode(Mode::Ft8));
        assert_eq!(engine.dial_hz, 7_074_000);
    }

    #[test]
    fn switch_mode_leaves_the_dial_untouched_with_no_entry_for_the_new_mode() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        // 160m has no FT4 allocation -- the radio-safety rule is "never guess
        // a different band's dial," so the frequency must stay exactly put.
        engine.dial_hz = 1_840_000;
        engine.handle(EngineCommand::SetMode(Mode::Ft4));
        assert_eq!(engine.mode, Mode::Ft4);
        assert_eq!(engine.dial_hz, 1_840_000, "no FT4 dial for 160m -- must not jump bands");
    }

    #[test]
    fn switch_mode_is_rejected_while_actually_transmitting() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        // Simulate a live transmission (a CAT tune key-down -- no real audio
        // device needed to construct this variant) without actually keying
        // any hardware; switch_mode must refuse rather than interrupt it.
        engine.tx_tune = Some(TuneHandle::Cat);
        engine.dial_hz = 7_074_000;
        engine.handle(EngineCommand::SetMode(Mode::Ft4));
        assert_eq!(engine.mode, Mode::Ft8, "must not switch while transmitting");
        assert_eq!(engine.dial_hz, 7_074_000, "must not retune while transmitting");

        // Once the transmission ends, the switch goes through normally.
        engine.tx_tune = None;
        engine.handle(EngineCommand::SetMode(Mode::Ft4));
        assert_eq!(engine.mode, Mode::Ft4);
    }

    #[test]
    fn legacy_rx_offsets_are_cleared_on_restart() {
        for saved in ["-4000", "528"] {
            let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
            db.set_config("rx_offset_ms", saved).expect("seed legacy offset");
            let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
            let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();

            let engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

            assert_eq!(engine.rx_offset_ms, 0, "saved offset {saved} must be ignored");
            assert_eq!(db.get_config("rx_offset_ms").as_deref(), Some("0"));
        }
    }

    #[test]
    fn decoded_dt_cannot_move_future_receive_windows() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);
        let decodes = [0.7, 0.8, 0.9, 1.0]
            .into_iter()
            .map(|time_sec| DecodedMessage { time_sec, ..Default::default() })
            .collect();

        engine.handle_decoded(decodes, 1);

        assert_eq!(engine.rx_offset_ms, 0);
        assert_eq!(db.get_config("rx_offset_ms").as_deref(), Some("0"));
    }

    #[test]
    fn publish_decodes_records_a_real_grid_in_the_callsign_cache() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        engine.publish_decodes(&[DecodedMessage {
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            ..Default::default()
        }], 0);

        assert_eq!(db.qth_for_call("K1ABC"), Some("FN42".to_string()));
    }

    #[test]
    fn publish_decodes_ignores_messages_with_no_grid() {
        // A signal-report/RRR/73 exchange never populates DecodedMessage.grid
        // (dsp/decoder.rs's looks_like_grid gate) -- nothing to record.
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);

        engine.publish_decodes(&[DecodedMessage {
            call_from: "K1ABC".into(),
            grid: "".into(),
            extra: "RRR".into(),
            ..Default::default()
        }], 0);

        assert_eq!(db.qth_for_call("K1ABC"), None);
    }

    #[test]
    fn reply_df_updates_session_offset_without_persisting() {
        // The operator's saved TX offset (base_freq) is what survives a restart.
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        db.set_config("base_freq", "1500").expect("seed base_freq");

        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);
        assert_eq!(engine.tx_audio_hz, 1500, "restored from persisted base_freq");

        // An inbound (untrusted) UDP Reply asks us to answer on 1800 Hz: the
        // in-memory session offset moves so we key up on the requested tone...
        assert!(engine.apply_reply_df(1800));
        assert_eq!(engine.tx_audio_hz, 1800);
        // ...but the operator's *persisted* offset is left exactly as they saved
        // it — a network datagram must not change what comes back after a restart.
        assert_eq!(db.get_config("base_freq").as_deref(), Some("1500"));

        // A 0/unspecified or out-of-band df changes neither the session offset nor
        // the persisted config.
        assert!(!engine.apply_reply_df(0));
        assert!(!engine.apply_reply_df(50_000));
        assert_eq!(engine.tx_audio_hz, 1800);
        assert_eq!(db.get_config("base_freq").as_deref(), Some("1500"));
    }

    #[test]
    fn reply_df_honors_in_band_and_ignores_the_rest() {
        // A plausible tone in the passband is adopted verbatim so we answer on it.
        assert_eq!(reply_tx_audio_hz(1500), Some(1500));
        assert_eq!(reply_tx_audio_hz(MIN_TX_AUDIO_HZ as u32), Some(MIN_TX_AUDIO_HZ));
        assert_eq!(reply_tx_audio_hz(MAX_TX_AUDIO_HZ as u32), Some(MAX_TX_AUDIO_HZ));

        // df == 0 is WSJT-X's "unspecified" — keep the current offset.
        assert_eq!(reply_tx_audio_hz(0), None);
        // Below/above the usable audio passband: a garbled datagram must not push
        // the TX tone off the band, so ignore it and keep the current offset.
        assert_eq!(reply_tx_audio_hz(MIN_TX_AUDIO_HZ as u32 - 1), None);
        assert_eq!(reply_tx_audio_hz(MAX_TX_AUDIO_HZ as u32 + 1), None);
        assert_eq!(reply_tx_audio_hz(50_000), None);
        // A value past i32::MAX can't be a real audio offset either.
        assert_eq!(reply_tx_audio_hz(u32::MAX), None);
    }

    #[test]
    fn rms_dbfs_flags_silence_and_signal() {
        // Digital silence floors well below the silence threshold (not -inf).
        let silence = vec![0.0f32; 6000];
        let db_silent = rms_dbfs(&silence);
        assert!(db_silent.is_finite());
        assert!(db_silent <= SILENCE_DBFS, "silence {db_silent} should be ≤ {SILENCE_DBFS}");

        // Empty buffer is treated as silence, never a panic or -inf.
        assert!(rms_dbfs(&[]) <= SILENCE_DBFS);

        // A half-scale full-amplitude square wave is ~ -6 dBFS — clearly "audio".
        let signal = vec![0.5f32; 6000];
        let db_signal = rms_dbfs(&signal);
        assert!((db_signal - -6.02).abs() < 0.1, "0.5 RMS should be ~-6 dBFS, got {db_signal}");
        assert!(db_signal > SILENCE_DBFS);
    }

    #[test]
    fn peak_and_rms_linear_match_known_buffers() {
        use super::{peak_linear, rms_linear};
        // A buffer with one hot sample: peak catches it, RMS is pulled down
        // by the mostly-quiet rest -- the two must disagree here, which is
        // exactly why classify_input_level checks them separately.
        let mut mixed = vec![0.1f32; 100];
        mixed[50] = 0.9;
        assert!((peak_linear(&mixed) - 0.9).abs() < 1e-6);
        assert!(rms_linear(&mixed) < 0.2, "RMS should stay low despite one hot sample");

        assert_eq!(peak_linear(&[]), 0.0);
        assert_eq!(rms_linear(&[]), 0.0);

        // Sign doesn't matter -- both measure magnitude.
        assert!((peak_linear(&[-0.7, 0.3]) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn classify_input_level_matches_androids_priority_order() {
        use super::{classify_input_level, to_dbfs, InputLevelStatus};
        // Silent: strictly below the silence floor -- matches Android's own
        // strict `<` (AudioInputLevel.classify), so exactly AT the floor
        // already falls through to Low in both apps, not Silent.
        assert_eq!(classify_input_level(0.0, to_dbfs(0.0), SILENCE_DBFS), InputLevelStatus::Low);
        assert_eq!(classify_input_level(0.0, to_dbfs(0.0), SILENCE_DBFS - 1.0), InputLevelStatus::Silent);
        // Low: above silence, below the low/good boundary.
        assert_eq!(classify_input_level(0.05, to_dbfs(0.05), SILENCE_DBFS + 1.0), InputLevelStatus::Low);
        // Good: comfortably in the healthy window.
        assert_eq!(classify_input_level(0.2, to_dbfs(0.2), LOW_RMS_DBFS + 1.0), InputLevelStatus::Good);
        // High: peak dBFS hits the hot threshold even though RMS alone would read Good.
        assert_eq!(
            classify_input_level(0.9, HIGH_PEAK_DBFS, LOW_RMS_DBFS + 1.0),
            InputLevelStatus::High,
        );
        // Clipping takes priority over everything else once the peak pins near full scale,
        // even with a quiet/silent RMS (a single hot transient in an otherwise-dead buffer).
        assert_eq!(classify_input_level(CLIP_PEAK, 0.0, SILENCE_DBFS - 10.0), InputLevelStatus::Clipping);
    }

    #[test]
    fn tx_gain_clamps_to_unit_range() {
        assert_eq!(clamp_tx_gain(0.5), 0.5); // in range, untouched
        assert_eq!(clamp_tx_gain(1.4), 1.0); // never overdrives past full scale
        assert_eq!(clamp_tx_gain(-0.2), 0.0); // never negative (phase flip / garbage)
    }

    #[test]
    fn rx_gain_clamps_to_unit_range() {
        assert_eq!(clamp_rx_gain(0.5), 0.5); // in range, untouched
        assert_eq!(clamp_rx_gain(1.4), 1.0); // attenuate-only, never past unity
        assert_eq!(clamp_rx_gain(-0.2), 0.0); // never negative (phase flip / garbage)
        assert_eq!(clamp_rx_gain(0.0), 0.0); // a deliberate mute is honored
    }

    #[test]
    fn non_finite_gain_falls_back_to_the_default() {
        // f32::clamp propagates NaN, and the clamped value is persisted --
        // "NaN" parses back cleanly on the next launch, so an unguarded NaN
        // would stick permanently (and zero RX, since NaN * sample is NaN).
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(clamp_tx_gain(bad), DEFAULT_TX_GAIN);
            assert_eq!(clamp_rx_gain(bad), DEFAULT_RX_GAIN);
        }
    }

    #[test]
    fn effective_tune_gain_follows_the_independent_toggle() {
        assert_eq!(effective_tune_gain(false, 0.3, 0.9), 0.9); // normal TX gain drives Tune
        assert_eq!(effective_tune_gain(true, 0.3, 0.9), 0.3); // independent override drives Tune
    }

    #[test]
    fn apply_live_tune_gain_is_a_no_op_without_an_audio_tune_running() {
        // Real gap, found live: riding an FT-857D's ALC, walking a manual
        // tuner, or letting an ATU cycle all need the TX level adjustable
        // *while Tune's carrier is already keyed* -- this just confirms the
        // engine-side wiring doesn't panic/misfire when there's nothing to
        // push the gain into yet (idle, or a CAT-tone tune, which has no
        // analog level to adjust at all).
        let mut engine = test_engine();
        engine.apply_live_tune_gain(); // tx_tune is None
        engine.tx_tune = Some(TuneHandle::Cat);
        engine.apply_live_tune_gain(); // CAT tones: no-op, not a crash
    }

    #[test]
    fn wf_boundary_fires_once_per_cycle() {
        let cycle_ms = Mode::Ft8.cycle_ms();
        let mut last = -1;
        // First call always marks (initial slot differs from the -1 sentinel).
        assert!(wf_boundary_row(0, &mut last, cycle_ms));
        // Subsequent rows within the same 15 s slot do not re-mark.
        assert!(!wf_boundary_row(100, &mut last, cycle_ms));
        assert!(!wf_boundary_row(cycle_ms - 1, &mut last, cycle_ms));
        // Crossing into the next slot marks exactly once.
        assert!(wf_boundary_row(cycle_ms, &mut last, cycle_ms));
        assert!(!wf_boundary_row(cycle_ms + 500, &mut last, cycle_ms));
        // And again at the following boundary.
        assert!(wf_boundary_row(2 * cycle_ms + 10, &mut last, cycle_ms));
    }

    #[test]
    fn wf_boundary_fires_on_ft4s_shorter_cycle() {
        let cycle_ms = Mode::Ft4.cycle_ms();
        let mut last = -1;
        assert!(wf_boundary_row(0, &mut last, cycle_ms));
        assert!(!wf_boundary_row(cycle_ms - 1, &mut last, cycle_ms));
        assert!(wf_boundary_row(cycle_ms, &mut last, cycle_ms));
    }

    #[test]
    fn waterfall_row_due_freezes_during_tx() {
        assert!(waterfall_row_due(true, false));
        assert!(!waterfall_row_due(true, true));
        // Not decoding at all: never due, PTT state is irrelevant.
        assert!(!waterfall_row_due(false, false));
        assert!(!waterfall_row_due(false, true));
    }

    #[test]
    fn rx_latency_zero_is_a_no_op() {
        use super::rx_latency_adjusted_into_cycle_ms;
        assert_eq!(rx_latency_adjusted_into_cycle_ms(13_200, 0), 13_200);
        assert_eq!(rx_latency_adjusted_into_cycle_ms(0, 0), 0);
    }

    #[test]
    fn rx_latency_shifts_back_by_the_configured_amount() {
        use super::rx_latency_adjusted_into_cycle_ms;
        // Matches the QMX's own real-world figure from Android's source.
        assert_eq!(rx_latency_adjusted_into_cycle_ms(13_200, 920), 12_280);
        assert_eq!(rx_latency_adjusted_into_cycle_ms(1_000, 1_000), 0);
    }

    #[test]
    fn rx_latency_never_goes_negative() {
        use super::rx_latency_adjusted_into_cycle_ms;
        // Early in the cycle with a large configured latency: clamps to 0
        // rather than asking the decoder to see into the future.
        assert_eq!(rx_latency_adjusted_into_cycle_ms(500, 3000), 0);
    }

    #[test]
    fn clamp_rx_latency_ms_matches_androids_own_bound() {
        use super::clamp_rx_latency_ms;
        assert_eq!(clamp_rx_latency_ms(-100), 0);
        assert_eq!(clamp_rx_latency_ms(0), 0);
        assert_eq!(clamp_rx_latency_ms(920), 920);
        assert_eq!(clamp_rx_latency_ms(3000), 3000);
        assert_eq!(clamp_rx_latency_ms(5000), 3000);
    }

    #[test]
    fn decode_dispatch_due_fires_once_per_slot_past_the_decode_point() {
        use super::decode_dispatch_due;
        let decode_at_ms = Mode::Ft8.decode_at_ms();
        // Fresh slot, past the decode point, not suspended: due.
        assert!(decode_dispatch_due(true, false, 4, -1, decode_at_ms, decode_at_ms));
        // Same slot already dispatched (rx_slot_id == last_decoded_slot): not due again.
        assert!(!decode_dispatch_due(true, false, 4, 4, decode_at_ms, decode_at_ms));
        // Not yet at the decode point this cycle: not due.
        assert!(!decode_dispatch_due(true, false, 4, -1, decode_at_ms - 1, decode_at_ms));
        // Not decoding at all: never due regardless of the rest.
        assert!(!decode_dispatch_due(false, false, 4, -1, decode_at_ms, decode_at_ms));
    }

    #[test]
    fn decode_dispatch_due_is_suppressed_while_suspended_for_import() {
        use super::decode_dispatch_due;
        let decode_at_ms = Mode::Ft8.decode_at_ms();
        // Otherwise-due in every respect, but suspended: must not dispatch.
        assert!(!decode_dispatch_due(true, true, 4, -1, decode_at_ms, decode_at_ms));
        // Un-suspending the identical slot state makes it due again.
        assert!(decode_dispatch_due(true, false, 4, -1, decode_at_ms, decode_at_ms));
    }

    #[test]
    fn set_decode_suspended_does_not_touch_decoding_or_capture() {
        // Confirms this is the narrower pause documented on
        // `EngineCommand::SetDecodeSuspended`: unlike StopDecode, it leaves
        // `decoding` (and therefore the waterfall/RX-meter/input capture it
        // gates) alone -- only `decode_suspended` moves.
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(Arc::clone(&db), evt_tx, cmd_tx);
        engine.decoding = true;

        engine.handle(EngineCommand::SetDecodeSuspended(true));
        assert!(engine.decode_suspended);
        assert!(engine.decoding);

        engine.handle(EngineCommand::SetDecodeSuspended(false));
        assert!(!engine.decode_suspended);
        assert!(engine.decoding);
    }

    #[test]
    fn tx_slot_eligible_enforces_active_parity_once_and_window() {
        use super::tx_slot_eligible;
        let tx_latest_ms = Mode::Ft8.tx_latest_ms(DEFAULT_PTT_DELAY_MS);
        // Inactive QSO never keys.
        assert!(!tx_slot_eligible(false, None, 10, -1, 0, tx_latest_ms));
        // Active, no parity lock, fresh slot, at the very top of the cycle → eligible.
        assert!(tx_slot_eligible(true, None, 10, -1, 0, tx_latest_ms));
        // The last instant that still fits the waveform is eligible; one ms later is not.
        assert!(tx_slot_eligible(true, None, 10, -1, tx_latest_ms, tx_latest_ms));
        assert!(!tx_slot_eligible(true, None, 10, -1, tx_latest_ms + 1, tx_latest_ms));
        // Already transmitted in this slot → no second keying.
        assert!(!tx_slot_eligible(true, None, 10, 10, 0, tx_latest_ms));
        // A locked parity must match the slot's parity (slot 10 is even, 11 is odd).
        assert!(tx_slot_eligible(true, Some(0), 10, -1, 0, tx_latest_ms));
        assert!(!tx_slot_eligible(true, Some(1), 10, -1, 0, tx_latest_ms));
        assert!(tx_slot_eligible(true, Some(1), 11, -1, 0, tx_latest_ms));
    }

    #[test]
    fn tx_slot_eligible_respects_whatever_tx_latest_ms_the_caller_passes() {
        use super::tx_slot_eligible;
        // FT4's TX window (2360ms) is a different, independently-computed value
        // from FT8's (2260ms) -- the gate must use exactly whichever one the
        // caller passes in, not a hard-coded FT8 assumption.
        let ft4_latest = Mode::Ft4.tx_latest_ms(DEFAULT_PTT_DELAY_MS);
        assert_ne!(ft4_latest, Mode::Ft8.tx_latest_ms(DEFAULT_PTT_DELAY_MS));
        assert!(tx_slot_eligible(true, None, 10, -1, ft4_latest, ft4_latest));
        assert!(!tx_slot_eligible(true, None, 10, -1, ft4_latest + 1, ft4_latest));
    }

    #[test]
    fn boundary_tx_keys_at_slot_top_only_after_this_slots_decode() {
        use super::boundary_tx_ready;
        let tx_latest_ms = Mode::Ft8.tx_latest_ms(DEFAULT_PTT_DELAY_MS);
        // Fast decode: this slot's decodes are already processed (awaiting=false), so a
        // reply computed in the previous cycle keys at the top of its slot. This is the
        // case the decode-arrival trigger used to miss — it fires mid-slot, past the
        // window, so the queued reply/CQ was silently dropped. (First arg is `decoding`,
        // true throughout here — the normal case.)
        assert!(boundary_tx_ready(true, true, false, None, 4, -1, 0, tx_latest_ms));
        // Slow decode still pending: must NOT key a (stale) message; handle_decoded
        // keys the fresh one when the decode lands.
        assert!(!boundary_tx_ready(true, true, true, None, 4, -1, 0, tx_latest_ms));
        // Past the window, already transmitted this slot, wrong parity, or inactive all
        // block it too (it delegates to tx_slot_eligible).
        assert!(!boundary_tx_ready(true, true, false, None, 4, -1, tx_latest_ms + 1, tx_latest_ms));
        assert!(!boundary_tx_ready(true, true, false, None, 4, 4, 0, tx_latest_ms));
        assert!(!boundary_tx_ready(true, true, false, Some(1), 4, -1, 0, tx_latest_ms)); // wants odd, slot 4 even
        assert!(!boundary_tx_ready(true, false, false, None, 4, -1, 0, tx_latest_ms));
    }

    #[test]
    fn boundary_tx_is_dormant_while_decoding_is_stopped() {
        use super::boundary_tx_ready;
        let tx_latest_ms = Mode::Ft8.tx_latest_ms(DEFAULT_PTT_DELAY_MS);
        // Regression for the StopDecode runaway-TX bug: after the operator stops
        // decoding, `pending_decodes` is reset to 0 (so the awaiting gate opens) but a
        // CQ/QSO left `active` keeps `qso.active` and `tx_parity` set. The per-tick
        // boundary trigger must stay dormant with the decoder off — otherwise the rig
        // keys every eligible slot with the receiver muted (transmitting unattended).
        // Every other input here is exactly the "would key" case from the test above;
        // only `decoding == false` must hold it back.
        assert!(!boundary_tx_ready(false, true, false, None, 4, -1, 0, tx_latest_ms));
        assert!(!boundary_tx_ready(false, true, false, Some(0), 4, -1, 0, tx_latest_ms));
        // Turning decoding back on restores keying (the fix changes nothing else).
        assert!(boundary_tx_ready(true, true, false, None, 4, -1, 0, tx_latest_ms));
    }

    #[test]
    fn awaiting_gate_stays_closed_until_every_pending_slot_returns() {
        use super::awaiting_any_decode;
        // The gate is driven by the pending-decode count. Walk the "decoder falls
        // behind by a slot" sequence a single in-flight bool would get wrong: it
        // would clear on the first returned batch while a later slot is still out.
        let mut pending = 0usize; // idle: nothing outstanding, gate open.
        assert!(!awaiting_any_decode(pending));

        pending += 1; // slot A enqueued
        assert!(awaiting_any_decode(pending));
        pending += 1; // slot B enqueued before A came back (decoder behind)
        assert!(awaiting_any_decode(pending));

        pending = pending.saturating_sub(1); // A's batch returns...
        assert!(
            awaiting_any_decode(pending),
            "gate must stay closed while slot B is still decoding",
        );
        pending = pending.saturating_sub(1); // B's batch returns
        assert!(!awaiting_any_decode(pending));

        // A stray settle (e.g. a batch arriving after a StopDecode reset to 0)
        // can't underflow the count and reopen/close spuriously.
        pending = pending.saturating_sub(1);
        assert_eq!(pending, 0);
        assert!(!awaiting_any_decode(pending));
    }

    // --- TX Protection / Tune timeout / Clear-on-change -------------------

    fn test_engine() -> Engine {
        let db = Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, _evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        Engine::new(db, evt_tx, cmd_tx)
    }

    #[test]
    fn swr_halt_sets_lockout_and_deactivates_the_qso() {
        let mut engine = test_engine();
        engine.swr_halt_enabled = true;
        engine.swr_halt_threshold = 3.0;
        engine.qso.start_cq();
        assert!(engine.qso.active);

        engine.note_swr_reading(Some(3.75));
        assert!(engine.swr_lockout);
        assert!(!engine.qso.active, "a halt must deactivate the whole auto-sequence");
    }

    #[test]
    fn swr_halt_never_fires_while_tuning() {
        let mut engine = test_engine();
        engine.swr_halt_enabled = true;
        engine.swr_halt_threshold = 3.0;
        engine.tx_tune = Some(TuneHandle::Cat);

        engine.note_swr_reading(Some(9.0));
        assert!(!engine.swr_lockout);
    }

    #[test]
    fn ptt_delay_and_late_start_tolerance_default_to_desktops_prior_behavior() {
        // A fresh engine (no config row yet) must reproduce exactly the old
        // hardcoded PTT_DELAY_MS=100 / "never transmit late" behavior -- a
        // new install, or any install that never opens this new setting,
        // must see zero timing change.
        let engine = test_engine();
        assert_eq!(engine.ptt_delay_ms, 100);
        assert_eq!(engine.late_start_tolerance_ms, 0);
        assert_eq!(engine.effective_tx_latest_ms(), Mode::Ft8.tx_latest_ms(100));
    }

    #[test]
    fn set_ptt_delay_ms_clamps_persists_and_shrinks_the_tx_window() {
        let mut engine = test_engine();
        engine.handle(EngineCommand::SetPttDelayMs(190));
        assert_eq!(engine.ptt_delay_ms, 190);
        assert_eq!(engine.db.get_config("pttDelay").as_deref(), Some("190"));
        // A longer PTT settle eats directly into the eligible TX window.
        assert_eq!(engine.effective_tx_latest_ms(), Mode::Ft8.tx_latest_ms(190));

        // Out-of-range values clamp rather than panic or silently wrap.
        engine.handle(EngineCommand::SetPttDelayMs(-50));
        assert_eq!(engine.ptt_delay_ms, 0);
        engine.handle(EngineCommand::SetPttDelayMs(9000));
        assert_eq!(engine.ptt_delay_ms, 190);
    }

    #[test]
    fn set_late_start_tolerance_ms_clamps_persists_and_widens_the_tx_window() {
        let mut engine = test_engine();
        let before = engine.effective_tx_latest_ms();
        engine.handle(EngineCommand::SetLateStartToleranceMs(2000));
        assert_eq!(engine.late_start_tolerance_ms, 2000);
        assert_eq!(engine.db.get_config("lateStartTolerance").as_deref(), Some("2000"));
        assert_eq!(engine.effective_tx_latest_ms(), before + 2000);

        engine.handle(EngineCommand::SetLateStartToleranceMs(-100));
        assert_eq!(engine.late_start_tolerance_ms, 0);
        engine.handle(EngineCommand::SetLateStartToleranceMs(9000));
        assert_eq!(engine.late_start_tolerance_ms, 4000);
    }

    #[test]
    fn late_start_tolerance_lets_the_already_existing_clip_math_engage() {
        // With tolerance=0 (today's behavior), a start past tx_latest_ms is
        // simply refused -- never clipped. With a real tolerance configured,
        // the SAME already-tested clip math (symbols_to_skip) now correctly
        // computes a non-zero skip for a start that tolerance just let
        // through, proving the two mechanisms compose correctly rather than
        // needing a parallel clip implementation.
        use super::tx_slot_eligible;
        let mut engine = test_engine();
        // Past audio_slack_ms itself (not just past tx_latest_ms) by more
        // than one whole FT8 symbol period (160ms), so a real *whole*
        // symbol must be clipped once this becomes eligible -- tx_latest_ms
        // already sits ptt_delay_ms (100ms) *before* audio_slack_ms, so
        // merely creeping past tx_latest_ms alone wouldn't yet cross into
        // real lateness, and less than one symbol period of lateness would
        // round down to a zero-symbol skip (symbols_to_skip truncates).
        let just_past_audio_slack = engine.mode.audio_slack_ms() + 200;

        // Not eligible at all under today's default (tolerance 0).
        assert!(!tx_slot_eligible(true, None, 0, -1, just_past_audio_slack, engine.effective_tx_latest_ms()));

        engine.handle(EngineCommand::SetLateStartToleranceMs(350));
        // Now eligible -- within the widened window (100ms ptt-delay gap + 350ms tolerance covers the 200ms overage).
        assert!(tx_slot_eligible(true, None, 0, -1, just_past_audio_slack, engine.effective_tx_latest_ms()));

        // The lateness past audio_slack_ms (not past tx_latest_ms) is what
        // the existing, already-tested clip math consumes.
        let ms_late = (just_past_audio_slack - engine.mode.audio_slack_ms()).max(0);
        assert!(ms_late > 0, "a late-but-now-eligible start must produce real lateness to clip");
        let skip = cat_tones::symbols_to_skip(ms_late, engine.mode.symbol_period(), 79);
        assert!(skip > 0, "that lateness must translate into a real leading-symbol skip");
    }

    #[test]
    fn set_station_keeps_the_full_precision_grid_in_config_but_truncates_for_tx() {
        // Real user-reported bug: entering an extended-precision grid (e.g.
        // 8-char "EM71HF21") and saving it kept showing the truncated 4-char
        // value back in Settings, because the persisted "my_grid" config
        // value was being overwritten with the already-truncated locator.
        // Matches Android's GeneralVariables: the stored grid is never
        // truncated, only a dedicated accessor truncates it for the FT8
        // exchange (which needs the protocol's fixed 4-char grid field).
        let mut engine = test_engine();
        engine.handle(EngineCommand::SetStation {
            call: "AI5II".to_string(),
            grid: "em71hf21".to_string(),
        });
        assert_eq!(engine.db.get_config("my_grid"), Some("EM71HF21".to_string()));
        assert_eq!(engine.qso.my_grid, "EM71");
    }

    #[test]
    fn lockout_blocks_new_transmission_until_explicitly_cleared() {
        let mut engine = test_engine();
        engine.swr_lockout = true;
        engine.qso.start_cq();
        engine.qso.active = true; // re-armed after the halt, as StartCq would do
        let before = engine.qso.tx_message().map(|s| s.to_string());

        engine.maybe_transmit();
        // Blocked: no transmission attempted, state untouched.
        assert_eq!(engine.qso.tx_message().map(|s| s.to_string()), before);

        engine.handle(EngineCommand::ClearSwrLockout);
        assert!(!engine.swr_lockout);
    }

    #[test]
    fn start_tune_refuses_while_locked_out() {
        let mut engine = test_engine();
        engine.swr_lockout = true;
        engine.start_tune();
        assert!(engine.tx_tune.is_none());
    }

    #[test]
    fn start_tune_refuses_internal_method_with_no_atu() {
        let mut engine = test_engine();
        engine.tune_method = crate::tx_protection::TUNE_METHOD_INTERNAL;
        engine.start_tune();
        assert!(engine.tx_tune.is_none(), "must not key a surprise carrier");
    }

    #[test]
    fn tune_timeout_force_stops_a_forgotten_tune() {
        let mut engine = test_engine();
        engine.tune_max_on_seconds = 10;
        engine.tx_tune = Some(TuneHandle::Cat);
        engine.tune_started_at = Some(std::time::Instant::now() - Duration::from_secs(11));

        // Mirrors the run loop's own check (see the main `run` method).
        let expired = engine
            .tune_started_at
            .map(|s| crate::tx_protection::tune_expired(s.elapsed().as_millis() as i64, engine.tune_max_on_seconds))
            .unwrap_or(false);
        assert!(expired);
        if expired {
            engine.finalize_tx();
        }
        assert!(engine.tx_tune.is_none());
        assert!(engine.tune_started_at.is_none());
    }

    #[test]
    fn clear_on_band_change_rearms_cq_only_if_something_was_running() {
        let mut engine = test_engine();
        engine.clear_on_band_mode_change = true;
        engine.qso.start_cq();
        engine.handle(EngineCommand::SetBand(14_074_000));
        assert_eq!(engine.qso.status().stage, crate::qso::TxStage::Cq);
        assert!(engine.qso.active);
    }

    #[test]
    fn clear_on_band_change_stays_idle_when_nothing_was_running() {
        let mut engine = test_engine();
        engine.clear_on_band_mode_change = true;
        assert!(!engine.qso.active);
        engine.handle(EngineCommand::SetBand(14_074_000));
        assert!(!engine.qso.active, "must not start transmitting out of nowhere");
    }

    #[test]
    fn clear_on_band_change_disabled_leaves_qso_untouched() {
        let mut engine = test_engine();
        engine.clear_on_band_mode_change = false;
        engine.qso.answer(&DecodedMessage { call_from: "K1ABC".into(), ..Default::default() });
        let target_before = engine.qso.status().target.clone();
        engine.handle(EngineCommand::SetBand(14_074_000));
        assert_eq!(engine.qso.status().target, target_before, "setting is off -- must not touch the QSO");
    }

    #[test]
    fn clear_decode_list_event_serializes_with_a_data_field() {
        // EngineEvent uses #[serde(tag = "type", content = "data")] -- but a
        // *unit* variant (no payload, unlike every other EngineEvent member)
        // serializes with no "data" key at all, not "data":null. Confirmed
        // here so the TS side's EngineEvent type matches precisely.
        let json = serde_json::to_string(&EngineEvent::ClearDecodeList).unwrap();
        assert_eq!(json, r#"{"type":"clear_decode_list"}"#);
    }

    #[test]
    fn tx_watchdog_and_max_73_sends_commands_persist_into_qso() {
        let mut engine = test_engine();
        engine.handle(EngineCommand::SetTxWatchdog(5));
        engine.handle(EngineCommand::SetMax73Sends(2));
        assert_eq!(engine.qso.no_reply_limit, 5);
        assert_eq!(engine.qso.max_73_sends, 2);
    }

    #[test]
    fn cq_watchdog_command_persists_into_qso_and_config_independent_of_tx_watchdog() {
        let mut engine = test_engine();
        engine.handle(EngineCommand::SetTxWatchdog(5));
        engine.handle(EngineCommand::SetCqWatchdog(10));
        assert_eq!(engine.qso.no_reply_limit, 5);
        assert_eq!(engine.qso.cq_no_reply_limit, 10);
        assert_eq!(engine.db.get_config("cqNoReplyLimit").as_deref(), Some("10"));
    }

    #[test]
    fn own_tx_parity_slot_is_not_counted_as_a_tx_watchdog_miss() {
        // Real bug, found live: every decoded slot -- including our OWN
        // transmit slot, where the partner structurally can't be replying
        // (FT8 alternates by parity) -- was passed to qso.process_rx, so a
        // full round trip (our slot + theirs) counted as TWO misses
        // instead of one. "Stop After 3" gave up after only ~1-2 real
        // retries, not 3.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(),
            call_from: "N2JFD".into(),
            grid: "FM09".into(),
            snr: -10,
            ..Default::default()
        });
        assert_eq!(engine.qso.status().stage, TxStage::Grid);
        engine.qso.no_reply_limit = 3;
        engine.has_transmitted_this_run = true;
        engine.tx_parity = Some(0); // we transmit on even slots, they on odd

        engine.handle_decoded(vec![], 0); // ours -- must not count
        engine.handle_decoded(vec![], 1); // theirs, silent -- 1st real miss
        engine.handle_decoded(vec![], 2); // ours -- must not count
        engine.handle_decoded(vec![], 3); // theirs, silent -- 2nd real miss
        assert_eq!(
            engine.qso.status().stage,
            TxStage::Grid,
            "must still be retrying after only 2 real misses, not given up early"
        );
        engine.handle_decoded(vec![], 4); // ours -- must not count
        engine.handle_decoded(vec![], 5); // theirs, silent -- 3rd real miss -> give up
        assert_eq!(
            engine.qso.status().stage,
            TxStage::Cq,
            "gives up at exactly 3 real retries, matching the configured limit"
        );
    }

    #[test]
    fn own_tx_parity_slot_skip_does_not_delay_a_bye73_completion() {
        // Checked carefully after a user report ("panel did not return to
        // decoding" after a logged QSO with Auto-CQ off): does skipping
        // process_rx on our own transmit-parity slot ever delay or drop the
        // Bye73 -> finish_qso() completion check? It must not -- bye73_sent
        // is set by notify_transmitted (called from maybe_transmit, entirely
        // independent of the process_rx skip), and the completion check
        // itself always lands on the PARTNER's parity (the slot after ours),
        // which this fix never skips.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.auto_return_to_cq = false;
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(),
            call_from: "K1ABC".into(),
            grid: "FN42".into(),
            snr: -5,
            ..Default::default()
        });
        // DX reports us, then sends RR73 -- advances straight to Bye73.
        engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }]);
        let outcome = engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }]);
        // Queued, not yet logged -- logging now waits for notify_transmitted.
        assert!(outcome.is_none());
        assert_eq!(engine.qso.status().stage, TxStage::Bye73);
        assert!(engine.qso.active, "still armed to send the courtesy 73");

        // The scheduler actually transmits the 73 (our parity slot) -- this logs it.
        assert!(matches!(engine.qso.notify_transmitted(), Some(QsoOutcome::Completed(_))));
        engine.tx_parity = Some(0);
        engine.has_transmitted_this_run = true;

        // Our own next slot -- process_rx must be skipped, but that must not
        // matter: nothing to complete here, the real check lands on theirs.
        engine.handle_decoded(vec![], 2);
        assert!(engine.qso.active, "must not have finished yet -- wrong (our) parity");

        // Their slot: the real completion check fires here, undelayed.
        engine.handle_decoded(vec![], 3);
        assert!(!engine.qso.active, "must finish (Auto-CQ off) on the very next non-skipped slot");
        assert_eq!(engine.qso.status().stage, TxStage::Idle);

        // And it must STAY off -- further decodes (either parity) must not
        // reactivate anything.
        engine.handle_decoded(vec![], 4);
        engine.handle_decoded(vec![], 5);
        assert!(!engine.qso.active, "must stay off, not get re-armed by later slots");
    }

    #[test]
    fn finalize_tx_ends_the_qso_the_instant_the_73_finishes_transmitting() {
        // Real UX bug, found live: "after logging a qso the panel did not
        // return to decoding... after i sent my 73." The QSO was only ever
        // wrapped up by the NEXT decoded slot noticing bye73_sent (up to
        // ~30s, 2-3 missed decode cycles) -- finalize_tx (the one place
        // audio AND CAT-tone TX both already detect "the transmission just
        // finished") must do it immediately instead, with no decoded slot
        // involved at all.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.auto_return_to_cq = false;
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "FN42".into(), snr: -5, ..Default::default()
        });
        engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }]);
        let outcome = engine.qso.process_rx(&[DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }]);
        // Not logged yet -- only queued. Logging now waits for
        // notify_transmitted to confirm the courtesy 73 was actually sent
        // (see that method's own doc comment).
        assert!(outcome.is_none());
        assert_eq!(engine.qso.status().stage, TxStage::Bye73);

        // The scheduler keys the 73 (notify_transmitted, same as the real
        // start_transmit -> notify_transmitted call in maybe_transmit) and
        // the CAT-tone transmission is now in flight. THIS logs the QSO.
        assert!(matches!(engine.qso.notify_transmitted(), Some(QsoOutcome::Completed(_))));
        engine.tx_cat = Some(CatToneTx {
            tones: [0; encode::FT4_NN],
            len: 0,
            symbol_period: 0.16,
            base_hz: 1500.0,
            start: Instant::now(),
            sent_through: 0,
            skip_until: 0,
        });
        assert!(engine.qso.active, "still finishing the over, not done yet");

        // The transmission finishes -- no decoded slot anywhere in sight.
        engine.finalize_tx();

        assert!(!engine.qso.active, "must end the instant the 73 finishes transmitting");
        assert_eq!(engine.qso.status().stage, TxStage::Idle);
    }

    #[test]
    fn qso_comment_includes_distance_when_both_grids_are_known() {
        // Real bug, found live: "check logic in android to see what
        // populates the comments when the qso is logged because field is
        // empty in the qso i just had." qso.rs's complete() always leaves
        // comment blank by design (no DB/config access there); Android's
        // QSLRecord constructor instead auto-fills "Distance: <N> <mi|km>,
        // QSO by FT8AF" -- ported into engine.rs alongside the existing
        // tx_pwr fill, which already lives in exactly this spot for the
        // same reason.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.my_grid = "FN42".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        // Matches the real Answer command handler, which locks tx_parity
        // immediately -- both decode slots below are the partner's incoming
        // messages (parity 0), so tx_parity must be the *other* parity or
        // the new pre-first-transmission skip (see is_own_tx_slot's doc
        // comment) would wrongly treat them as our own slot too.
        engine.tx_parity = Some(1);
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }], 0);
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }], 2);
        // RR73 only queues the courtesy 73 now -- logging (and this test's
        // own fill-in logic) waits for the scheduler to confirm it was
        // actually sent, same as the real maybe_transmit success path.
        if let Some(QsoOutcome::Completed(record)) = engine.qso.notify_transmitted() {
            engine.log_completed_qso(record);
        }

        let qsos = engine.db.list_qsos(10, 0);
        assert_eq!(qsos.len(), 1);
        let comment = &qsos[0].comment;
        assert!(comment.starts_with("Distance: "), "got: {comment}");
        assert!(comment.ends_with(", QSO by FT8AF"), "got: {comment}");
        assert!(comment.contains(" mi,"), "distanceInMiles defaults on, got: {comment}");
    }

    #[test]
    fn qso_comment_respects_distance_in_miles_config() {
        let mut engine = test_engine();
        let _ = engine.db.set_config("distanceInMiles", "0");
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.my_grid = "FN42".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        engine.tx_parity = Some(1); // see the sibling test's own comment
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }], 0);
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }], 2);
        if let Some(QsoOutcome::Completed(record)) = engine.qso.notify_transmitted() {
            engine.log_completed_qso(record);
        }

        let comment = engine.db.list_qsos(10, 0)[0].comment.clone();
        assert!(comment.contains(" km,"), "distanceInMiles=0 must use km, got: {comment}");
    }

    #[test]
    fn qso_comment_is_plain_when_a_grid_is_unknown() {
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.my_grid = String::new(); // operator never set a grid
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        engine.tx_parity = Some(1); // see qso_comment_includes_distance...'s own comment
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }], 0);
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }], 2);
        if let Some(QsoOutcome::Completed(record)) = engine.qso.notify_transmitted() {
            engine.log_completed_qso(record);
        }

        assert_eq!(engine.db.list_qsos(10, 0)[0].comment, "QSO by FT8AF");
    }

    #[test]
    fn logged_qso_record_carries_its_real_row_id() {
        // Real bug, found live (user asked to audit the logbook sync flags
        // specifically): `insert_qso`'s returned row id was discarded
        // (`Ok(_)`), so `record.id` stayed `None` through the rest of
        // log_completed_qso -- including the clone handed to the
        // auto-upload-on-log QRZ thread. `upload_and_mark_one`'s success
        // path is `if let Some(id) = q.id { mark_synced(id, "qrz") }`, so
        // every auto-uploaded QSO actually reached QRZ (it uploads by
        // content, not by id) but could never mark the local row
        // `synced_qrz = 1` -- silently skipped every single time. Confirmed
        // here via the emitted `QsoCompleted` event (what the QRZ-upload
        // clone is taken from) rather than hitting the real network.
        let db = std::sync::Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(db, evt_tx, cmd_tx);
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.my_grid = "FN42".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "EM12".into(), snr: -5, ..Default::default()
        });
        engine.tx_parity = Some(1);
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "-12".into(), snr: -8, ..Default::default()
        }], 0);
        engine.handle_decoded(vec![DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            extra: "RR73".into(), grid: "RR73".into(), snr: -8, ..Default::default()
        }], 2);
        if let Some(QsoOutcome::Completed(record)) = engine.qso.notify_transmitted() {
            engine.log_completed_qso(record);
        }

        let completed = (0..)
            .map_while(|_| evt_rx.try_recv().ok())
            .find_map(|e| match e {
                EngineEvent::QsoCompleted(r) => Some(r),
                _ => None,
            })
            .expect("a QsoCompleted event must have been emitted");
        assert!(completed.id.is_some(), "the emitted record must carry its real row id, not None");
        assert_eq!(completed.id, engine.db.list_qsos(10, 0)[0].id);
    }

    #[test]
    fn pending_decode_slots_preserves_dispatch_order_across_a_backlog() {
        // Real bug, found live: "cq wathdog set to 5 but called cq 6 times
        // and then i cancelled it" (dead silence the whole run -- ruling
        // out a real reply resetting the counter). Root cause: the drain
        // loop used to label every result with the CURRENT tick's own
        // `slot_id` (freshly recomputed from `now`), not the slot its audio
        // was actually captured for -- the decode worker runs on its own
        // thread and is only polled opportunistically, so a result can
        // surface a tick or more after the slot that dispatched it. If
        // that crosses a slot boundary, the label is one slot too new,
        // flipping its parity and making the own-tx-slot skip silently
        // treat a genuine partner-silence miss as "our own slot" -- the
        // watchdog counter then undercounts and gives up late or never.
        // This FIFO is what fixes it: dispatch order must survive even
        // with several slots outstanding at once (decoder badly behind),
        // not just the single-outstanding-decode common case.
        let mut engine = test_engine();
        engine.pending_decode_slots.push_back(10);
        engine.pending_decode_slots.push_back(11);
        engine.pending_decode_slots.push_back(12);
        assert_eq!(engine.pending_decode_slots.pop_front(), Some(10));
        assert_eq!(engine.pending_decode_slots.pop_front(), Some(11));
        assert_eq!(engine.pending_decode_slots.pop_front(), Some(12));
        assert_eq!(engine.pending_decode_slots.pop_front(), None);
    }

    #[test]
    fn a_late_published_decode_still_carries_its_true_slots_timestamp() {
        // Real bug, confirmed live with two screenshots: "the message box
        // shows him calling CQ at the :00 time slot but he actually called
        // in the previous :45 slot when looking at the waterfall." The
        // `pending_decode_slots` fix above already corrects `handle_decoded`'s
        // OWN `slot_id` parameter for the engine's internal watchdog/parity
        // accounting, but `publish_decodes` -- what builds the `UiMessage`
        // the UI (and `AnswerArgs::msg_utc_ms`) actually sees -- still
        // stamped every decode with `self.now()` at publish time instead of
        // that corrected slot, so a late-published decode crossing a slot
        // boundary showed the WRONG (one-slot-too-new) time regardless.
        let db = std::sync::Arc::new(Db::open_in_memory().expect("in-memory db"));
        let (evt_tx, evt_rx) = std::sync::mpsc::channel();
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(db, evt_tx, cmd_tx);

        let cycle_ms = engine.mode.cycle_ms();
        let true_slot_id = engine.cur_slot() + 10; // some slot, far from "now"
        // Simulate publishing arriving late -- well after the slot that
        // actually captured this decode's audio.
        engine.clock_offset_ms += true_slot_id * cycle_ms + cycle_ms * 3 - engine.now();

        engine.handle_decoded(
            vec![DecodedMessage { call_from: "K1ABC".into(), grid: "FN42".into(), ..Default::default() }],
            true_slot_id,
        );

        let ui = (0..)
            .map_while(|_| evt_rx.try_recv().ok())
            .find_map(|e| match e {
                EngineEvent::Decoded(msgs) => Some(msgs),
                _ => None,
            })
            .expect("a Decoded event must have been emitted");
        assert_eq!(
            ui[0].utc_ms,
            true_slot_id * cycle_ms,
            "must carry the slot it was actually captured for, not whenever publish happened to run"
        );
    }

    #[test]
    fn maybe_transmit_never_keys_on_a_stale_slot_id_from_a_backlogged_decode() {
        // Real bug, confirmed live with two screenshots and a full
        // diagnostic trace: the QMX CAT-tone TX path blocks the engine
        // thread for the full ~12.64s of a transmission (one rig_send_raw
        // per symbol). A queued command or a backlogged decode result that
        // finally gets processed after that can be labeled with a slot_id
        // that's a full slot (or more) behind real time -- but
        // `tx_slot_eligible`'s `into_cycle_ms` check is a bare modulo, so
        // it can't tell "106ms into the stale slot" from "106ms into the
        // slot after it." `handle_decoded`'s tail call used to pass that
        // stale slot_id straight through to `maybe_transmit`, which could
        // then key up believing it was still replying opposite the DX,
        // while the radio actually transmitted during the REAL current
        // slot -- sometimes the DX's own parity, sometimes not, matching
        // the live report "sometimes everything looks correct other times
        // it does not." Fixed: `maybe_transmit` always derives its own
        // slot fresh from `self.cur_slot()`, never trusting a caller's
        // value.
        let mut engine = test_engine();
        engine.qso.my_call = "K0XYZ".into();
        engine.qso.answer(&DecodedMessage {
            call_to: "K0XYZ".into(), call_from: "K1ABC".into(),
            grid: "FN42".into(), snr: -5, ..Default::default()
        });
        let true_slot = engine.cur_slot();
        // Lock tx_parity to the OPPOSITE of the true current slot's parity
        // -- by real time, right now is NOT our turn to transmit.
        engine.tx_parity = Some(1 - true_slot.rem_euclid(2));
        engine.has_transmitted_this_run = true;
        engine.txed_slot = -999;

        // Simulate a backlogged decode result labeled with a STALE slot_id
        // that happens to match tx_parity's parity -- the exact shape that
        // used to fool the old, caller-trusting eligibility check.
        let stale_slot_id = true_slot - 1;
        assert_eq!(stale_slot_id.rem_euclid(2), engine.tx_parity.unwrap(), "the stale label must match, to exercise the bug");
        engine.handle_decoded(vec![], stale_slot_id);

        assert_ne!(
            engine.txed_slot, stale_slot_id,
            "must never key up for the stale slot_id -- only ever the true current slot"
        );
    }

    #[test]
    fn own_tx_parity_slot_is_not_counted_as_a_cq_watchdog_miss() {
        // Same bug, same fix, for the idle-CQ counter.
        let mut engine = test_engine();
        engine.qso.start_cq();
        engine.qso.cq_no_reply_limit = 2;
        engine.tx_parity = Some(0);
        engine.has_transmitted_this_run = true;

        engine.handle_decoded(vec![], 0); // ours -- must not count
        engine.handle_decoded(vec![], 1); // theirs, silent -- 1st real miss
        assert!(engine.qso.active, "must still be calling CQ after only 1 real miss");
        engine.handle_decoded(vec![], 2); // ours -- must not count
        engine.handle_decoded(vec![], 3); // theirs, silent -- 2nd real miss -> give up
        assert!(!engine.qso.active, "gives up at exactly 2 real misses, matching the configured limit");
    }

    #[test]
    fn a_decode_cycle_before_the_first_real_transmission_is_not_counted_as_a_cq_watchdog_miss() {
        // Real bug, found live: "cq watchdog set to 3, only transmitted 2
        // cq's." Confirmed from a full trace -- the watchdog's own count DID
        // reach exactly 3/3, but only 2 real CQ overs had gone out. Root
        // cause: StartCq tries to key the very slot it's issued in, which
        // misses the TX window far more often than not (an operator's click
        // lands at an arbitrary point in the cycle, not at its top) --
        // tx_parity then stays `None` until the NEXT slot's boundary trigger
        // gets the first real over out. The OLD is_own_tx_slot check only
        // matched `Some(parity)`, so that in-between slot (decoded, but
        // nothing could possibly be heard yet -- nothing has been
        // transmitted at all) fell through to a real `process_rx` call and
        // counted as a genuine "no reply" miss before CQ #1 ever existed,
        // making every run give up one real transmission earlier than its
        // configured limit.
        let mut engine = test_engine();
        engine.qso.start_cq();
        engine.qso.cq_no_reply_limit = 3;
        // tx_parity starts None (StartCq's own immediate attempt missed the
        // window) -- this slot must be skipped exactly like an own-parity
        // slot, not counted as a real miss.
        assert_eq!(engine.tx_parity, None);
        engine.handle_decoded(vec![], 0);
        assert_eq!(engine.qso.status().stage, TxStage::Cq);
        assert!(engine.qso.active, "the pre-first-transmission slot must not itself cost a miss");

        // The first real over now lands (boundary trigger, next slot) --
        // tx_parity locks, has_transmitted_this_run flips, and real counting
        // starts from here.
        engine.tx_parity = Some(1);
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![], 1); // ours -- must not count
        engine.handle_decoded(vec![], 2); // theirs, silent -- 1st real miss
        assert!(engine.qso.active, "only 1 real miss so far");
        engine.handle_decoded(vec![], 3); // ours -- must not count
        engine.handle_decoded(vec![], 4); // theirs, silent -- 2nd real miss
        assert!(engine.qso.active, "only 2 real misses so far -- this is the slot the old bug gave up on");
        engine.handle_decoded(vec![], 5); // ours -- must not count
        engine.handle_decoded(vec![], 6); // theirs, silent -- 3rd real miss -> give up
        assert!(!engine.qso.active, "gives up at exactly 3 real misses, matching the configured limit");
    }

    #[test]
    fn a_fixed_tx_period_still_skips_the_pre_transmission_slot() {
        // The sibling test above covers "Auto" TX Period, where tx_parity
        // starts `None`. Real bug, found live (the operator, after the
        // Auto-case fix above was already shipped: "the counter should
        // increment after I call the cq not before the actual
        // transmission" -- still reproducing): with a *fixed* TX Period
        // preference (Settings/TxBar's Even/Odd, not Auto),
        // `EngineCommand::StartCq` seeds `tx_parity` from
        // `preferred_tx_parity` immediately -- `tx_parity` is `Some(_)` from
        // the very first instant, never `None`, so the old
        // `tx_parity.is_none()` check could never have caught this case even
        // though the exact same "click missed the window" scenario applies
        // just as much. `has_transmitted_this_run` fixes this by tracking
        // the real signal directly, independent of whether parity happened
        // to be locked early.
        // Mirrors EngineCommand::StartCq's own two lines exactly (not routed
        // through `.handle()`, which would also call the real,
        // wall-clock-dependent `maybe_transmit` -- irrelevant here and a
        // source of test flakiness, since this test only cares about
        // is_own_tx_slot's skip logic, not whether a real device exists).
        let mut engine = test_engine();
        engine.preferred_tx_parity = Some(0);
        engine.tx_parity = engine.preferred_tx_parity;
        engine.has_transmitted_this_run = false;
        engine.qso.start_cq();
        engine.qso.cq_no_reply_limit = 2;
        assert_eq!(engine.tx_parity, Some(0), "fixed preference locks tx_parity immediately, unlike Auto");

        // A decode cycle lands on slot 0 (matches the fixed preference's own
        // parity) before any real over has gone out -- must not count,
        // exactly like the pre-transmission slot in the Auto case.
        engine.handle_decoded(vec![], 0);
        assert!(engine.qso.active, "the pre-first-transmission slot must not itself cost a miss, even with a fixed TX Period");

        // The first real over now lands.
        engine.has_transmitted_this_run = true;
        engine.handle_decoded(vec![], 0); // ours -- must not count
        engine.handle_decoded(vec![], 1); // theirs, silent -- 1st real miss
        assert!(engine.qso.active, "only 1 real miss so far");
        engine.handle_decoded(vec![], 2); // ours -- must not count
        engine.handle_decoded(vec![], 3); // theirs, silent -- 2nd real miss -> give up
        assert!(!engine.qso.active, "gives up at exactly 2 real misses, matching the configured limit");
    }
}
