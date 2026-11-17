// FT8AF desktop — Tauri entry point. Owns the SQLite DB and the runtime engine,
// exposes commands to the web UI, and forwards engine events to the webview.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, State};

use ft8af::audio::{self, AudioDevice};
use ft8af::bands::{self, BandEntry, CustomBand};
use ft8af::db::{ConfirmedFilter, Db, ImportProgress, ImportSummary, LogStats, QsoRecord};
use ft8af::engine::{self, AnswerArgs, EngineCommand, EngineHandle};
use ft8af::fcc_grid::{self, FccSeedSummary};
use ft8af::lotw::{self, LotwDownloadResult, LotwUploadResult};
use ft8af::map_tiles::MapTiles;
use ft8af::os_location::{self, OsLocation};
use ft8af::psk_reporter::{PskFetchResult, PskReporterState};
use ft8af::qrz::{self, QrzUploadResult};
use ft8af::rig::{self, HamlibRig, RigConfig, SerialPortInfo};
use ft8af::settings_backup::{self, ParsedBackup};
use ft8af::wf::WfConfig;

struct AppState {
    engine: EngineHandle,
    db: Arc<Db>,
    map_tiles: MapTiles,
    psk_reporter: PskReporterState,
}

// --- device / port / band enumeration --------------------------------------

#[tauri::command]
fn list_audio_inputs() -> Vec<AudioDevice> {
    audio::list_input_devices()
}

#[tauri::command]
fn list_audio_outputs() -> Vec<AudioDevice> {
    audio::list_output_devices()
}

#[tauri::command]
fn list_serial_ports() -> Vec<SerialPortInfo> {
    rig::list_serial_ports()
}

#[tauri::command]
fn list_hamlib_rigs() -> Vec<HamlibRig> {
    rig::list_hamlib_rigs()
}

/// Read + parse the persisted custom-band list from the config store.
fn load_custom_bands(db: &Db) -> Vec<CustomBand> {
    bands::parse_custom_bands(&db.get_config(bands::CUSTOM_BANDS_KEY).unwrap_or_default())
}

#[tauri::command]
fn list_bands(state: State<AppState>) -> Vec<BandEntry> {
    bands::merged_bands(&load_custom_bands(&state.db))
}

/// Band entries for a specific mode (FT8's full list with customs merged in,
/// or FT4's smaller canonical list) -- drives `BandModeSheet`'s band list,
/// which must follow whichever mode tile is currently selected.
#[tauri::command]
fn list_bands_for_mode(state: State<AppState>, mode: ft8af::mode::Mode) -> Vec<BandEntry> {
    bands::bands_for_mode_entries(mode, &load_custom_bands(&state.db))
}

#[tauri::command]
fn list_custom_bands(state: State<AppState>) -> Vec<CustomBand> {
    load_custom_bands(&state.db)
}

/// Add (or rename, if the dial already exists) a custom band. `freq` is the raw
/// operator input, validated here so a clear error surfaces in the UI. Returns
/// the merged band list on success.
#[tauri::command]
fn add_custom_band(
    state: State<AppState>,
    freq: String,
    name: String,
) -> Result<Vec<BandEntry>, String> {
    let dial_hz = bands::parse_dial_hz(&freq)?;
    let mut custom = load_custom_bands(&state.db);
    bands::upsert_custom_band(&mut custom, &name, dial_hz);
    state
        .db
        .set_config(bands::CUSTOM_BANDS_KEY, &bands::serialize_custom_bands(&custom))
        .map_err(|e| e.to_string())?;
    Ok(bands::merged_bands(&custom))
}

#[tauri::command]
fn delete_custom_band(state: State<AppState>, dial_hz: u64) -> Result<Vec<BandEntry>, String> {
    let mut custom = load_custom_bands(&state.db);
    bands::remove_custom_band(&mut custom, dial_hz);
    state
        .db
        .set_config(bands::CUSTOM_BANDS_KEY, &bands::serialize_custom_bands(&custom))
        .map_err(|e| e.to_string())?;
    Ok(bands::merged_bands(&custom))
}

// --- engine control ---------------------------------------------------------

#[tauri::command]
fn start_decode(state: State<AppState>) {
    state.engine.send(EngineCommand::StartDecode);
}

#[tauri::command]
fn stop_decode(state: State<AppState>) {
    state.engine.send(EngineCommand::StopDecode);
}

#[tauri::command]
fn set_station(state: State<AppState>, call: String, grid: String) {
    state.engine.send(EngineCommand::SetStation { call, grid });
}

#[tauri::command]
fn set_band(state: State<AppState>, dial_hz: u64) {
    state.engine.send(EngineCommand::SetBand(dial_hz));
}

#[tauri::command]
fn set_base_freq(state: State<AppState>, hz: i32) {
    state.engine.send(EngineCommand::SetBaseFreq(hz));
}

#[tauri::command]
fn set_tx_gain(state: State<AppState>, gain: f32) {
    state.engine.send(EngineCommand::SetTxGain(gain));
}

#[tauri::command]
fn set_rx_gain(state: State<AppState>, gain: f32) {
    state.engine.send(EngineCommand::SetRxGain(gain));
}

#[tauri::command]
fn set_input_device(state: State<AppState>, name: Option<String>) {
    state.engine.send(EngineCommand::SetInputDevice(name));
}

#[tauri::command]
fn set_output_device(state: State<AppState>, name: Option<String>) {
    state.engine.send(EngineCommand::SetOutputDevice(name));
}

#[tauri::command]
fn select_rig(state: State<AppState>, config: RigConfig) {
    state.engine.send(EngineCommand::SelectRig(config));
}

#[tauri::command]
fn set_rx_audio_channel(state: State<AppState>, channel: i32) {
    state.engine.send(EngineCommand::SetRxAudioChannel(channel));
}

/// Settings -> Advanced's "Decode Depth" toggle (Fast/Deep).
#[tauri::command]
fn set_decode_depth(state: State<AppState>, deep: bool) {
    state.engine.send(EngineCommand::SetDecodeDepth(deep));
}

#[tauri::command]
fn set_tx_audio_channel(state: State<AppState>, channel: i32) {
    state.engine.send(EngineCommand::SetTxAudioChannel(channel));
}

#[tauri::command]
fn set_qmx_cat_tones(state: State<AppState>, enabled: bool) {
    state.engine.send(EngineCommand::SetQmxCatTones(enabled));
}

