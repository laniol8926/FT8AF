//! The runtime engine: a dedicated thread that owns audio + decoder + QSO state
//! machine + rig, runs the 15 s UTC cycle (ports `UtcTimer`), and turns QSO
//! decisions into encode + PTT + playback + DB writes. Communicates with the
//! Tauri layer over command/event channels so the cpal `Stream`s (which are
//! `!Send` on Windows) never leave this thread.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::audio::{output, AudioInput, SlotAccumulator};
use crate::bands;
use crate::db::{Db, QsoRecord};
use crate::dsp::encode;
use crate::dsp::{DecodedMessage, Decoder, SAMPLE_RATE};
use crate::mode::Mode;
use crate::qmx_cat_tones as cat_tones;
use crate::qmx_meters;
use crate::qso::{QsoEngine, QsoOutcome, QsoStatus};
use crate::rig::{RigConfig, RigConnection};
use crate::util;
use crate::wf::{WfConfig, WfProcessor, WfWindow};

// Cycle/TX-timing constants (15s/12.64s-audio FT8, 7.5s/5.04s-audio FT4) now
// live on `Mode` (cycle_ms/audio_slack_ms/tx_latest_ms/decode_at_ms) instead
// of as fixed consts here -- see mode.rs. PTT_DELAY_MS stays a plain const:
// it's rig-settle time, not FT8/FT4-specific.
const PTT_DELAY_MS: u64 = 100;
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
}

