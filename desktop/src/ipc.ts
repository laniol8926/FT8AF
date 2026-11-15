// Typed wrappers over the Tauri command/event boundary. The Rust side defines
// the matching commands (src-tauri/src/main.rs) and emits a single tagged
// "engine-event" stream (src-tauri/src/engine.rs::EngineEvent).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface UiMessage {
  utc_ms: number;
  call_to: string;
  call_from: string;
  extra: string;
  grid: string;
  snr: number;
  freq_hz: number;
  time_sec: number;
  text: string;
  is_cq: boolean;
  to_me: boolean;
  /** The sender's DXCC entity prefix, or null for an unresolvable callsign -- see dxcc.rs. */
  dxcc_prefix: string | null;
  /** The sender's CQ zone, alongside dxcc_prefix. */
  cq_zone: number | null;
  /** The sender's US state, derived from its grid (grid-only, no callsign fallback -- see us_states.rs). */
  state: string | null;
}

export interface CycleTick {
  utc_ms: number;
  sequential: number;
  ms_into_cycle: number;
}

export type TxStage =
  | "idle" | "cq" | "grid" | "report" | "r_report" | "rr73" | "bye73";

/** Operating mode -- matches `crate::mode::Mode`'s `#[serde(rename_all = "lowercase")]`. */
export type OperatingMode = "ft8" | "ft4" | "ft2";

/** RX input-level health -- matches `crate::engine::InputLevelStatus` and
 *  Android's `AudioInputLevel.Status`. */
export type InputLevelStatus = "silent" | "low" | "good" | "high" | "clipping";

export interface InputLevelEvent {
  /** RMS level of the recent capture in dBFS (≈ -100 for digital silence). */
  db: number;
  /** Peak sample magnitude, linear (0..~1) -- feeds the meter's peak tick and percent readout. */
  peak: number;
  /** RMS sample magnitude, linear (0..~1) -- feeds the meter's bar fill. */
  rms: number;
  status: InputLevelStatus;
}

export interface QsoStatus {
  active: boolean;
  stage: TxStage;
  target: string | null;
  tx_message: string | null;
  report_sent: number | null;
  report_rcvd: number | null;
}

export interface TxStateEvent {
  transmitting: boolean;
  message: string | null;
  status: QsoStatus;
  /** A TUNE (steady carrier, no FT8 message) is in flight — drives the Tune button's on/off look. */
  tuning: boolean;
  /** Operator's preferred TX slot parity for the next CQ/QSO start (0 = Even,
   *  1 = Odd, null = no preference) — see `set_tx_period`. */
  preferred_tx_parity: number | null;
  /** SWR Halt's lockout -- see `clear_swr_lockout`. True blocks starting any
   *  new TX/Tune until explicitly cleared. */
  swr_lockout: boolean;
}

export interface RigStatusEvent {
  connected: boolean;
  model: string;
  frequency_hz: number;
  ptt: boolean;
  /** TX-audio-channel/CAT-tones values actually in effect right now (after
   *  auto-detect), not the raw config -- use these to display current state,
   *  the raw `txAudioChannel`/`qmxCatTones` config only reflects an explicit
   *  override. */
  tx_via_cat_tones: boolean;
  tx_audio_channel: number;
}

export interface ClockSyncEvent {
  /** ms added to the local clock to reach true UTC (negative = local clock is fast). */
  offset_ms: number;
  /** true once an NTP sync has succeeded (or a saved offset was restored). */
  synced: boolean;
  /** Compatibility field; fixed at zero since unsafe RX auto-calibration was disabled. */
  rx_offset_ms: number;
}