#[tauri::command]
fn disconnect_rig(state: State<AppState>) {
    state.engine.send(EngineCommand::DisconnectRig);
}

/// Settings -> Transmission -> TX Protection's SWR Halt toggle.
#[tauri::command]
fn set_swr_halt_enabled(state: State<AppState>, enabled: bool) {
    state.engine.send(EngineCommand::SetSwrHaltEnabled(enabled));
}

/// SWR Halt's threshold, a plain ratio (e.g. 3.0).
#[tauri::command]
fn set_swr_halt_threshold(state: State<AppState>, threshold: f32) {
    state.engine.send(EngineCommand::SetSwrHaltThreshold(threshold));
}

/// Operator dismisses the SWR lockout banner, allowing TX again.
#[tauri::command]
fn clear_swr_lockout(state: State<AppState>) {
    state.engine.send(EngineCommand::ClearSwrLockout);
}

#[tauri::command]
fn set_tune_max_on_seconds(state: State<AppState>, seconds: i64) {
    state.engine.send(EngineCommand::SetTuneMaxOnSeconds(seconds));
}

#[tauri::command]
fn set_tune_method(state: State<AppState>, method: i32) {
    state.engine.send(EngineCommand::SetTuneMethod(method));
}

#[tauri::command]
fn set_tune_level_independent(state: State<AppState>, independent: bool) {
    state.engine.send(EngineCommand::SetTuneLevelIndependent(independent));
}

#[tauri::command]
fn set_tune_level(state: State<AppState>, gain: f32) {
    state.engine.send(EngineCommand::SetTuneLevel(gain));
}

/// Settings -> Transmission's "TX Watchdog" ("Stop After" N unanswered attempts).
#[tauri::command]
fn set_tx_watchdog(state: State<AppState>, limit: u32) {
    state.engine.send(EngineCommand::SetTxWatchdog(limit));
}

#[tauri::command]
fn set_max_73_sends(state: State<AppState>, cap: u32) {
    state.engine.send(EngineCommand::SetMax73Sends(cap));
}

/// Settings -> Transmission's "CQ Watchdog" -- how many unanswered CQ
/// cycles before Call CQ gives itself up; 0 = unlimited (robot/unattended
/// operation). No Android equivalent -- see `QsoEngine::cq_no_reply_limit`.
#[tauri::command]
fn set_cq_watchdog(state: State<AppState>, limit: u32) {
    state.engine.send(EngineCommand::SetCqWatchdog(limit));
}

/// Settings -> Auto-Sequence's "Auto-CQ after QSO".
#[tauri::command]
fn set_auto_cq_after_qso(state: State<AppState>, enabled: bool) {
    state.engine.send(EngineCommand::SetAutoCqAfterQso(enabled));
}

/// Settings -> Transmission's "Clear decodes on band/mode change".
#[tauri::command]
fn set_clear_on_band_mode_change(state: State<AppState>, enabled: bool) {
    state.engine.send(EngineCommand::SetClearOnBandModeChange(enabled));
}

/// RX audio capture latency compensation (ms) -- see `rx_latency_adjusted_into_cycle_ms`'s doc comment.
#[tauri::command]
fn set_rx_latency_ms(state: State<AppState>, ms: i64) {
    state.engine.send(EngineCommand::SetRxLatencyMs(ms));
}

#[tauri::command]
fn set_ptt_delay_ms(state: State<AppState>, ms: i64) {
    state.engine.send(EngineCommand::SetPttDelayMs(ms));
}

#[tauri::command]
fn set_late_start_tolerance_ms(state: State<AppState>, ms: i64) {
    state.engine.send(EngineCommand::SetLateStartToleranceMs(ms));
}

#[tauri::command]
fn refresh_status(state: State<AppState>) {
    state.engine.send(EngineCommand::RefreshStatus);
}

#[tauri::command]
fn resync_time(state: State<AppState>) {
    state.engine.send(EngineCommand::ResyncTime);
}

/// Settings -> Time Sync's manual clock-correction stepper, for offline
/// operating when NTP can't reach a server (Android's own
/// `timeCorrectionMs`/`UtcTimer.delay`) -- reuses the exact same
/// `SetClockOffset` command/code path an NTP sync success already uses,
/// since both are "establish this offset as current truth" with identical
/// semantics; a manual value is just another source for it.
#[tauri::command]
fn set_clock_offset(state: State<AppState>, offset_ms: i64) {
    state.engine.send(EngineCommand::SetClockOffset(offset_ms));
}

#[tauri::command]
fn start_cq(state: State<AppState>) {
    state.engine.send(EngineCommand::StartCq);
}

#[tauri::command]
fn answer(state: State<AppState>, args: AnswerArgs) {
    state.engine.send(EngineCommand::Answer(args));
}

#[tauri::command]
fn set_stage(state: State<AppState>, stage: ft8af::qso::TxStage) {
    state.engine.send(EngineCommand::SetStage(stage));
}

#[tauri::command]
fn stop_tx(state: State<AppState>) {
    state.engine.send(EngineCommand::StopTx);
}

#[tauri::command]
fn set_tx_period(state: State<AppState>, parity: Option<i64>) {
    state.engine.send(EngineCommand::SetTxPeriod(parity));
}

#[tauri::command]
fn set_mode(state: State<AppState>, mode: ft8af::mode::Mode) {
    state.engine.send(EngineCommand::SetMode(mode));
}

#[tauri::command]
fn start_tune(state: State<AppState>) {
    state.engine.send(EngineCommand::StartTune);
}

#[tauri::command]
fn stop_tune(state: State<AppState>) {
    state.engine.send(EngineCommand::StopTune);
}

#[tauri::command]
fn free_text(state: State<AppState>, text: String) {
    state.engine.send(EngineCommand::FreeText(text));
}

// --- logbook / config -------------------------------------------------------

#[tauri::command]
fn list_log(state: State<AppState>, limit: i64, offset: i64) -> Vec<QsoRecord> {
    state.db.list_qsos(limit, offset)
}

/// Feeds the QSO Sheet's "Worked before" card -- see `Db::prior_qsos`.
#[tauri::command]
fn prior_qsos(state: State<AppState>, callsign: String) -> Vec<QsoRecord> {
    state.db.prior_qsos(&callsign)
}

