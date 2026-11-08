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
}

export interface CycleTick {
  utc_ms: number;
  sequential: number;
  ms_into_cycle: number;
}

export type TxStage =
  | "idle" | "cq" | "grid" | "report" | "r_report" | "rr73" | "bye73";

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
}

export interface RigStatusEvent {
  connected: boolean;
  model: string;
  frequency_hz: number;
  ptt: boolean;
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

/** Waterfall FFT window (developer setting, issue #428). */
export type WfWindow = "rect" | "hann" | "hamming" | "blackman" | "blackman_harris";

/** Live-waterfall FFT parameters. Defaults: hann / 2048 / 6. */
export interface WaterfallConfig {
  window: WfWindow;
  fft_size: number;
  avg: number;
}

export type RigBackend = "none" | "flrig" | "hamlib" | "g106";

export interface RigConfig {
  backend: RigBackend;
  port: string;
  baud: number;
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
  | { type: "input_level"; data: { db: number; silent: boolean } }
  /** SWR/output-power readings polled over CAT while transmitting (QMX only
   *  -- see rig.rs's supports_meter_query). Either field is null when that
   *  particular query didn't return a valid reading. */
  | { type: "meter"; data: { swr_ratio: number | null; power_watts: number | null } }
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
  selectRig: (config: RigConfig) => invoke("select_rig", { config }),
  disconnectRig: () => invoke("disconnect_rig"),
  refreshStatus: () => invoke("refresh_status"),
  resyncTime: () => invoke("resync_time"),

  startCq: () => invoke("start_cq"),
  answer: (args: { call_from: string; grid: string; snr: number }) =>
    invoke("answer", { args }),
  setStage: (stage: TxStage) => invoke("set_stage", { stage }),
  stopTx: () => invoke("stop_tx"),
  freeText: (text: string) => invoke("free_text", { text }),
  startTune: () => invoke("start_tune"),
  stopTune: () => invoke("stop_tune"),

  listLog: (limit: number, offset: number) =>
    invoke<QsoRecord[]>("list_log", { limit, offset }),
  /** Server-side callsign search + confirmation filter. */
  searchLog: (
    callsign: string,
    filter: "all" | "confirmed" | "unconfirmed",
    limit: number,
    offset: number,
  ) => invoke<QsoRecord[]>("search_log", { callsign, filter, limit, offset }),
  logCount: () => invoke<number>("log_count"),
  /** Calls already worked on the currently-tuned band, for Decode-list strikethrough. */
  workedCalls: (mode: string) => invoke<string[]>("worked_calls", { mode }),
  /** Grid squares already worked (any band/mode), for Hunt's new-grid priority
   *  and the New Grid filter chip. */
  workedGrids: () => invoke<string[]>("worked_grids"),
  /** CQ WPX prefixes already worked (any band/mode), for the New Prefix filter chip. */
  workedPrefixes: () => invoke<string[]>("worked_prefixes"),
  deleteQso: (id: number) => invoke("delete_qso", { id }),
  saveQso: (record: QsoRecord) => invoke<number>("save_qso", { record }),
  /** Export ADIF, optionally restricted to an inclusive YYYYMMDD date range. */
  exportAdif: (start?: string, end?: string) =>
    invoke<string>("export_adif", { start: start ?? null, end: end ?? null }),
  /** Import ADIF text (already read client-side from the chosen file). */
  importAdif: (adif: string) => invoke<ImportSummary>("import_adif", { adif }),

  getConfig: (key: string) => invoke<string | null>("get_config", { key }),
  setConfig: (key: string, value: string) => invoke("set_config", { key, value }),
  allConfig: () => invoke<[string, string][]>("all_config"),

  /** Resolve the operator grid from the OS location service (opt-in, issue #471).
   * Rejects unless `location_service_enabled` is on. */
  getOsLocation: () => invoke<OsLocation>("get_os_location"),

  /** Apply + persist live-waterfall FFT parameters (Rust side persists). */
  setWaterfallConfig: (config: WaterfallConfig) =>
    invoke("set_waterfall_config", { config }),

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