export interface QsoRecord {
  id?: number | null;
  call: string;
  gridsquare: string;
  mode: string;
  rst_sent: string;
  rst_rcvd: string;
  qso_date: string;
  time_on: string;
  qso_date_off: string;
  time_off: string;
  band: string;
  freq: string;
  station_callsign: string;
  my_gridsquare: string;
  comment: string;
  /** Paper QSL card received (drives the Logbook filter + ADIF QSL_RCVD). */
  confirmed: boolean;
  /** Paper QSL card sent (ADIF QSL_SENT). */
  qsl_sent: boolean;
  /** LoTW confirmation sent (ADIF LOTW_QSL_SENT). */
  lotw_sent: boolean;
  /** LoTW confirmation received (ADIF LOTW_QSL_RCVD). */
  lotw_rcvd: boolean;
  /** eQSL sent (ADIF EQSL_QSL_SENT). */
  eqsl_sent: boolean;
  /** eQSL received (ADIF EQSL_QSL_RCVD). */
  eqsl_rcvd: boolean;
  /** Confirmed on QRZ.com. */
  qrz_qsl: boolean;
  /** Logged US state, or another country's subdivision (ADIF STATE). Not
   *  derived from the grid -- only ever what was explicitly logged/imported. */
  state: string;
  /** Logged country (ADIF COUNTRY). */
  country: string;
  /** Already uploaded to Cloudlog/Wavelog/Nextlog. Read-only: set by syncing,
   *  not by editing or importing a QSO -- optional here only so a record built
   *  for `saveQso` doesn't have to carry it through; always present when read
   *  back from the backend. */
  synced_cloudlog?: boolean;
  /** Already uploaded to QRZ.com's logbook. Read-only: see synced_cloudlog. */
  synced_qrz?: boolean;
  /** Transmit power in watts at QSO completion (ADIF TX_PWR) -- the
   *  operator's `powerWatts` Station setting at that moment, same free-text
   *  field as Android's own QSLTable.tx_pwr. Optional here for the same
   *  reason as synced_cloudlog/synced_qrz: always present when read back. */
  tx_pwr?: string;
}

/** Result of an ADIF import: see the Rust `Db::import_adif` doc comment for
 *  the duplicate-matching rule (call + date + mode, TIME_ON to the minute,
 *  band not compared; a match only ever has its QSL_RCVD flag turned on). */
export interface ImportSummary {
  imported: number;
  duplicates: number;
  skipped: number;
}

/** Live progress for an in-flight ADIF import, emitted on `"import-progress"`. */
export interface ImportProgress {
  processed: number;
  total: number;
}

export function onImportProgress(cb: (p: ImportProgress) => void): Promise<UnlistenFn> {
  return listen<ImportProgress>("import-progress", (ev) => cb(ev.payload));
}

/** The furthest logged station and its great-circle distance from the
 *  operator -- see Rust `db.rs`'s `BestDx`/`Db::best_dx`. */
export interface BestDx {
  callsign: string;
  grid: string;
  distance_km: number;
}

/** Aggregate Logbook stats for the Stats/Awards tabs, matching Android's
 *  `LogbookStats` data class field-for-field -- see Rust `db.rs`'s
 *  `LogStats`/`Db::log_stats`. `band_counts` serializes as an array of
 *  [band, count] pairs (Rust's `Vec<(String, i64)>`), most-worked first. */
export interface LogStats {
  total_qsos: number;
  dxcc_entities: number;
  cq_zones: number;
  itu_zones: number;
  grid_squares: number;
  band_counts: [string, number][];
  continent_codes: string[];
  states_worked: string[];
  worked_grid_fields: string[];
  best_dx: BestDx | null;
}

/** One PSK Reporter reception report, already resolved to "the other
 *  station" (not necessarily the sender -- see `psk_reporter.rs`'s
 *  `parse_spots` doc comment for the sender/receiver direction logic). */
export interface PskReporterSpot {
  callsign: string;
  grid: string;
  band: string;
  frequency_hz: number;
  snr: number;
  mode: string;
  flow_start_seconds: number;
}

export interface PskFetchResult {
  spots: PskReporterSpot[] | null;
  error: string | null;
}

/** First-run FCC/Census grid-table seed: see Rust `fcc_grid.rs`'s module doc
 *  comment. "downloading" is the ~200MB FCC amateur-license zip (`total` is
 *  bytes, from Content-Length); "processing" is the per-record FCC→Gazetteer
 *  cross-reference + DB insert (`total` is the active-license count). */
export type FccSeedPhase = "downloading" | "processing";

export interface FccSeedProgress {
  phase: FccSeedPhase;
  processed: number;
  total: number;
  /** The callsign just processed -- empty during "downloading" and on the
   *  final completion tick. */
  current_callsign: string;
}

export interface FccSeedSummary {
  seeded: number;
  active_licenses: number;
}

export function onFccGridProgress(cb: (p: FccSeedProgress) => void): Promise<UnlistenFn> {
  return listen<FccSeedProgress>("fcc-grid-progress", (ev) => cb(ev.payload));
}