/// Server-side search/filter for the Logbook tab. `filter` is one of
/// `"all"` / `"confirmed"` / `"unconfirmed"`; anything else is treated as All.
#[tauri::command]
fn search_log(
    state: State<AppState>,
    callsign: String,
    filter: String,
    sort_column: i64,
    sort_ascending: bool,
    limit: i64,
    offset: i64,
) -> Vec<QsoRecord> {
    let filter = match filter.as_str() {
        "confirmed" => ConfirmedFilter::Confirmed,
        "unconfirmed" => ConfirmedFilter::Unconfirmed,
        _ => ConfirmedFilter::All,
    };
    state.db.search_qsos(&callsign, filter, sort_column, sort_ascending, limit, offset)
}

#[tauri::command]
fn log_count(state: State<AppState>) -> i64 {
    state.db.count()
}

/// Calls already worked on the currently-tuned band, for the Decode list to
/// strike through. Reads the same `dial_hz` config engine.rs itself uses when
/// stamping a logged QSO's `band`, so this always matches whatever band a
/// freshly-worked QSO would actually be filed under. `mode` is passed in by
/// the caller (desktop only ever logs "FT8" today -- see qso.rs).
#[tauri::command]
fn worked_calls(state: State<AppState>, mode: String) -> Vec<String> {
    let dial_hz: u64 = state
        .db
        .get_config("dial_hz")
        .and_then(|v| v.parse().ok())
        .unwrap_or(bands::default_band().dial_hz);
    state.db.worked_calls(bands::band_for_freq_hz(dial_hz), &mode)
}

/// Calls already worked on `mode`, any band -- for the Decode list's
/// New-Band status (worked elsewhere, but not the currently-tuned band).
#[tauri::command]
fn worked_calls_any_band(state: State<AppState>, mode: String) -> Vec<String> {
    state.db.worked_calls_any_band(&mode)
}

/// Grid squares already logged (any band/mode), for the Decode list's Hunt
/// "new grid" priority and New-Grid filter.
#[tauri::command]
fn worked_grids(state: State<AppState>) -> Vec<String> {
    state.db.worked_grids()
}

/// CQ WPX prefixes already logged (any band/mode), for the Decode list's
/// New-Prefix filter.
#[tauri::command]
fn worked_prefixes(state: State<AppState>) -> Vec<String> {
    state.db.worked_prefixes()
}

/// DXCC entity prefixes already logged (any band/mode), for the Decode
/// list's New-DXCC filter.
#[tauri::command]
fn worked_dxcc_prefixes(state: State<AppState>) -> Vec<String> {
    state.db.worked_dxcc_prefixes()
}

/// CQ zones already logged (any band/mode), for the Decode list's New-Zone
/// filter.
#[tauri::command]
fn worked_cq_zones(state: State<AppState>) -> Vec<i64> {
    state.db.worked_cq_zones()
}

/// US states already logged on the currently-tuned band/mode (WAS is per
/// band and mode -- see `Db::worked_states`), for the Decode list's
/// New-State filter.
#[tauri::command]
fn worked_states(state: State<AppState>, mode: String) -> Vec<String> {
    let dial_hz: u64 = state
        .db
        .get_config("dial_hz")
        .and_then(|v| v.parse().ok())
        .unwrap_or(bands::default_band().dial_hz);
    state.db.worked_states(bands::band_for_freq_hz(dial_hz), &mode)
}

/// Aggregate Logbook stats for the Stats/Awards tabs (totals, DXCC/CQ/ITU
/// zone and grid-square counts, band distribution, WAC/WAS progress, Best
/// DX) -- see `Db::log_stats`. `my_grid` drives the Best DX distance; an
/// empty or unparseable grid just means no Best DX card, not an error.
#[tauri::command]
fn log_stats(state: State<AppState>, my_grid: String) -> LogStats {
    state.db.log_stats(&my_grid)
}