#[derive(Debug, Clone)]
pub enum EngineCommand {
    StartDecode,
    StopDecode,
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
    SelectRig(RigConfig),
    /// Drop the live rig connection for this session (CAT/PTT released). Leaves
    /// the saved rig config intact so Connect — or the next launch — reconnects.
    DisconnectRig,
    StartCq,
    Answer(AnswerArgs),
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
    /// Apply new WSJT-X UDP settings (enable/host/port/accept-requests). Rebinds
    /// the socket + listener live so the Settings screen takes effect at once.
    SetUdpConfig(crate::udp::UdpConfig),
    /// Re-broadcast this session's decodes over UDP (inbound WSJT-X Replay).
    UdpReplay,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct RigStatusEvent {
    pub connected: bool,
    pub model: String,
    pub frequency_hz: u64,
    pub ptt: bool,
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
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct InputLevelEvent {
    /// RMS level of the recent capture in dBFS (≈ -100 for digital silence).
    pub db: f32,
    /// True when the level is at/below the silence floor while decoding — the
    /// "no audio reaching the app" condition.
    pub silent: bool,
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
    crate::timesync::spawn_loop(cmd_tx.clone());
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
    /// Looping steady-tone cpal stream; dropping it halts the carrier. Never
    /// read -- held purely for this `Drop` side effect, hence `allow(dead_code)`.
    Audio(#[allow(dead_code)] output::TunePlayback),
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
    rig: Option<RigConnection>,
    input: Option<AudioInput>,
    input_device: Option<String>,
    output_device: Option<String>,
    decoding: bool,
    dial_hz: u64,
    tx_audio_hz: i32,
    /// TX output level (0.0–1.0) applied to the waveform before playback.
    tx_gain: f32,
    /// Whether the connected rig needs TX audio confined to the left channel
    /// to fully drive its modulator (currently just the QRP Labs QMX -- see
    /// [`crate::rig::needs_left_channel_tx_audio`]). Recomputed on every
    /// `select_rig`.
    tx_left_channel_only: bool,
    /// Whether TX should go out as CAT tone commands instead of audio at all
    /// (currently just the QRP Labs QMX -- see
    /// [`crate::rig::needs_cat_tone_tx`]). Recomputed on every `select_rig`.
    tx_via_cat_tones: bool,
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
/// come back over the returned receiver. `is_ft8` selects the decode
/// protocol (`Mode::is_ft8()`) -- called once from `Engine::new` and again
/// from `switch_mode` whenever the operating mode changes, since a `Decoder`
/// is built for one protocol for its whole lifetime.
fn spawn_decoder_thread(is_ft8: bool) -> (Sender<Vec<f32>>, Receiver<Vec<DecodedMessage>>) {
    let (slot_tx, slot_rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let (dec_tx, dec_rx) = std::sync::mpsc::channel::<Vec<DecodedMessage>>();
    std::thread::Builder::new()
        .name("ft8af-decoder".into())
        .spawn(move || {
            let mut decoder = Decoder::new(SAMPLE_RATE, is_ft8);
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
        let mut qso = QsoEngine::new(my_call, my_grid);
        qso.band = bands::band_for_freq_hz(dial_hz).to_string();
        qso.freq_mhz = bands::freq_mhz_string(dial_hz);
        qso.mode = mode.display_name().to_string();

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
        }
        .sanitize();
        let wf = WfProcessor::new(wf_cfg);

        let (slot_tx, dec_rx) = spawn_decoder_thread(mode.is_ft8());

        Engine {
            db,
            evt,
            accum: Arc::new(SlotAccumulator::new()),
            slot_tx,
            dec_rx,
            mode,
            txed_slot: -1,
            qso,
            rig: None,
            input: None,
            input_device,
            output_device,
            decoding: false,
            dial_hz,
            tx_audio_hz,
            tx_gain,
            tx_left_channel_only: false,
            tx_via_cat_tones: false,
            rig_supports_meters: false,
            tx_cat: None,
            tx_tune: None,
            rx_gain,
            last_decoded_slot: -1,
            pending_decodes: 0,
            rx_offset_ms,
            last_tick_ms: 0,
            last_ptt_on_sent: None,
            tx_parity: None,
            preferred_tx_parity: None,
            clock_offset_ms: saved_offset.unwrap_or(0),
            time_synced: saved_offset.is_some(),
            udp: crate::udp::UdpService::new(cmd_tx.clone()),
            cmd_tx,
            ptt: false,
            tx_playback: None,
            wf,
            last_wf_slot: -1,
            last_udp_heartbeat_ms: 0,
        }
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
        if let Some(json) = self.db.get_config("rig_config") {
            if let Ok(cfg) = serde_json::from_str::<RigConfig>(&json) {
                if cfg.backend != crate::rig::RigBackend::None {
                    self.select_rig(cfg);
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
                    self.emit(EngineEvent::InputLevel(InputLevelEvent {
                        db,
                        silent: db <= SILENCE_DBFS,
                    }));
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
                    self.poll_meters();
                }
            }

            // CAT PTT keepalive (G106). Checked every loop pass against a
            // monotonic clock, not on the 1 Hz UI tick, which runs on the
            // NTP-corrected clock and can stall when the offset steps backwards.
            self.refresh_ptt_keepalive();

            // Decode stays anchored to the UTC slot. Do not shift this from a
            // decoded message's DT: doing so changes the sample chosen as the slot
            // start and can cut off the next message's leading Costas sync block.
            // We fire once per slot as soon as the 12.64 s waveform is captured
            // (~13.2 s in) rather than at the 15 s boundary, so decodes land a slot
            // earlier and the operator can reply on the next cycle (not skip one).
            let rx_slot_id = now.div_euclid(self.mode.cycle_ms());
            let into_rx_cycle = now.rem_euclid(self.mode.cycle_ms());
            if self.decoding
                && rx_slot_id != self.last_decoded_slot
                && into_rx_cycle >= self.mode.decode_at_ms()
            {
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
                self.handle_decoded(msgs, slot_id);
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
                self.mode.tx_latest_ms(),
            ) {
                self.maybe_transmit(slot_id);
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
        self.publish_decodes(&decoded);
        self.broadcast_decodes(&decoded);

        if let Some(QsoOutcome::Completed(record)) = self.qso.process_rx(&decoded) {
            match self.db.insert_qso(&record) {
                Ok(_) => {
                    self.broadcast_qso(&record);
                    self.emit(EngineEvent::QsoCompleted(record));
                }
                Err(e) => self.emit(EngineEvent::Error(format!("log QSO failed: {e}"))),
            }
        }
        self.maybe_transmit(slot_id);
        self.publish_tx_state();
    }

    /// Key up this slot's QSO message if we're armed, it's our turn, and the
    /// 12.64 s waveform still fits before the next boundary. Idempotent per slot
    /// (`txed_slot` guard), so calling it from both `handle_decoded` and a command
    /// handler in the same slot transmits at most once.
    fn maybe_transmit(&mut self, slot_id: i64) {
        // All the keying rules — active QSO, locked-parity alternation, once per
        // slot, and early enough in the cycle to fit the waveform — live in the pure
        // `tx_slot_eligible` so the run loop's boundary trigger shares them exactly.
        if !tx_slot_eligible(
            self.qso.active,
            self.tx_parity,
            slot_id,
            self.txed_slot,
            self.now().rem_euclid(self.mode.cycle_ms()),
            self.mode.tx_latest_ms(),
        ) {
            return;
        }
        // Answering a CQ pins the parity to the operator's click slot so we always
        // reply opposite the DX; a CQ / free-text start locks it on this first TX.
        self.tx_parity = Some(slot_id.rem_euclid(2));
        self.txed_slot = slot_id;
        if let Some(msg) = self.qso.tx_message().map(|s| s.to_string()) {
            if self.start_transmit(&msg) {
                // Tell the sequencer the message actually went out. This is what
                // lets a courtesy "73" wrap the QSO up only after it's on the air;
                // if start_transmit bailed (busy/encode error) the QSO stays armed
                // and retransmits next slot instead of clobbering the 73.
                self.qso.notify_transmitted();
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

    /// Switch operating mode (FT8/FT4) -- see `EngineCommand::SetMode`.
    ///
    /// Stops any in-flight TX/QSO/tune first, the same precaution
    /// `DisconnectRig` takes: a mode's cycle length and tone encoding are
    /// baked into a transmission from the moment it starts (the CAT-tone
    /// path in particular captures `symbol_period` once into `CatToneTx`),
    /// so switching mode mid-transmission would desync the rig from what's
    /// actually still being clocked out. A no-op (but still safe to call)
    /// when `mode` is already the current mode.
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
        self.qso.stop();
        self.tx_parity = None;
        self.finalize_tx();

        let (slot_tx, dec_rx) = spawn_decoder_thread(mode.is_ft8());
        self.slot_tx = slot_tx;
        self.dec_rx = dec_rx;

        self.mode = mode;
        self.qso.mode = mode.display_name().to_string();
        self.last_decoded_slot = -1;
        self.pending_decodes = 0;
        self.txed_slot = -1;
        self.last_wf_slot = -1;

        let _ = self.db.set_config("operating_mode", mode.display_name());
        self.emit(EngineEvent::Info(format!("switched to {}", mode.display_name())));
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
            self.tx_left_channel_only,
        ) {
            Ok(p) => p,
            Err(e) => {
                self.emit(EngineEvent::Error(format!("playback failed: {e}")));
                return false;
            }
        };

        // Key up, let the rig's T/R relay settle, then start clocking samples.
        self.set_ptt(true);
        std::thread::sleep(Duration::from_millis(PTT_DELAY_MS));

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
        std::thread::sleep(Duration::from_millis(PTT_DELAY_MS));

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
        self.set_ptt(true);
        std::thread::sleep(Duration::from_millis(PTT_DELAY_MS));
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
            match output::prepare_tune(
                self.output_device.as_deref(),
                self.tx_audio_hz as f32,
                self.tx_gain,
                self.tx_left_channel_only,
            ) {
                Ok(h) => self.tx_tune = Some(TuneHandle::Audio(h)),
                Err(e) => {
                    self.emit(EngineEvent::Error(format!("tune failed: {e}")));
                    self.set_ptt(false);
                    return;
                }
            }
        }
        self.publish_tx_state();
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
        let tune_was_cat = matches!(had_tune, Some(TuneHandle::Cat));
        if had_cat || tune_was_cat {
            // A CAT-tone transmission doesn't get polled live (see the 1Hz
            // tick block's comment on why) -- take one last reading here
            // instead, while RF from the final symbol is still live, just
            // before the key-up command below silences it.
            if had_cat && self.rig_supports_meters {
                self.poll_meters();
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
            self.set_ptt(false);
            self.publish_tx_state();
        }
    }

    /// One SWR/power reading over CAT (QMX only -- gated by
    /// `rig_supports_meters`, checked by callers). Emits a `Meter` event only
    /// if at least one of the two queries returned a valid reading; a failed
    /// or unparseable reply just means no event this call, not an error.
    fn poll_meters(&mut self) {
        let Some(rig) = self.rig.as_mut() else { return };
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

    fn publish_decodes(&self, decoded: &[DecodedMessage]) {
        let now = self.now();
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
        }));
        // Mirror the state change out over WSJT-X UDP so companion apps see TX
        // start/stop and QSO progress without waiting for the next 1 Hz tick.
        self.broadcast_status();
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
                // Tell companion apps to wipe their decode band and forget the
                // replay history — this is a genuine reset, not a per-cycle churn.
                if self.udp.is_enabled() {
                    self.udp.send(&crate::udp::codec::clear());
                }
                self.udp.clear_replay_cache();
                self.broadcast_status();
                self.emit(EngineEvent::Info("decode stopped".into()));
            }
            EngineCommand::SetStation { call, grid } => {
                self.qso.my_call = call.to_uppercase();
                self.qso.my_grid = util::grid4(&grid).to_uppercase();
                let _ = self.db.set_config("my_call", &self.qso.my_call);
                let _ = self.db.set_config("my_grid", &self.qso.my_grid);
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
            }
            EngineCommand::SetBaseFreq(hz) => {
                self.tx_audio_hz = hz.clamp(MIN_TX_AUDIO_HZ, MAX_TX_AUDIO_HZ);
                let _ = self.db.set_config("base_freq", &self.tx_audio_hz.to_string());
            }
            EngineCommand::SetTxGain(g) => {
                self.tx_gain = clamp_tx_gain(g);
                let _ = self.db.set_config("tx_gain", &self.tx_gain.to_string());
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
            EngineCommand::SetWaterfallConfig(cfg) => {
                let cfg = cfg.sanitize();
                let _ = self.db.set_config("wf_window", cfg.window.as_str());
                let _ = self.db.set_config("wf_fft_size", &cfg.fft_size.to_string());
                let _ = self.db.set_config("wf_avg", &cfg.avg.to_string());
                // Rebuild plan + window table; the noise-floor EMA restarts and
                // reconverges within a couple of seconds.
                self.wf = WfProcessor::new(cfg);
                self.emit(EngineEvent::Info(format!(
                    "waterfall: {} {} avg {}",
                    cfg.window.as_str(),
                    cfg.fft_size,
                    cfg.avg
                )));
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
                self.preferred_tx_parity = parity;
                self.publish_tx_state();
            }
            EngineCommand::SetMode(mode) => self.switch_mode(mode),
            EngineCommand::StartCq => {
                self.tx_parity = self.preferred_tx_parity;
                self.qso.start_cq();
                // Start CQ this very slot if we're still inside the window;
                // otherwise the first transmission lands at the next free slot.
                self.maybe_transmit(self.cur_slot());
                self.publish_tx_state();
            }
            EngineCommand::Answer(args) => {
                // Honor the audio tone the companion asked us to answer on (WSJT-X's
                // `df`) for *this session* when it carries a plausible one, so we key
                // up on the requested offset rather than the current TX tone —
                // matching the Android/iOS ports. This is an in-memory-only change:
                // the `df` arrives on an untrusted UDP socket, so it must never
                // rewrite the operator's persisted `base_freq` (only SetBaseFreq / the
                // UI do that, and the saved offset survives a restart). The desktop
                // UI's own "click to answer" sends df=0 (offset unchanged).
                self.apply_reply_df(args.delta_freq);
                let msg = DecodedMessage {
                    call_from: args.call_from,
                    grid: args.grid,
                    snr: args.snr,
                    ..Default::default()
                };
                // Reply opposite the DX: the operator is clicking during the slot
                // after the CQ, so pin TX to this slot's parity and key up right
                // away if we're still inside the window — matching WSJT-X.
                self.tx_parity = Some(self.cur_parity());
                self.qso.answer(&msg);
                self.maybe_transmit(self.cur_slot());
                self.publish_tx_state();
            }
            EngineCommand::SetStage(stage) => {
                self.tx_parity = None;
                self.qso.set_stage(stage);
                self.maybe_transmit(self.cur_slot());
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
                self.qso.set_free_text(&text);
                self.maybe_transmit(self.cur_slot());
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
                crate::timesync::spawn_once(self.cmd_tx.clone());
            }
            EngineCommand::Shutdown => {}
        }
    }

    fn start_decode(&mut self) {
        match AudioInput::start(self.input_device.as_deref(), self.accum.clone(), self.rx_gain) {
            Ok(input) => {
                self.emit(EngineEvent::Info(format!(
                    "capturing from '{}' @ {} Hz",
                    input.device_name, input.device_rate
                )));
                self.input = Some(input);
                self.decoding = true;
            }
            Err(e) => self.emit(EngineEvent::Error(format!("audio start failed: {e}"))),
        }
    }

    fn select_rig(&mut self, cfg: RigConfig) {
        // Close any existing connection FIRST. Some CAT servers (notably
        // SmartSDR CAT) accept only one client, so opening the new session while
        // the old one is still open makes the new handshake time out. Dropping
        // here runs the old rig's close/cleanup before we reconnect.
        self.rig = None;
        self.tx_left_channel_only = crate::rig::needs_left_channel_tx_audio(cfg.backend, cfg.hamlib_model);
        self.tx_via_cat_tones = crate::rig::needs_cat_tone_tx(cfg.backend, cfg.hamlib_model);
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
                } else if self.tx_left_channel_only {
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
        clamp_rx_gain, clamp_tx_gain, reply_tx_audio_hz, rms_dbfs, should_refresh_ptt,
        waterfall_row_due, wf_boundary_row, Engine, EngineCommand, DEFAULT_RX_GAIN,
        DEFAULT_TX_GAIN, MAX_TX_AUDIO_HZ, MIN_TX_AUDIO_HZ, PTT_KEEPALIVE_INTERVAL, SILENCE_DBFS,
    };
    use crate::db::Db;
    use crate::dsp::DecodedMessage;
    use crate::mode::Mode;
    use std::sync::Arc;
    use std::time::Duration;

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
    fn tx_slot_eligible_enforces_active_parity_once_and_window() {
        use super::tx_slot_eligible;
        let tx_latest_ms = Mode::Ft8.tx_latest_ms();
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
        let ft4_latest = Mode::Ft4.tx_latest_ms();
        assert_ne!(ft4_latest, Mode::Ft8.tx_latest_ms());
        assert!(tx_slot_eligible(true, None, 10, -1, ft4_latest, ft4_latest));
        assert!(!tx_slot_eligible(true, None, 10, -1, ft4_latest + 1, ft4_latest));
    }

    #[test]
    fn boundary_tx_keys_at_slot_top_only_after_this_slots_decode() {
        use super::boundary_tx_ready;
        let tx_latest_ms = Mode::Ft8.tx_latest_ms();
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
        let tx_latest_ms = Mode::Ft8.tx_latest_ms();
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
}