/** Fires once, after "fcc-grid-progress" stops, whichever triggered the
 *  seed (the backend's first-run auto-start or a manual re-run) -- see
 *  Rust `run_fcc_grid_seed`'s doc comment. */
export type FccSeedDone = { ok: FccSeedSummary } | { err: string };

export function onFccGridDone(cb: (d: FccSeedDone) => void): Promise<UnlistenFn> {
  return listen<FccSeedDone>("fcc-grid-done", (ev) => cb(ev.payload));
}

export interface AudioDevice {
  id: string;
  name: string;
  is_default: boolean;
  default_sample_rate: number;
  channels: number;
}

export interface SerialPortInfo {
  name: string;
  kind: string;
}

export interface HamlibRig {
  model: number;
  name: string;
}

export interface BandInfo {
  name: string;
  dial_hz: number;
  /** True for operator-defined dials (issue #470), false for the built-in plan. */
  custom: boolean;
}

/** A user-defined dial frequency (issue #470). */
export interface CustomBand {
  name: string;
  dial_hz: number;
}

/** Result of resolving the operator grid from the OS location service (issue #471).
 *  Only the coarse grid is returned — the backend derives it from the raw fix and
 *  discards the precise coordinates, so no pinpoint location crosses the IPC. */
export interface OsLocation {
  /** Derived Maidenhead grid (6-char subsquare). */
  grid: string;
}

/** Settings -> About's version/build-date row. */
export interface AppInfo {
  version: string;
  build_date: string;
}

/** Settings -> Advanced's parsed backup file, previewed before import. */
export interface ParsedSettingsBackup {
  format_version: number;
  app_version: string;
  created_at: string;
  config: Record<string, string>;
  follow_callsigns: string[];
}

/** Waterfall FFT window (developer setting, issue #428). */
export type WfWindow = "rect" | "hann" | "hamming" | "blackman" | "blackman_harris";

/** Live-waterfall FFT parameters. Defaults: hann / 2048 / 6 / 3500. */
export interface WaterfallConfig {
  window: WfWindow;
  fft_size: number;
  avg: number;
  /** Displayed span of the audio band in Hz ("Width") -- clamped to
   *  [WF_MIN_DISPLAY_HZ, WF_MAX_DISPLAY_HZ] = [2500, 5000] server-side,
   *  matching Android's spectrumWidth range exactly. */
  display_hz: number;
  /** "NR" -- display-only band-tilt flattening, matching Android's deNoise.
   *  Not edited from the Settings panel (Android doesn't duplicate NR
   *  there either -- only Width), but must round-trip through here so a
   *  Settings-panel edit to window/fft_size/avg doesn't silently reset it. */
  denoise: boolean;
}

export const WF_MIN_DISPLAY_HZ = 2_500;
export const WF_MAX_DISPLAY_HZ = 5_000;
export const WF_DEFAULT_DISPLAY_HZ = 3_500;

export type RigBackend = "none" | "flrig" | "hamlib" | "g106";

/** How PTT is keyed for the Hamlib backend -- see `rig.rs`'s `RigControlMode`. */
export type RigControlMode = "vox" | "cat" | "rts" | "dtr";

export interface RigConfig {
  backend: RigBackend;
  port: string;
  baud: number;
  control_mode: RigControlMode;
  flrig_host: string;
  flrig_port: number;
  hamlib_model: number;
  hamlib_network: boolean;
  hamlib_address: string;
}

export type EngineEvent =
  | { type: "decoded"; data: UiMessage[] }
  | { type: "cycle"; data: CycleTick }
  | { type: "tx_state"; data: TxStateEvent }
  | { type: "rig_status"; data: RigStatusEvent }
  | { type: "clock_sync"; data: ClockSyncEvent }
  | { type: "qso_completed"; data: QsoRecord }
  | { type: "waterfall"; data: { bins: number[]; rows: number; cols: number; hz_per_col: number } }
  | { type: "waterfall_row"; data: { bins: number[]; hz_per_col: number; boundary: boolean } }
  | { type: "input_level"; data: InputLevelEvent }
  /** SWR/output-power readings polled over CAT while transmitting (QMX only
   *  -- see rig.rs's supports_meter_query). Either field is null when that
   *  particular query didn't return a valid reading. */
  | { type: "meter"; data: { swr_ratio: number | null; power_watts: number | null } }
  /** Settings -> Transmission's "Clear decodes on band/mode change": wipe the
   *  decode-row list, same as the manual "Clear the decode list" button. */
  | { type: "clear_decode_list" } // unit variant -- no "data" key at all
  | { type: "info"; data: string }
  | { type: "error"; data: string };