#[tauri::command]
fn delete_qso(state: State<AppState>, id: i64) -> Result<(), String> {
    state.db.delete_qso(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_qso(state: State<AppState>, record: QsoRecord) -> Result<i64, String> {
    state.db.insert_qso(&record).map_err(|e| e.to_string())
}

/// Export the log as ADIF, optionally restricted to an inclusive `[start, end]`
/// `YYYYMMDD` date range. Empty strings / omitted args mean an open bound.
#[tauri::command]
fn export_adif(
    state: State<AppState>,
    start: Option<String>,
    end: Option<String>,
) -> String {
    state
        .db
        .export_adif_range(start.as_deref(), end.as_deref())
}

/// Native "pick an ADIF file" dialog -- returns the chosen path, or `None`
/// on Cancel. A native dialog (not an `<input type="file">`) for two
/// reasons: it's immune to the WebKitGTK repaint-freeze bug on this board
/// (confirmed in the grid-seed-popup round, same plugin), and it's what lets
/// `import_adif` take a path instead of file *contents* -- see that
/// command's doc comment for why the content never crossing IPC is the
/// whole point of this change.
///
/// `async fn`, not a plain `fn`: Tauri dispatches a non-async command on the
/// main thread, and `blocking_pick_file()`'s own doc comment is explicit
/// that it "should *NOT* be used when running on the main thread" (its
/// example is `async fn`) -- a plain `fn` here hung forever with no dialog
/// ever appearing, confirmed live.
#[tauri::command]
async fn pick_adif_file(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter("ADIF log", &["adi", "adif", "ADI", "ADIF"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Import an ADIF file, given its path, into the log. See
/// `Db::import_adif_file_with_progress` for the duplicate-matching rule and
/// the streaming design. Emits `"import-progress"` events (payload:
/// [`ImportProgress`]) so the UI can show a live counter instead of a
/// silent wait on a large file.
///
/// Takes a **path**, not file contents: an earlier version took the whole
/// file as a `String` argument (read client-side via `file.text()`, sent
/// over IPC as one JSON payload) -- that buffered a file at least 3-4 times
/// over (the JS string, the IPC payload, the Rust `String`, the fully
/// parsed record list) before a single row was written, and locked up this
/// 3.8GB board twice on a real ~80MB log. Opening the file directly here
/// means the content never crosses IPC at all.
///
/// Wrapped in `catch_unwind`: this is the one command that parses arbitrary
/// external file content, and on Linux a Tauri command runs inside
/// webkit2gtk's C callback, where an unwinding panic can't cross back out —
/// it aborts the entire running app instead of just failing this call. The
/// parser itself (`adif::AdifStreamReader`) is written to never panic, but
/// this is cheap insurance against anything we missed.
///
/// `async fn`, not a plain `fn`, for the same reason as `pick_adif_file`:
/// a non-async command runs on the main thread, which would otherwise
/// freeze the whole window for as long as a large import takes (the
/// progress events still have somewhere to be delivered, but the UI
/// couldn't repaint to show them).
/// Resumes the decoder on drop, however `import_adif` returns (success,
/// error, or an unwinding panic caught by `catch_unwind` below). Holding
/// this for the whole command body is what makes the pause/resume pair
/// unconditional instead of needing a matching `SetDecodeSuspended(false)`
/// on every return path.
struct DecodeSuspendGuard<'a>(&'a EngineHandle);

impl Drop for DecodeSuspendGuard<'_> {
    fn drop(&mut self) {
        self.0.send(EngineCommand::SetDecodeSuspended(false));
    }
}

#[tauri::command]
async fn import_adif(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<ImportSummary, String> {
    // A large ADIF import is a long burst of DB writes competing with the
    // engine thread's always-on FT8 decode work for CPU on this board's
    // class of hardware -- confirmed live: the identical import took 10+
    // minutes inside the running app but 34s in isolation with the decoder
    // idle, against the same code and data. Suspending the decoder for the
    // duration (resumed by `DecodeSuspendGuard` on drop) buys the import
    // back that CPU without touching anything else live decoding gates --
    // see `EngineCommand::SetDecodeSuspended`'s doc comment.
    state.engine.send(EngineCommand::SetDecodeSuspended(true));
    let _resume_decoder = DecodeSuspendGuard(&state.engine);

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state
            .db
            .import_adif_file_with_progress(std::path::Path::new(&path), |processed, total| {
                let _ = app.emit("import-progress", ImportProgress { processed, total });
            })
    }))
    .map_err(|_| {
        "ADIF import hit an unexpected error partway through and stopped; QSOs already \
         imported before that point are kept, nothing after it was touched."
            .to_string()
    })?
    .map_err(|e| format!("Couldn't read that file: {e}"))
}

// --- Settings backup/export/import (Advanced category) ---------------------

/// Serialize the whole config table (+ the Hunt watchlist) as backup JSON --
/// returned as a string, same shape as `export_adif`, so the frontend can
/// save it with the identical Blob+`<a download>` trick already proven for
/// the ADIF export (lands in the OS Downloads folder with no native dialog
/// needed, unlike import below).
#[tauri::command]
fn export_settings_backup(state: State<AppState>, include_sensitive: bool) -> String {
    let config = state.db.all_config();
    let follow_callsigns = state.db.list_follow_callsigns();
    let created_at = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
    settings_backup::build_backup_json(
        &config,
        include_sensitive,
        env!("CARGO_PKG_VERSION"),
        &created_at,
        follow_callsigns,
    )
}

/// Native "pick a settings backup file" dialog, same reasoning as
/// `pick_adif_file` (immune to the WebKitGTK repaint-freeze bug; lets the
/// following read happen by path rather than piping file contents through
/// IPC as a `<input type="file">` would).
#[tauri::command]
async fn pick_settings_backup_file(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter("FT8AF settings backup", &["json"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Read and validate a settings backup file, for the import confirmation
/// dialog to preview (key count, created-at) before anything is applied.
#[tauri::command]
fn preview_settings_backup(path: String) -> Result<ParsedBackup, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Couldn't read that file: {e}"))?;
    settings_backup::parse_backup_json(&text)
}

/// Apply a previewed backup: writes every config key, and adds every
/// followed callsign (union with whatever's already in the watchlist,
/// matching `add_follow_callsign`'s own insert-or-ignore semantics --
/// restoring a backup never *removes* a callsign the operator added since).
#[tauri::command]
fn apply_settings_backup(
    state: State<AppState>,
    config: std::collections::BTreeMap<String, String>,
    follow_callsigns: Vec<String>,
) -> Result<(), String> {
    // Android's own Control Mode setting is a flat `ctrMode` config key
    // (its own int-coded `database.ControlMode`); desktop's equivalent
    // lives *nested* inside the serialized `rig_config` blob's
    // `control_mode` field, not as a standalone key of its own. Writing
    // `ctrMode` verbatim via the loop below (same as every other key)
    // would leave it an inert, unread row -- confirmed live: a real
    // import that included `ctrMode=1` (CAT) never changed the rig's
    // actual control mode, because nothing reads a bare "ctrMode" key.
    // Translate it into the live `rig_config` blob here instead.
    if let Some(ctr_mode) = config.get("ctrMode") {
        if let Some(mode) = rig::control_mode_from_android_ctr_mode(ctr_mode) {
            // "{}" rather than `RigConfig::default()` (no such derive) --
            // every field already has its own `#[serde(default = ...)]`,
            // so deserializing an empty object gives the same per-field
            // fallbacks `rig_config` is loaded with everywhere else.
            let mut cfg: RigConfig = state
                .db
                .get_config("rig_config")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_else(|| serde_json::from_str("{}").expect("RigConfig fields all have serde defaults"));
            cfg.control_mode = mode;
            if let Ok(json) = serde_json::to_string(&cfg) {
                let _ = state.db.set_config("rig_config", &json);
            }
        }
    }
    // Same translation gap, same fix shape: Android's manual clock
    // correction is its own flat `timeCorrectionMs` key (applied to
    // `UtcTimer.delay`); desktop's equivalent is `clock_offset_ms` (applied
    // to `self.now()` the same way, same ±5000ms range -- see
    // `timeCorrection.ts`'s `clampCorrectionMs`). Writing `timeCorrectionMs`
    // verbatim via the loop below would leave it just as inert as `ctrMode`
    // was.
    if let Some(ms) = config.get("timeCorrectionMs").and_then(|s| s.trim().parse::<i64>().ok()) {
        let clamped = ms.clamp(-5000, 5000);
        let _ = state.db.set_config("clock_offset_ms", &clamped.to_string());
    }
    for (k, v) in config {
        state.db.set_config(&k, &v).map_err(|e| e.to_string())?;
    }
    for call in follow_callsigns {
        state.db.add_follow_callsign(&call);
    }
    Ok(())
}

/// Total rows in `callsign_qth` -- the frontend's first-run check: an empty
/// table means `import_fcc_grid_seed` hasn't run yet (see `db.rs` schema
/// v7's doc comment).
#[tauri::command]
fn callsign_qth_count(state: State<AppState>) -> i64 {
    state.db.callsign_qth_count()
}

/// The cached grid for one callsign, if any -- the Map screen's per-message
/// lookup for TX3-5/73/R73 decodes, which don't carry a grid themselves
/// (see `db.rs::qth_for_call`'s doc comment). Looked up one callsign at a
/// time as the Map screen needs them, never in bulk.
#[tauri::command]
fn qth_for_call(state: State<AppState>, call: String) -> Option<String> {
    state.db.qth_for_call(&call)
}

/// Raw JPEG bytes for one offline basemap tile, or an empty body if `(z,x,y)`
/// isn't in the bundled archive (see `map_tiles.rs`). Returns a raw
/// `tauri::ipc::Response` rather than a JSON-wrapped/base64 payload: the IPC
/// bridge resolves that straight to an `ArrayBuffer` in JS, which
/// `createImageBitmap` decodes without ever navigating a URL -- smaller than
/// base64, no extra Cargo dependency, and it bypasses the page's `img-src`
/// CSP entirely (unlike a `data:`/`blob:` URL, which would need it).
#[tauri::command]
fn map_tile(state: State<AppState>, z: u32, x: u32, y: u32) -> tauri::ipc::Response {
    tauri::ipc::Response::new(state.map_tiles.tile_for(z, x, y).unwrap_or_default())
}

/// Fetches PSK Reporter spots for the Map screen's overlay. `mode` is always
/// a concrete string the frontend resolves itself (e.g. the live operating
/// mode, or an explicit filter override) -- there's no synchronous "ask the
/// engine what mode is live" read on this side the way Android's
/// `GeneralVariables.currentMode()` singleton allows. Internally
/// rate-limited/cooled-down exactly like Android's own client -- see
/// `psk_reporter.rs`'s doc comment.
#[tauri::command]
fn psk_reporter_spots(
    state: State<AppState>,
    call: String,
    seconds_back: i64,
    force: bool,
    mode: String,
    by_receiver: bool,
) -> PskFetchResult {
    state.psk_reporter.fetch_spots(&call, seconds_back, force, &mode, by_receiver)
}

/// Download the FCC amateur-license database, cross-reference it against the
/// bundled Census ZCTA Gazetteer, and seed `callsign_qth` -- see
/// `fcc_grid.rs`'s module doc comment for the full pipeline. Emits
/// `"fcc-grid-progress"` events (payload: [`fcc_grid::FccSeedProgress`])
/// throughout both the download and the processing phase, since either one
/// alone can take long enough that a silent wait would look like a hang (the
/// download is ~200MB; processing scans ~1.7M FCC records), then
/// `"fcc-grid-done"` once (payload: `{ok: FccSeedSummary} | {err: string}`)
/// so a listener never has to guess "has it finished?" from progress alone.
///
/// Called from two places: the `setup()` first-run auto-trigger (below,
/// which also drives native OS popups -- see its own doc comment for why)
/// and the `import_fcc_grid_seed` command (the future Settings "update now"
/// button). Both share this one function so there's exactly one seed
/// implementation and one event contract regardless of trigger source.
///
/// Wrapped in `catch_unwind` like `import_adif`: this also parses an
/// external file of content this app doesn't control.
fn run_fcc_grid_seed(db: &Db, app: &tauri::AppHandle) -> Result<FccSeedSummary, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fcc_grid::download_and_seed(db, &app_data_dir(), |progress| {
            let _ = app.emit("fcc-grid-progress", progress);
        })
    }))
    .map_err(|_| {
        "The US grid table download hit an unexpected error partway through and stopped; \
         any stations already added are kept."
            .to_string()
    })
    .and_then(|r| r.map_err(|e| e.to_string()));

    // Tuple variants + rename_all so this serializes as exactly
    // `{"ok": <summary>}` or `{"err": "<message>"}`, not double-nested.
    #[derive(Clone, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Done {
        Ok(FccSeedSummary),
        Err(String),
    }
    let _ = app.emit(
        "fcc-grid-done",
        match &result {
            Ok(summary) => Done::Ok(summary.clone()),
            Err(e) => Done::Err(e.clone()),
        },
    );
    result
}

#[tauri::command]
fn import_fcc_grid_seed(
    state: State<AppState>,
    app: tauri::AppHandle,
) -> Result<FccSeedSummary, String> {
    run_fcc_grid_seed(&state.db, &app)
}

#[tauri::command]
fn get_config(state: State<AppState>, key: String) -> Option<String> {
    state.db.get_config(&key)
}

#[tauri::command]
fn set_config(state: State<AppState>, key: String, value: String) -> Result<(), String> {
    state.db.set_config(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
fn all_config(state: State<AppState>) -> Vec<(String, String)> {
    state.db.all_config()
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: String,
    build_date: String,
}

/// Settings -> About's version/build-date row -- `CARGO_PKG_VERSION` (CI-
/// stamped from the release tag, see `ci/stamp-version.mjs`) and a build
/// timestamp `build.rs` embeds at compile time (see its own comment),
/// formatted here since `chrono` isn't a build-dependency.
/// Open a URL in the system's default browser/mail client, for Settings ->
/// About's website/community/author links and "Report a Bug"'s GitHub-issue/
/// email actions. A narrow `http(s)`/`mailto` scheme allowlist, not a
/// general-purpose "run anything" command -- the string is operator-visible
/// UI text (link targets this file defines or a mailto: this module builds),
/// not untrusted network input, but the allowlist costs nothing and rules
/// out e.g. a `file://` or shell-metacharacter surprise. Shells out to the
/// platform opener directly (`xdg-open`/`open`/`start`) rather than adding
/// `tauri-plugin-opener` as a dependency for three links.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")) {
        return Err("refusing to open a non-http(s)/mailto URL".into());
    }
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn()
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_app_info() -> AppInfo {
    let ts: i64 = env!("FT8AF_BUILD_TIMESTAMP").parse().unwrap_or(0);
    let build_date = chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    AppInfo { version: env!("CARGO_PKG_VERSION").to_string(), build_date }
}

// --- optional OS location -> grid (issue #471) ------------------------------

/// Resolve the operator grid from the OS location service. Opt-in only: refuses
/// unless the user has turned on the `location_service_enabled` flag, so nothing
/// requests location (and no permission prompt appears) until they explicitly
/// opt in. Returns only the coarse derived Maidenhead grid (the raw fix is
/// discarded inside the backend), or a human-readable error the UI shows without
/// touching the manual field.
#[tauri::command]
fn get_os_location(state: State<AppState>) -> Result<OsLocation, String> {
    let enabled = os_location::location_enabled(
        state
            .db
            .get_config(os_location::LOCATION_ENABLED_KEY)
            .as_deref(),
    );
    if !enabled {
        return Err(
            "Location service is off. Turn on \"Use OS location service\" in \
             Settings → Station first."
                .into(),
        );
    }
    os_location::resolve()
}

// --- LoTW (ARRL Logbook of the World) ---------------------------------------

#[tauri::command]
fn lotw_download_confirmations(state: State<AppState>, full_refresh: bool) -> LotwDownloadResult {
    let username = state.db.get_config(lotw::CONFIG_USERNAME).unwrap_or_default();
    let password = state.db.get_config(lotw::CONFIG_PASSWORD).unwrap_or_default();
    lotw::download_confirmations(&state.db, &username, &password, full_refresh)
}

#[tauri::command]
fn lotw_pending_upload_count(state: State<AppState>) -> i64 {
    state.db.qsos_pending_lotw_upload().len() as i64
}

/// `callsign`: the station callsign to sign with -- the caller passes the
/// live Station-settings callsign, not a separate config key (TQSL's own
/// cert/location selection already keys off `-c <callsign>`).
#[tauri::command]
fn lotw_upload_pending(state: State<AppState>, callsign: String) -> LotwUploadResult {
    lotw::upload_pending(&state.db, &callsign, &app_data_dir())
}

// --- QRZ.com -----------------------------------------------------------

#[tauri::command]
fn qrz_pending_upload_count(state: State<AppState>) -> i64 {
    state.db.qsos_pending_qrz_upload().len() as i64
}

#[tauri::command]
fn qrz_upload_pending(state: State<AppState>) -> QrzUploadResult {
    let api_key = state.db.get_config(qrz::CONFIG_API_KEY).unwrap_or_default();
    qrz::upload_pending(&state.db, &api_key)
}

/// Read-only Logbook API key check (`ACTION=STATUS`, inserts nothing).
#[tauri::command]
fn qrz_test_logbook_connection(state: State<AppState>) -> Result<(), String> {
    let api_key = state.db.get_config(qrz::CONFIG_API_KEY).unwrap_or_default();
    qrz::check_logbook_connection(&api_key)
}

/// Verifies the XML subscription credentials with a fresh auth call. Uses a
/// throwaway client -- this is a one-off check, not worth sharing the
/// engine's own prefetch cache/session for.
#[tauri::command]
fn qrz_test_xml_connection(state: State<AppState>) -> String {
    let username = state.db.get_config(qrz::CONFIG_XML_USERNAME).unwrap_or_default();
    let password = state.db.get_config(qrz::CONFIG_XML_PASSWORD).unwrap_or_default();
    qrz::QrzXmlClient::new().test_connection(&username, &password)
}

#[tauri::command]
fn set_waterfall_config(state: State<AppState>, config: WfConfig) {
    // The engine sanitizes, persists (wf_window/wf_fft_size/wf_avg/
    // wf_display_hz/wf_denoise), and rebuilds its FFT plan; see
    // EngineCommand::SetWaterfallConfig.
    state.engine.send(EngineCommand::SetWaterfallConfig(config));
}

/// "Width" -- merges into the engine's current waterfall config so the
/// operator doesn't need the full `WfConfig` to change just this one field.
/// See `EngineCommand::SetWfDisplayHz`.
#[tauri::command]
fn set_wf_display_hz(state: State<AppState>, hz: f32) {
    state.engine.send(EngineCommand::SetWfDisplayHz(hz));
}

/// "NR" -- same merge-into-current-config treatment as `set_wf_display_hz`.
/// See `EngineCommand::SetWfDenoise`.
#[tauri::command]
fn set_wf_denoise(state: State<AppState>, denoise: bool) {
    state.engine.send(EngineCommand::SetWfDenoise(denoise));
}

fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("FT8AF")
}