export function onEngineEvent(cb: (e: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>("engine-event", (ev) => cb(ev.payload));
}

// --- command wrappers -------------------------------------------------------

export const api = {
  listAudioInputs: () => invoke<AudioDevice[]>("list_audio_inputs"),
  listAudioOutputs: () => invoke<AudioDevice[]>("list_audio_outputs"),
  listSerialPorts: () => invoke<SerialPortInfo[]>("list_serial_ports"),
  listHamlibRigs: () => invoke<HamlibRig[]>("list_hamlib_rigs"),
  listBands: () => invoke<BandInfo[]>("list_bands"),
  listBandsForMode: (mode: OperatingMode) => invoke<BandInfo[]>("list_bands_for_mode", { mode }),
  listCustomBands: () => invoke<CustomBand[]>("list_custom_bands"),
  /** Add/rename a custom dial. Rejects bad input with a message (Err string). */
  addCustomBand: (freq: string, name: string) =>
    invoke<BandInfo[]>("add_custom_band", { freq, name }),
  deleteCustomBand: (dialHz: number) =>
    invoke<BandInfo[]>("delete_custom_band", { dialHz }),

  startDecode: () => invoke("start_decode"),
  stopDecode: () => invoke("stop_decode"),
  setStation: (call: string, grid: string) => invoke("set_station", { call, grid }),
  setBand: (dialHz: number) => invoke("set_band", { dialHz }),
  setBaseFreq: (hz: number) => invoke("set_base_freq", { hz }),
  setTxGain: (gain: number) => invoke("set_tx_gain", { gain }),
  setRxGain: (gain: number) => invoke("set_rx_gain", { gain }),
  setInputDevice: (name: string | null) => invoke("set_input_device", { name }),
  setOutputDevice: (name: string | null) => invoke("set_output_device", { name }),
  /** RX/TX audio channel selection (0=Mix/Both, 1=Left, 2=Right) -- see
   *  src-tauri/src/audio/channel_select.rs. */
  setRxAudioChannel: (channel: number) => invoke("set_rx_audio_channel", { channel }),
  setTxAudioChannel: (channel: number) => invoke("set_tx_audio_channel", { channel }),
  setQmxCatTones: (enabled: boolean) => invoke("set_qmx_cat_tones", { enabled }),
  selectRig: (config: RigConfig) => invoke("select_rig", { config }),
  disconnectRig: () => invoke("disconnect_rig"),
  /** Settings -> Transmission -> TX Protection. SWR threshold is a plain
   *  ratio (e.g. 3.0), not Android's normalized 0-255 scale. */
  setSwrHaltEnabled: (enabled: boolean) => invoke("set_swr_halt_enabled", { enabled }),
  setSwrHaltThreshold: (threshold: number) => invoke("set_swr_halt_threshold", { threshold }),
  clearSwrLockout: () => invoke("clear_swr_lockout"),
  /** Settings -> Transmission -> Tune. */
  setTuneMaxOnSeconds: (seconds: number) => invoke("set_tune_max_on_seconds", { seconds }),
  setTuneMethod: (method: number) => invoke("set_tune_method", { method }),
  setTuneLevelIndependent: (independent: boolean) => invoke("set_tune_level_independent", { independent }),
  setTuneLevel: (gain: number) => invoke("set_tune_level", { gain }),
  /** Settings -> Transmission's "TX Watchdog" / "Max 73 Sends" (0 = unlimited/Auto). */
  setTxWatchdog: (limit: number) => invoke("set_tx_watchdog", { limit }),
  setMax73Sends: (cap: number) => invoke("set_max_73_sends", { cap }),
  /** Settings -> Auto-Sequence's "Auto-CQ after QSO". */
  setAutoCqAfterQso: (enabled: boolean) => invoke("set_auto_cq_after_qso", { enabled }),
  /** Settings -> Transmission's "Clear decodes on band/mode change". */
  setClearOnBandModeChange: (enabled: boolean) => invoke("set_clear_on_band_mode_change", { enabled }),
  /** RX audio capture latency compensation (ms) -- Settings -> Time Sync's
   *  "RX Audio Latency" (a manual stand-in for Android's own auto-calibrated
   *  value; see TimeSyncSettings.tsx's doc comment). */
  setRxLatencyMs: (ms: number) => invoke("set_rx_latency_ms", { ms }),
  refreshStatus: () => invoke("refresh_status"),
  resyncTime: () => invoke("resync_time"),
  /** Manual clock-correction stepper for offline operating (Settings -> Time Sync). */
  setClockOffset: (offsetMs: number) => invoke("set_clock_offset", { offsetMs }),

  startCq: () => invoke("start_cq"),
  answer: (args: { call_from: string; grid: string; snr: number }) =>
    invoke("answer", { args }),
  setStage: (stage: TxStage) => invoke("set_stage", { stage }),
  stopTx: () => invoke("stop_tx"),
  setTxPeriod: (parity: number | null) => invoke("set_tx_period", { parity }),
  setMode: (mode: OperatingMode) => invoke("set_mode", { mode }),
  freeText: (text: string) => invoke("free_text", { text }),
  startTune: () => invoke("start_tune"),
  stopTune: () => invoke("stop_tune"),

  listLog: (limit: number, offset: number) =>
    invoke<QsoRecord[]>("list_log", { limit, offset }),
  /** Server-side callsign search + confirmation filter + column sort
   *  (sortColumn is one of logbook.ts's QSO_COL_* indices). Sorting happens
   *  in SQL before `limit` truncates the result -- see db.rs's
   *  `Db::search_qsos` doc comment for why that matters once the log
   *  exceeds `limit`: a client-side sort of only the fetched page can never
   *  surface a row outside it. */
  searchLog: (
    callsign: string,
    filter: "all" | "confirmed" | "unconfirmed",
    sortColumn: number,
    sortAscending: boolean,
    limit: number,
    offset: number,
  ) => invoke<QsoRecord[]>("search_log", { callsign, filter, sortColumn, sortAscending, limit, offset }),
  logCount: () => invoke<number>("log_count"),
  /** Calls already worked on the currently-tuned band, for Decode-list strikethrough. */
  workedCalls: (mode: string) => invoke<string[]>("worked_calls", { mode }),
  /** Calls already worked on mode, any band -- for the New Band status. */
  workedCallsAnyBand: (mode: string) => invoke<string[]>("worked_calls_any_band", { mode }),
  /** Grid squares already worked (any band/mode), for Hunt's new-grid priority
   *  and the New Grid filter chip. */
  workedGrids: () => invoke<string[]>("worked_grids"),
  /** CQ WPX prefixes already worked (any band/mode), for the New Prefix filter chip. */
  workedPrefixes: () => invoke<string[]>("worked_prefixes"),
  /** DXCC entity prefixes already worked (any band/mode), for the New DXCC filter chip. */
  workedDxccPrefixes: () => invoke<string[]>("worked_dxcc_prefixes"),
  /** CQ zones already worked (any band/mode), for the New Zone filter chip. */
  workedCqZones: () => invoke<number[]>("worked_cq_zones"),
  /** US states already worked on the currently-tuned band/mode, for the New State filter chip. */
  workedStates: (mode: string) => invoke<string[]>("worked_states", { mode }),
  /** Aggregate Logbook stats for the Stats/Awards tabs; myGrid drives the Best
   *  DX card (an empty/unparseable grid just means no card, not an error). */
  logStats: (myGrid: string) => invoke<LogStats>("log_stats", { myGrid }),
  deleteQso: (id: number) => invoke("delete_qso", { id }),
  saveQso: (record: QsoRecord) => invoke<number>("save_qso", { record }),
  /** Export ADIF, optionally restricted to an inclusive YYYYMMDD date range. */
  exportAdif: (start?: string, end?: string) =>
    invoke<string>("export_adif", { start: start ?? null, end: end ?? null }),
  /** Import ADIF text (already read client-side from the chosen file). */
  /** Native "pick an ADIF file" dialog; null on Cancel. */
  pickAdifFile: () => invoke<string | null>("pick_adif_file"),
  /** Import the ADIF file at `path` -- the file is read on the Rust side,
   *  never loaded into JS/sent over IPC (an earlier version did that and
   *  could buffer a large real log several times over in memory). */
  importAdif: (path: string) => invoke<ImportSummary>("import_adif", { path }),

  /** Settings -> Advanced's backup/export/import. */
  exportSettingsBackup: (includeSensitive: boolean) =>
    invoke<string>("export_settings_backup", { includeSensitive }),
  pickSettingsBackupFile: () => invoke<string | null>("pick_settings_backup_file"),
  previewSettingsBackup: (path: string) => invoke<ParsedSettingsBackup>("preview_settings_backup", { path }),
  applySettingsBackup: (config: Record<string, string>, followCallsigns: string[]) =>
    invoke<void>("apply_settings_backup", { config, followCallsigns }),
  /** Settings -> Advanced's "Decode Depth" toggle. */
  setDecodeDepth: (deep: boolean) => invoke("set_decode_depth", { deep }),

  /** Rows in the callsign/grid cache -- 0 means the FCC/Census seed hasn't run yet. */
  callsignQthCount: () => invoke<number>("callsign_qth_count"),
  /** The cached grid for one callsign, if any -- the Map screen's per-message
   *  lookup for decodes that don't carry a grid themselves (TX3-5/73/R73).
   *  Looked up one callsign at a time, never in bulk. */
  qthForCall: (call: string) => invoke<string | null>("qth_for_call", { call }),
  /** Raw JPEG bytes for one offline basemap tile, or a zero-length
   *  ArrayBuffer if `(z,x,y)` isn't in the bundled archive (map_tiles.rs).
   *  Returned as raw IPC bytes, not JSON/base64 -- see main.rs's `map_tile`
   *  doc comment for why. */
  mapTile: (z: number, x: number, y: number) => invoke<ArrayBuffer>("map_tile", { z, x, y }),
  /** PSK Reporter spots for the Map screen's overlay (psk_reporter.rs).
   *  `mode` must be a concrete string ("FT8"/"FT4"/"FT2") -- see
   *  pskFilter.ts's `resolvePskModeString`. A `null` `spots` field means
   *  the fetch was skipped (cooldown/rate-limit) or failed; callers should
   *  keep whatever spots they already have, not clear them. */
  pskReporterSpots: (call: string, secondsBack: number, force: boolean, mode: string, byReceiver: boolean) =>
    invoke<PskFetchResult>("psk_reporter_spots", {
      call,
      secondsBack,
      force,
      mode,
      byReceiver,
    }),
  /** Download + cross-reference the FCC/Census data, seeding the callsign/grid
   *  cache. Long-running (~200MB download + ~1.7M-record scan) -- pair with
   *  `onFccGridProgress` for a live counter. */
  importFccGridSeed: () => invoke<FccSeedSummary>("import_fcc_grid_seed"),

  getConfig: (key: string) => invoke<string | null>("get_config", { key }),
  setConfig: (key: string, value: string) => invoke("set_config", { key, value }),
  allConfig: () => invoke<[string, string][]>("all_config"),

  /** Settings -> About's version/build-date row. */
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  /** Open a URL in the system default browser/mail client (http(s)/mailto only). */
  openExternal: (url: string) => invoke<void>("open_external", { url }),

  /** Resolve the operator grid from the OS location service (opt-in, issue #471).
   * Rejects unless `location_service_enabled` is on. */
  getOsLocation: () => invoke<OsLocation>("get_os_location"),

  /** Apply + persist live-waterfall FFT parameters (Rust side persists). */
  setWaterfallConfig: (config: WaterfallConfig) =>
    invoke("set_waterfall_config", { config }),
  /** "Width" -- merges into the engine's current waterfall config. */
  setWfDisplayHz: (hz: number) => invoke("set_wf_display_hz", { hz }),
  /** "NR" -- merges into the engine's current waterfall config. */
  setWfDenoise: (denoise: boolean) => invoke("set_wf_denoise", { denoise }),

  /** Apply + persist WSJT-X UDP settings (Rust side persists the udp_* keys and
   * rebinds the socket/listener live). */
  setUdpConfig: (config: UdpConfig) => invoke("set_udp_config", { config }),
};

/** WSJT-X UDP settings. Field names match the Rust `UdpConfig` (snake_case). */
export type UdpConfig = {
  enabled: boolean;
  host: string;
  port: number;
  accept_requests: boolean;
};