// The compiled-in baseline -- only this constant needs a rebuild to change;
// everything a user actually edits lives at styles_path() on disk instead.
const DEFAULT_STYLES_CSS: &str = include_str!("../../public/styles.css");

fn styles_path() -> PathBuf {
    app_data_dir().join("styles.css")
}

/// Marker line prefixed to the seeded copy of `DEFAULT_STYLES_CSS`. The value
/// is the hash of the body written underneath it, which does double duty: it
/// identifies *which* build's stylesheet the copy came from, and comparing it
/// against a re-hash of the body tells us whether the user has since edited
/// the file. A CSS comment, so a stale reader just treats it as one.
const STYLES_STAMP_PREFIX: &str = "/* ft8af-seeded: ";

/// FNV-1a (64-bit). Inlined rather than pulled in as a dependency, and chosen
/// over `DefaultHasher` because the value is written to disk and compared
/// across builds -- `DefaultHasher`'s output is explicitly not stable between
/// Rust releases, which would make every toolchain bump look like a new
/// stylesheet version.
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn stamped_default_css(default_css: &str) -> String {
    format!("{STYLES_STAMP_PREFIX}{:016x} */\n{default_css}", fnv1a64(default_css))
}

/// Split a stamped file into `(recorded hash, body)`, or `None` if it carries
/// no stamp (a hand-written file, or one seeded before stamping existed).
fn split_styles_stamp(content: &str) -> Option<(u64, &str)> {
    let rest = content.strip_prefix(STYLES_STAMP_PREFIX)?;
    let (hex, after) = rest.split_once(" */")?;
    let hash = u64::from_str_radix(hex.trim(), 16).ok()?;
    Some((hash, after.strip_prefix('\n').unwrap_or(after)))
}

/// What to do with whatever `styles_path()` currently holds. Pure so it can be
/// unit-tested without touching the filesystem.
#[derive(Debug, PartialEq, Eq)]
enum StylesAction {
    /// Serve this content as-is; leave the file alone.
    Use(String),
    /// (Re-)write the file from the compiled default and serve that.
    Seed,
}

/// Decide between the on-disk stylesheet and the compiled-in default.
///
/// The point of the on-disk copy is that editing it only needs an app
/// relaunch, not a rebuild -- so a file the user has actually edited always
/// wins, and an unstamped file is left alone too (we did not verifiably write
/// it, so we must not clobber it). The two cases we *do* take back are the
/// ones where the file is worse than useless:
///
///  * missing, empty, or whitespace-only -- an interrupted or truncated first
///    write leaves a 0-byte file that reads back `Ok("")`, which would inject
///    an empty `<style>` and render the whole app unstyled forever, with no
///    in-app way out;
///  * an untouched seed from an older build -- its stamp still matches its own
///    body, so we know the user never edited it, and serving it would freeze
///    the UI at that build's CSS while newer releases add markup it has no
///    rules for.
fn resolve_styles(on_disk: Option<&str>, default_css: &str) -> StylesAction {
    let Some(content) = on_disk else {
        return StylesAction::Seed;
    };
    if content.trim().is_empty() {
        return StylesAction::Seed;
    }
    match split_styles_stamp(content) {
        // Stamp matches the body: an untouched seed. Take it back if it came
        // from a different build than the one running now.
        Some((stamp, body)) if stamp == fnv1a64(body) => {
            if stamp == fnv1a64(default_css) {
                StylesAction::Use(body.to_string())
            } else {
                StylesAction::Seed
            }
        }
        // Stamped but edited, or not stamped at all -- the user's file.
        _ => StylesAction::Use(content.to_string()),
    }
}

#[tauri::command]
fn get_custom_css() -> String {
    // Read from disk on every call, not from the Vite/Tauri-bundled frontend
    // -- Tauri embeds frontendDist into the compiled binary at build time
    // (confirmed directly: editing the bundled dist/styles.css after a build
    // and relaunching the same binary had zero effect), so anything served
    // from there needs a full rebuild for every change. This file lives
    // outside that embed entirely, so editing it just needs an app relaunch
    // -- the whole point, since this stylesheet is expected to change often.
    let path = styles_path();
    let on_disk = std::fs::read_to_string(&path).ok();
    match resolve_styles(on_disk.as_deref(), DEFAULT_STYLES_CSS) {
        StylesAction::Use(css) => css,
        StylesAction::Seed => {
            let _ = std::fs::create_dir_all(app_data_dir());
            let _ = std::fs::write(&path, stamped_default_css(DEFAULT_STYLES_CSS));
            DEFAULT_STYLES_CSS.to_string()
        }
    }
}

/// Apply new WSJT-X UDP settings. The engine persists the `udp_*` config keys
/// and rebinds the socket + inbound listener live; see
/// `EngineCommand::SetUdpConfig`.
#[tauri::command]
fn set_udp_config(state: State<AppState>, config: ft8af::udp::UdpConfig) {
    state.engine.send(EngineCommand::SetUdpConfig(config));
}

fn main() {
    // Debug helper: `ft8af --list-rigs` prints the Hamlib-enumerated rig count
    // and exits — verifies the bundled Hamlib library loads without the GUI.
    if std::env::args().any(|a| a == "--list-rigs") {
        let rigs = rig::list_hamlib_rigs();
        println!("Hamlib enumerated {} rigs", rigs.len());
        for r in rigs.iter() {
            println!("  #{:<6} {}", r.model, r.name);
        }
        return;
    }

    // Debug helper: `ft8af --list-audio` prints cpal's enumerated input/output
    // devices and exits -- same idea as --list-rigs, verifies device
    // enumeration without needing to click through the GUI (native <select>
    // popups render as a separate top-level X window on Linux, so they don't
    // show up in a window-scoped screenshot either).
    if std::env::args().any(|a| a == "--list-audio") {
        let inputs = audio::list_input_devices();
        println!("Input devices ({}):", inputs.len());
        for d in inputs.iter() {
            println!(
                "  {}{} -- {} Hz, {} ch",
                if d.is_default { "* " } else { "  " },
                d.name,
                d.default_sample_rate,
                d.channels
            );
        }
        let outputs = audio::list_output_devices();
        println!("Output devices ({}):", outputs.len());
        for d in outputs.iter() {
            println!(
                "  {}{} -- {} Hz, {} ch",
                if d.is_default { "* " } else { "  " },
                d.name,
                d.default_sample_rate,
                d.channels
            );
        }
        return;
    }

    let data_dir = app_data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let db = Arc::new(
        Db::open(data_dir.join("ft8af.sqlite")).expect("failed to open database"),
    );
    let map_tiles = MapTiles::open(&data_dir).expect("failed to open map tile archive");
    let psk_reporter = PskReporterState::new();

    let (engine, evt_rx) = engine::spawn(db.clone());
    let db_for_setup = db.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { engine, db, map_tiles, psk_reporter })
        .setup(move |app| {
            // Forward engine events to the webview on the "engine-event" channel.
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("ft8af-event-forwarder".into())
                .spawn(move || {
                    while let Ok(ev) = evt_rx.recv() {
                        // Mirror status messages to the terminal. The app has no
                        // logger, so audio/rig/decode problems (e.g. "audio start
                        // failed") were previously invisible outside the in-app
                        // status line — making remote diagnosis impossible.
                        match &ev {
                            engine::EngineEvent::Error(m) => eprintln!("[ft8af] ERROR: {m}"),
                            engine::EngineEvent::Info(m) => eprintln!("[ft8af] {m}"),
                            _ => {}
                        }
                        let _ = handle.emit("engine-event", ev);
                    }
                })?;

            // First-run callsign/grid table seed, triggered here (backend)
            // rather than waited-on from the frontend's own mount effect, and
            // narrated through native OS dialogs rather than the main
            // window: a real, reproducible WebKitGTK bug on this hardware
            // means the main window's content can stop repainting shortly
            // after launch and not visibly update again (not the clock, not
            // the sync readout, nothing) until a real click forces a
            // repaint -- see docs/desktop-callsign-grid-table.md. A
            // brand-new user watching a ~200MB download + a 1.7M-record scan
            // happen behind a silently frozen window would reasonably
            // conclude the app had hung and kill it. Native dialogs (GTK
            // message boxes on Linux, via tauri-plugin-dialog) render
            // through a completely different pipeline than webview content,
            // so they stay visible and responsive regardless of that bug.
            // `fcc-grid-progress`/`fcc-grid-done` are still emitted
            // throughout (run_fcc_grid_seed, shared with the manual-trigger
            // command below) so the in-app dialog also works correctly
            // whenever the window *is* repainting.
            let db_for_seed = db_for_setup.clone();
            let handle_for_seed = app.handle().clone();
            std::thread::Builder::new()
                .name("ft8af-grid-seed".into())
                .spawn(move || {
                    if db_for_seed.callsign_qth_count() != 0 {
                        return;
                    }
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    handle_for_seed
                        .dialog()
                        .message(
                            "FT8AF is downloading the FCC amateur-license database and \
                             building a US callsign/grid table for the upcoming Map \
                             screen. This can take up to a minute on a slow connection. \
                             FT8AF keeps working normally while this runs in the \
                             background -- this message is just letting you know the \
                             app hasn't hung.",
                        )
                        .title("Creating US Grid Table")
                        .kind(MessageDialogKind::Info)
                        .blocking_show();

                    // No native completion/error popup here -- only the
                    // "it's working" one above needs to be native (that's
                    // the one standing in for a frozen main window). Once
                    // the operator has dismissed that, they're already
                    // engaged with the app; run_fcc_grid_seed's
                    // "fcc-grid-done" event drives the in-window completion/
                    // error dialog (App.tsx) same as it would for a manual
                    // re-run from Settings, so there's exactly one "it's
                    // done" notification, not two.
                    if let Err(e) = run_fcc_grid_seed(&db_for_seed, &handle_for_seed) {
                        eprintln!("[ft8af] ERROR: FCC grid seed failed: {e}");
                    }
                })?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_audio_inputs,
            list_audio_outputs,
            list_serial_ports,
            list_hamlib_rigs,
            list_bands,
            list_bands_for_mode,
            list_custom_bands,
            add_custom_band,
            delete_custom_band,
            start_decode,
            stop_decode,
            set_station,
            set_band,
            set_base_freq,
            set_tx_gain,
            set_rx_gain,
            set_input_device,
            set_output_device,
            set_rx_audio_channel,
            set_decode_depth,
            set_tx_audio_channel,
            set_qmx_cat_tones,
            select_rig,
            disconnect_rig,
            set_swr_halt_enabled,
            set_swr_halt_threshold,
            clear_swr_lockout,
            set_tune_max_on_seconds,
            set_tune_method,
            set_tune_level_independent,
            set_tune_level,
            set_tx_watchdog,
            set_max_73_sends,
            set_cq_watchdog,
            set_auto_cq_after_qso,
            set_clear_on_band_mode_change,
            set_rx_latency_ms,
            set_ptt_delay_ms,
            set_late_start_tolerance_ms,
            refresh_status,
            resync_time,
            set_clock_offset,
            start_cq,
            answer,
            set_stage,
            stop_tx,
            set_tx_period,
            set_mode,
            start_tune,
            stop_tune,
            free_text,
            list_log,
            prior_qsos,
            search_log,
            log_count,
            worked_calls,
            worked_calls_any_band,
            worked_grids,
            worked_prefixes,
            worked_dxcc_prefixes,
            worked_cq_zones,
            worked_states,
            log_stats,
            delete_qso,
            save_qso,
            export_adif,
            pick_adif_file,
            import_adif,
            export_settings_backup,
            pick_settings_backup_file,
            preview_settings_backup,
            apply_settings_backup,
            callsign_qth_count,
            qth_for_call,
            map_tile,
            psk_reporter_spots,
            import_fcc_grid_seed,
            get_config,
            set_config,
            all_config,
            get_app_info,
            open_external,
            set_waterfall_config,
            set_wf_display_hz,
            set_wf_denoise,
            get_custom_css,
            set_udp_config,
            get_os_location,
            lotw_download_confirmations,
            lotw_pending_upload_count,
            lotw_upload_pending,
            qrz_pending_upload_count,
            qrz_upload_pending,
            qrz_test_logbook_connection,
            qrz_test_xml_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error running FT8AF");
}

#[cfg(test)]
mod tests {
    use super::{
        fnv1a64, resolve_styles, split_styles_stamp, stamped_default_css, StylesAction,
        DEFAULT_STYLES_CSS,
    };

    const OLD: &str = "body { color: red; }\n";
    const NEW: &str = "body { color: blue; }\n";

    #[test]
    fn stamp_round_trips() {
        let seeded = stamped_default_css(OLD);
        assert_eq!(split_styles_stamp(&seeded), Some((fnv1a64(OLD), OLD)));
    }

    #[test]
    fn unstamped_content_has_no_stamp() {
        assert_eq!(split_styles_stamp(OLD), None);
        // A truncated or malformed marker must not be mistaken for a stamp.
        assert_eq!(split_styles_stamp("/* ft8af-seeded: zzzz */\nx"), None);
        assert_eq!(split_styles_stamp("/* ft8af-seeded: 00ff"), None);
    }

    #[test]
    fn missing_or_blank_file_is_seeded() {
        // No file at all: first run.
        assert_eq!(resolve_styles(None, NEW), StylesAction::Seed);
        // A 0-byte file from an interrupted first write reads back Ok("") --
        // serving that would inject an empty <style> and render nothing.
        assert_eq!(resolve_styles(Some(""), NEW), StylesAction::Seed);
        assert_eq!(resolve_styles(Some("  \n\t "), NEW), StylesAction::Seed);
    }

    #[test]
    fn untouched_seed_from_this_build_is_served_without_its_stamp() {
        let seeded = stamped_default_css(NEW);
        assert_eq!(
            resolve_styles(Some(&seeded), NEW),
            StylesAction::Use(NEW.to_string())
        );
    }

    #[test]
    fn untouched_seed_from_an_older_build_is_reseeded() {
        // The staleness case: the app shipped new CSS, the on-disk copy is
        // still the previous build's and the user never touched it.
        let seeded = stamped_default_css(OLD);
        assert_eq!(resolve_styles(Some(&seeded), NEW), StylesAction::Seed);
    }

    #[test]
    fn user_edits_survive_a_default_change() {
        let edited = format!("{}body {{ color: green; }}\n", stamped_default_css(OLD));
        assert_eq!(
            resolve_styles(Some(&edited), NEW),
            StylesAction::Use(edited.clone())
        );
    }

    #[test]
    fn unstamped_file_is_never_clobbered() {
        // Hand-written, or seeded by a build from before stamping existed.
        assert_eq!(
            resolve_styles(Some(OLD), NEW),
            StylesAction::Use(OLD.to_string())
        );
    }

    #[test]
    fn shipped_default_seeds_and_round_trips() {
        // Guards the real constant, not just fixtures: seeding then resolving
        // must hand back the compiled stylesheet byte-for-byte.
        let seeded = stamped_default_css(DEFAULT_STYLES_CSS);
        assert_eq!(
            resolve_styles(Some(&seeded), DEFAULT_STYLES_CSS),
            StylesAction::Use(DEFAULT_STYLES_CSS.to_string())
        );
    }

    #[test]
    fn fnv1a64_matches_reference_vectors() {
        // Pinned so a refactor can't silently change the on-disk stamp format.
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64("foobar"), 0x8594_4171_f739_67e8);
    }
}
