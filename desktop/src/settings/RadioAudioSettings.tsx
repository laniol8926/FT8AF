import { useEffect, useState } from "react";
import { api } from "../ipc";
import type {
  AudioDevice,
  BandInfo,
  CustomBand,
  HamlibRig,
  OperatingMode,
  RigConfig,
  RigControlMode,
  RigStatusEvent,
  SerialPortInfo,
  UdpConfig,
  WaterfallConfig,
  WfWindow,
} from "../ipc";
import { applySelection, configToSelection, rigName, rigOptions } from "../rig";
import { filterEnabledBands, parseExcludedBands, serializeExcludedBands } from "../bandFilter";
import { audioDeviceLabel } from "../audioDevice";
import { BandModeSheet } from "../bandModePicker";
import { ListPickerDialog, NumberInputDialog, SettingsRow, SettingsSegmentedRow } from "./SettingsPrimitives";

const WF_WINDOWS: WfWindow[] = ["rect", "hann", "hamming", "blackman", "blackman_harris"];
const WF_WINDOW_LABELS: Record<WfWindow, string> = {
  rect: "Rectangular (none)",
  hann: "Hann (default)",
  hamming: "Hamming",
  blackman: "Blackman",
  blackman_harris: "Blackman-Harris",
};
const WF_FFT_SIZES = [512, 1024, 2048, 4096, 8192];
const WF_AVG_OPTIONS = [1, 2, 3, 4, 6, 8, 12, 16];
const BAUD_RATES = [4800, 9600, 19200, 38400, 57600, 115200];

/** Mix/Both, Left, Right -- matches `audio::channel_select` exactly. */
const RX_CHANNEL_OPTIONS = [
  { value: 0, label: "Mix" },
  { value: 1, label: "Left" },
  { value: 2, label: "Right" },
];
const TX_CHANNEL_OPTIONS = [
  { value: 0, label: "Both" },
  { value: 1, label: "Left" },
  { value: 2, label: "Right" },
];

/** VOX/CAT/RTS/DTR -- matches `rig.rs`'s `RigControlMode` exactly (ported
 *  from Android's own `ControlMode`; its Bluetooth/Network variants have no
 *  desktop equivalent). */
const CONTROL_MODE_OPTIONS: { value: RigControlMode; label: string }[] = [
  { value: "vox", label: "VOX" },
  { value: "cat", label: "CAT" },
  { value: "rts", label: "RTS" },
  { value: "dtr", label: "DTR" },
];
const CONTROL_MODE_LABEL: Record<RigControlMode, string> = { vox: "VOX", cat: "CAT", rts: "RTS", dtr: "DTR" };

/**
 * Defaults for every `RigConfig` field, reused both as the initial
 * `useState` value and to backfill a persisted `rig_config` blob on load
 * (see that `useEffect`'s own comment). Rust's own deserialization already
 * has `#[serde(default = ...)]` per field for exactly this reason -- a
 * blob saved by an older build (or one hand-edited/corrupted) shouldn't
 * blank out a field it predates -- but that only protects the *Rust* side
 * (`serde_json::from_str`); the frontend's own `JSON.parse(v) as RigConfig`
 * is an unchecked cast with no such fallback, so it needs its own copy of
 * the same defaults.
 */
const DEFAULT_RIG_CONFIG: RigConfig = {
  backend: "none",
  port: "",
  baud: 38400,
  control_mode: "cat",
  flrig_host: "127.0.0.1",
  flrig_port: 12345,
  hamlib_model: 1,
  hamlib_network: false,
  hamlib_address: "127.0.0.1:4992",
};

type OpenDialog =
  | null
  | "rigModel"
  | "controlMode"
  | "connectionMode"
  | "serialPort"
  | "baudRate"
  | "bandFrequency"
  | "enabledBands"
  | "audioFrequency"
  | "spectrumWidth"
  | "audioInput"
  | "audioOutput"
  | "inputVolume"
  | "txVolume";

/**
 * Radio connection + audio I/O -- a row+drill-down redesign matching the
 * real Android app's `RadioAudioSettings.kt` exactly (confirmed against live
 * screenshots of the real app: each enumerable field -- Rig Model, Control
 * Mode, Connection Mode, Baud Rate, Band & Frequency, Enabled Bands, Audio
 * Frequency, Spectrum Width, Audio Input/Output, Input/TX Volume -- is its
 * own tappable row opening a picker; Receive/Transmit Audio Channel are
 * inline segmented controls; QMX CAT tones/Full Duplex/Show Volume Slider/
 * per-band memory are inline toggles), not desktop's old single scroll of
 * inline sliders/selects. The goal (per the user) is full parity, not just
 * visual: every row here is backed by real desktop logic, including two
 * pieces that needed new Rust work to port honestly rather than fake:
 *
 * - **Control Mode** (VOX/CAT/RTS/DTR): ported from Android's own
 *   `ControlMode`, ONLY for the Hamlib backend (FLrig manages its own PTT;
 *   G106 has a fixed CAT-PTT protocol). Maps onto Hamlib's own `ptt_type`
 *   conf token (confirmed via `rigctl --help`) -- RTS/DTR still use the
 *   Hamlib connection for CAT frequency control, only the PTT signal itself
 *   moves to the serial line (matching Android's own doc comment: "CI-V
 *   commands are still effective"); VOX asserts no PTT at all. See
 *   `rig.rs`'s `RigControlMode`.
 * - **Show Volume Slider**: a real, functional toggle (`showTxVolumeSlider`,
 *   Android's own key, default on) gating the inline TX-volume row in the
 *   main screen's "More controls" drawer (`App.tsx`'s `TxBar`) -- not just
 *   a decorative switch.
 * - **Full Duplex** is the one disabled/stub row: Android's version is a
 *   decode-list filter (`OwnTxEchoFilter`, hides/shows decodes of the
 *   operator's own TX), not an audio-path feature as its description might
 *   suggest, but desktop's decode pipeline doesn't track "is this decode an
 *   echo of our own last transmission" at all yet -- real but bounded future
 *   work, not faked here.
 * - **Rig Model** stays desktop's existing single combined selector (None/
 *   FLrig/G106/every Hamlib model as one flat list, see `rig.ts`) rather
 *   than adopting Android's separate rig-model-vs-transport split --
 *   desktop's Hamlib-backed architecture (one connection abstraction behind
 *   `RigTransport`, a decision already made earlier in this project) doesn't
 *   have a second axis for this to meaningfully split into.
 *
 * **Enabled bands**: a checklist over the real (unfiltered) band list for
 * the current mode, persisted as `excludedBands` -- Android's own key
 * name/shape (comma-separated band names), now its own dialog (toggle rows)
 * instead of a chip grid, with Android's own "at least one band must stay
 * enabled" guard. **Per-band volume memory**: `perBandInputLevel`/
 * `perBandOutputLevel` toggles, no per-band table to review -- Android has
 * none either; the levels themselves save/restore silently as the operator
 * changes bands (see `perBandLevels.ts`, wired up in `App.tsx`'s `onBand`).
 * **QMX CAT tones** and **TX/RX audio channel**: the CAT-tones toggle and
 * TX-channel segment display whatever is *actually in effect* (from the
 * `rig` status event's `tx_via_cat_tones`/`tx_audio_channel`, a live
 * auto-detect/one-time-latch resolution -- see `engine.rs`'s
 * `resolve_tx_audio_channel`/`should_default_qmx_cat_tones_on`), not the raw
 * possibly-unset config. RX has no auto-detect counterpart, so it just
 * reads/writes its own config directly.
 */
export function RadioAudioSettings({
  onStatus,
  onRigLabel,
  onBandsChanged,
  onRigConnected,
  operatingMode,
  rig,
  bands,
  dialHz,
  onBand,
  onMode,
  showVolumeSlider,
  onShowVolumeSlider,
  inputGain,
  onInputGain,
  txGain,
  onTxGain,
  displayHz,
  onDisplayHz,
  onUpdateGridTable,
  gridSeedBusy,
  onAudioBandConfigChanged,
}: {
  onStatus: (s: string) => void;
  onRigLabel: (v: string) => void;
  onBandsChanged: (bands: BandInfo[]) => void;
  onRigConnected: (rigBackend: string) => void;
  operatingMode: OperatingMode;
  rig: RigStatusEvent | null;
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  onMode: (m: OperatingMode) => void;
  showVolumeSlider: boolean;
  onShowVolumeSlider: (v: boolean) => void;
  inputGain: number;
  onInputGain: (v: number) => void;
  txGain: number;
  onTxGain: (v: number) => void;
  displayHz: number;
  onDisplayHz: (hz: number) => void;
  onUpdateGridTable: () => void;
  gridSeedBusy: boolean;
  onAudioBandConfigChanged: () => void;
}) {
  const [rigLabel, setRigLabel] = useState("");
  const [inputs, setInputs] = useState<AudioDevice[]>([]);
  const [outputs, setOutputs] = useState<AudioDevice[]>([]);
  const [ports, setPorts] = useState<SerialPortInfo[]>([]);
  const [hamlibRigs, setHamlibRigs] = useState<HamlibRig[]>([]);
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [baseFreq, setBaseFreq] = useState(1500);
  const [wfCfg, setWfCfg] = useState<WaterfallConfig>({
    window: "hann",
    fft_size: 2048,
    avg: 6,
    display_hz: displayHz,
    denoise: false,
  });
  useEffect(() => {
    setWfCfg((prev) => ({ ...prev, display_hz: displayHz }));
  }, [displayHz]);
  const [udp, setUdp] = useState<UdpConfig>({
    enabled: false,
    host: "127.0.0.1",
    port: 2237,
    accept_requests: false,
  });
  const [rigCfg, setRigCfg] = useState<RigConfig>(DEFAULT_RIG_CONFIG);

  // Which row+dialog is currently open -- one flag covers every picker
  // below since only one can be open at a time (matches Android's own
  // one-dialog-at-a-time pattern in RadioAudioSettings.kt).
  const [openDialog, setOpenDialog] = useState<OpenDialog>(null);

  // New: enabled-bands checklist (the real, unfiltered list for this mode --
  // see this component's own doc comment for why it can't reuse the
  // already-filtered list App.tsx passes elsewhere).
  const [allBands, setAllBands] = useState<BandInfo[]>([]);
  const [excludedBands, setExcludedBands] = useState<Set<string>>(new Set());
  const [bandToggleError, setBandToggleError] = useState<string | null>(null);
  useEffect(() => {
    api.listBandsForMode(operatingMode).then(setAllBands);
  }, [operatingMode]);
  useEffect(() => {
    api.getConfig("excludedBands").then((v) => setExcludedBands(parseExcludedBands(v)));
  }, []);
  const nonCustomBands = allBands.filter((b) => !b.custom);
  const enabledBandsCount = nonCustomBands.length - excludedBands.size;

  // Every raw band list this screen can produce (the toggle below, and
  // CustomBandsPanel's own add/remove, which both return the *unfiltered*
  // list from their own IPC calls) must pass through the current exclusion
  // set before reaching App.tsx's `bands` state -- otherwise adding/removing
  // a custom dial would silently undo the enabled-bands filter until the
  // next mode change.
  function pushFilteredBands(rawBands: BandInfo[]) {
    onBandsChanged(filterEnabledBands(rawBands, serializeExcludedBands(excludedBands)));
  }

  function toggleBandEnabled(name: string, enabled: boolean) {
    // Android's own guard: "At least one band must stay enabled."
    if (!enabled && enabledBandsCount <= 1) {
      setBandToggleError("At least one band must stay enabled.");
      return;
    }
    setBandToggleError(null);
    const next = new Set(excludedBands);
    if (enabled) next.delete(name);
    else next.add(name);
    setExcludedBands(next);
    const csv = serializeExcludedBands(next);
    api.setConfig("excludedBands", csv);
    api.listBandsForMode(operatingMode).then((bands) => onBandsChanged(filterEnabledBands(bands, csv)));
    onAudioBandConfigChanged();
  }

  // New: per-band volume memory toggles (levels themselves are wired up in
  // App.tsx's onBand -- no table to show/edit here, matching Android).
  const [perBandInputEnabled, setPerBandInputEnabled] = useState(false);
  const [perBandOutputEnabled, setPerBandOutputEnabled] = useState(false);
  useEffect(() => {
    api.getConfig("perBandInputLevel").then((v) => setPerBandInputEnabled(v === "1"));
    api.getConfig("perBandOutputLevel").then((v) => setPerBandOutputEnabled(v === "1"));
  }, []);
  function togglePerBandInput(on: boolean) {
    setPerBandInputEnabled(on);
    api.setConfig("perBandInputLevel", on ? "1" : "0");
    onAudioBandConfigChanged();
  }
  function togglePerBandOutput(on: boolean) {
    setPerBandOutputEnabled(on);
    api.setConfig("perBandOutputLevel", on ? "1" : "0");
    onAudioBandConfigChanged();
  }

  // New: QMX CAT tones + TX/RX audio channel overrides -- apply live via
  // their own IPC calls, no rig reconnect needed (see engine.rs).
  const [qmxCatTones, setQmxCatTonesState] = useState(false);
  const [txAudioChannel, setTxAudioChannelState] = useState(0);
  const [rxAudioChannel, setRxAudioChannelState] = useState(0);
  useEffect(() => {
    api.getConfig("qmxCatTones").then((v) => setQmxCatTonesState(v === "1"));
    api.getConfig("txAudioChannel").then((v) => setTxAudioChannelState(v ? parseInt(v, 10) || 0 : 0));
    api.getConfig("rxAudioChannel").then((v) => setRxAudioChannelState(v ? parseInt(v, 10) || 0 : 0));
  }, []);
  // Once a rig status arrives, prefer its *effective* (post-auto-detect)
  // values over raw config for display -- see the component doc comment.
  useEffect(() => {
    if (rig) {
      setQmxCatTonesState(rig.tx_via_cat_tones);
      setTxAudioChannelState(rig.tx_audio_channel);
    }
  }, [rig]);
  function onQmxCatTones(on: boolean) {
    setQmxCatTonesState(on);
    api.setQmxCatTones(on);
  }
  function onTxAudioChannel(v: number) {
    setTxAudioChannelState(v);
    api.setTxAudioChannel(v);
  }
  function onRxAudioChannel(v: number) {
    setRxAudioChannelState(v);
    api.setRxAudioChannel(v);
  }

  useEffect(() => {
    api.listAudioInputs().then(setInputs);
    api.listAudioOutputs().then(setOutputs);
    api.listSerialPorts().then(setPorts);
    api.listHamlibRigs().then(setHamlibRigs);
    api.getConfig("input_device").then((v) => v && setInput(v));
    api.getConfig("output_device").then((v) => v && setOutput(v));
    api.getConfig("base_freq").then((v) => v && setBaseFreq(parseInt(v, 10)));
    api.getConfig("rig_config").then((v) => {
      if (v) {
        try {
          // Backfill with DEFAULT_RIG_CONFIG first -- a blob saved before a
          // field existed (confirmed live: a real persisted rig_config
          // predating `control_mode` leaves it `undefined` after a plain
          // cast, blanking the Control Mode row) must not leak `undefined`
          // into state just because JSON.parse has no schema to fall back
          // to, unlike Rust's own `#[serde(default)]` on the same struct.
          const parsed = { ...DEFAULT_RIG_CONFIG, ...(JSON.parse(v) as Partial<RigConfig>) };
          setRigCfg(applySelection(parsed, configToSelection(parsed)));
        } catch {
          /* ignore malformed saved config */
        }
      }
    });
    api.getConfig("rig_label").then((v) => v && setRigLabel(v));
    Promise.all([
      api.getConfig("wf_window"),
      api.getConfig("wf_fft_size"),
      api.getConfig("wf_avg"),
      api.getConfig("wf_denoise"),
    ]).then(([w, size, avg, denoiseStr]) => {
      const fftSize = parseInt(size ?? "", 10);
      const avgN = parseInt(avg ?? "", 10);
      setWfCfg((prev) => ({
        window: WF_WINDOWS.includes(w as WfWindow) ? (w as WfWindow) : prev.window,
        fft_size: WF_FFT_SIZES.includes(fftSize) ? fftSize : prev.fft_size,
        avg: WF_AVG_OPTIONS.includes(avgN) ? avgN : prev.avg,
        display_hz: prev.display_hz,
        denoise: denoiseStr === "true",
      }));
    });
    Promise.all([
      api.getConfig("udp_enabled"),
      api.getConfig("udp_host"),
      api.getConfig("udp_port"),
      api.getConfig("udp_accept_requests"),
    ]).then(([en, host, port, accept]) => {
      const p = parseInt(port ?? "", 10);
      setUdp((prev) => ({
        enabled: en === "true",
        host: host && host.length > 0 ? host : prev.host,
        port: Number.isFinite(p) && p > 0 && p <= 65535 ? p : prev.port,
        accept_requests: accept === "true",
      }));
    });
  }, []);

  function applyWfCfg(next: WaterfallConfig) {
    setWfCfg(next);
    api.setWaterfallConfig(next);
  }

  function applyUdp(next: UdpConfig) {
    const port = Number.isFinite(next.port) && next.port > 0 && next.port <= 65535 ? next.port : 2237;
    const host = next.host.trim().length > 0 ? next.host.trim() : "127.0.0.1";
    const cfg: UdpConfig = { ...next, host, port };
    setUdp(cfg);
    api.setUdpConfig(cfg);
    onStatus(
      cfg.enabled
        ? `WSJT-X UDP → ${cfg.host}:${cfg.port}${cfg.accept_requests ? " (accepting requests)" : ""}`
        : "WSJT-X UDP off",
    );
  }

  const refreshPorts = () => api.listSerialPorts().then(setPorts);
  const showsPortList = rigCfg.backend === "g106" || (rigCfg.backend === "hamlib" && !rigCfg.hamlib_network);
  useEffect(() => {
    if (showsPortList) refreshPorts();
  }, [showsPortList]);

  const currentBand = bands.find((b) => b.dial_hz === dialHz);
  const connectionModeValue = rigCfg.hamlib_network ? "network" : "serial";

  // Connect/Disconnect stay explicit buttons rather than auto-applying each
  // field edit live (unlike the other rows here) -- establishing a serial/
  // network connection is a real action that can fail, and auto-connecting
  // on every intermediate field change (e.g. Rig Model picked before Baud
  // Rate is set) would fire doomed connection attempts mid-edit. Desktop's
  // existing deliberate design, kept as-is through this redesign.
  function connectRig() {
    api.selectRig(rigCfg);
    const detail =
      rigCfg.backend === "flrig"
        ? ` (${rigCfg.flrig_host}:${rigCfg.flrig_port})`
        : rigCfg.backend === "g106"
        ? ` (${rigCfg.port || "no port"} at 19200 baud)`
        : rigCfg.backend === "hamlib"
        ? ` (${rigCfg.hamlib_network ? rigCfg.hamlib_address : rigCfg.port || "no port"})`
        : "";
    onStatus(`Connecting ${rigName(rigCfg, hamlibRigs)}${detail}`);
    onRigConnected(rigCfg.backend);
  }

  return (
    <>
      <div className="panel">
        <h3>Radio</h3>
        <div className="settings-category-list">
          <SettingsRow label="Rig Model" value={rigName(rigCfg, hamlibRigs)} onClick={() => setOpenDialog("rigModel")} />
          {rigCfg.backend === "hamlib" && (
            <>
              <SettingsRow label="Control Mode" value={CONTROL_MODE_LABEL[rigCfg.control_mode]} onClick={() => setOpenDialog("controlMode")} />
              {/* Android greys this row out (non-clickable, not hidden) when
                  Control Mode is VOX (`isCatMode` gate) -- Serial Port/Baud
                  Rate/Network Address below stay clickable regardless,
                  though, since desktop's unified Hamlib connection still
                  needs them for CAT frequency tracking even under VOX PTT. */}
              <SettingsRow
                label="Connection Mode"
                value={rigCfg.hamlib_network ? "Network" : "USB Cable"}
                onClick={rigCfg.control_mode === "vox" ? undefined : () => setOpenDialog("connectionMode")}
                disabled={rigCfg.control_mode === "vox"}
              />
              {rigCfg.hamlib_network ? (
                <SettingsRow label="Network Address" value={rigCfg.hamlib_address} disabled />
              ) : (
                <>
                  <SettingsRow label="Serial Port" value={rigCfg.port || "(none)"} onClick={() => { refreshPorts(); setOpenDialog("serialPort"); }} />
                  <SettingsRow label="Baud Rate" value={String(rigCfg.baud)} onClick={() => setOpenDialog("baudRate")} />
                </>
              )}
            </>
          )}
          {rigCfg.backend === "g106" && (
            <SettingsRow label="Serial Port" value={rigCfg.port || "(none)"} onClick={() => { refreshPorts(); setOpenDialog("serialPort"); }} />
          )}
          {rigCfg.backend === "flrig" && (
            <SettingsRow label="FLrig" value={`${rigCfg.flrig_host}:${rigCfg.flrig_port}`} disabled />
          )}
          <SettingsRow
            label="Band & Frequency"
            value={`${(dialHz / 1e6).toFixed(3)}MHz (${currentBand ? currentBand.name : "--"})`}
            onClick={() => setOpenDialog("bandFrequency")}
          />
          <SettingsRow
            label="Enabled Bands"
            value={`${enabledBandsCount} of ${nonCustomBands.length} enabled`}
            onClick={() => setOpenDialog("enabledBands")}
          />
          <SettingsRow label="Audio Frequency" value={`${baseFreq} Hz`} onClick={() => setOpenDialog("audioFrequency")} />
          <SettingsRow label="Spectrum Width" value={`${Math.round(displayHz)} Hz`} onClick={() => setOpenDialog("spectrumWidth")} />
        </div>

        {/* Network/FLrig addresses are free text, not enumerable -- kept as
            inline fields rather than a picker dialog. */}
        {rigCfg.backend === "hamlib" && rigCfg.hamlib_network && (
          <div className="field" style={{ marginTop: 10 }}>
            <label>Network address (host:port)</label>
            <input
              value={rigCfg.hamlib_address}
              onChange={(e) => setRigCfg({ ...rigCfg, hamlib_address: e.target.value })}
              placeholder="127.0.0.1:4992"
              style={{ width: 180 }}
            />
            <span className="muted" style={{ display: "block", marginTop: 4 }}>
              Native FlexRadio 6xxx model: <code>&lt;radio-IP&gt;:4992</code>. For
              SmartSDR CAT's TCP port, use model <b>Kenwood TS-2000</b> +{" "}
              <code>127.0.0.1:&lt;cat-port&gt;</code> (e.g. 5002). Use 127.0.0.1, not
              localhost.
            </span>
          </div>
        )}
        {rigCfg.backend === "flrig" && (
          <div className="row" style={{ marginTop: 10 }}>
            <div className="field">
              <label>FLrig host</label>
              <input
                value={rigCfg.flrig_host}
                onChange={(e) => setRigCfg({ ...rigCfg, flrig_host: e.target.value })}
                style={{ width: 130 }}
              />
            </div>
            <div className="field">
              <label>Port</label>
              <input
                value={rigCfg.flrig_port}
                onChange={(e) => setRigCfg({ ...rigCfg, flrig_port: parseInt(e.target.value, 10) || 12345 })}
                style={{ width: 80 }}
              />
            </div>
          </div>
        )}
        {hamlibRigs.length === 0 && (
          <span className="muted" style={{ display: "block", marginTop: 10 }}>
            The built-in choices remain available, but Hamlib radios are not listed —
            the Hamlib library didn't load, so no radios could be enumerated. Install
            Hamlib and put <code>hamlib-4.dll</code> / <code>libhamlib</code> next to
            the app or on PATH, then reopen.
          </span>
        )}

        <div className="field" style={{ marginTop: 10 }}>
          <label>Display name (optional)</label>
          <input
            value={rigLabel}
            onChange={(e) => {
              setRigLabel(e.target.value);
              onRigLabel(e.target.value);
            }}
            placeholder="e.g. Flex 6400"
          />
          <span className="muted" style={{ display: "block", marginTop: 4 }}>
            Shown in the top bar instead of the CAT model. Useful when connecting
            a radio through an emulator (e.g. a Flex via SmartSDR CAT reports as
            Kenwood TS-2000). Leave blank to show the detected model.
          </span>
        </div>

        <div className="row" style={{ marginTop: 10 }}>
          <button className="primary" onClick={connectRig}>Connect rig</button>
          <button
            onClick={() => {
              api.disconnectRig();
              onStatus("Rig disconnected");
            }}
          >
            Disconnect
          </button>
        </div>
      </div>

      <div className="panel">
        <h3>Audio</h3>
        <div className="row" style={{ justifyContent: "space-between" }}>
          <span className="muted" style={{ fontSize: 12 }}>
            Plugged in a device after opening Settings? Re-scan below.
          </span>
          <button
            onClick={() => {
              api.listAudioInputs().then(setInputs);
              api.listAudioOutputs().then(setOutputs);
            }}
          >
            ↻ Refresh audio devices
          </button>
        </div>
        <div className="settings-category-list" style={{ marginTop: 8 }}>
          <SettingsRow
            label="Audio Input"
            value={input ? (inputs.find((d) => d.id === input) ? audioDeviceLabel(inputs.find((d) => d.id === input)!) : input) : "System default"}
            onClick={() => setOpenDialog("audioInput")}
          />
          <SettingsRow
            label="Audio Output"
            value={output ? (outputs.find((d) => d.id === output) ? audioDeviceLabel(outputs.find((d) => d.id === output)!) : output) : "System default"}
            onClick={() => setOpenDialog("audioOutput")}
          />
          <SettingsRow label="Audio Frequency (TX tone)" value={`${baseFreq} Hz`} onClick={() => setOpenDialog("audioFrequency")} />
          <SettingsRow
            label="Input Volume"
            description="RX audio gain applied before decoding (100% = unchanged)"
            value={`${inputGain}%`}
            onClick={() => setOpenDialog("inputVolume")}
          />
          <SettingsSegmentedRow
            label="Receive Audio Channel"
            description="Which side of a stereo input to decode. Use Left or Right when the radio only feeds one channel."
            options={RX_CHANNEL_OPTIONS}
            value={rxAudioChannel}
            onChange={onRxAudioChannel}
          />
          <SettingsSegmentedRow
            label="Transmit Audio Channel"
            description="Which side of a stereo output carries the transmitted audio. Use Left or Right to drive one radio on a splitter cable."
            options={TX_CHANNEL_OPTIONS}
            value={txAudioChannel}
            onChange={onTxAudioChannel}
          />
          <SettingsRow
            label="QMX: Send Tones over CAT"
            description="QRP Labs QMX only. Transmit by sending each tone as a CAT command, so the radio makes the signal itself and no audio is needed. Use this when the radio's audio input level stays too low to transmit. Turned on automatically the first time a QMX connects."
            toggle={qmxCatTones}
            onToggleChange={onQmxCatTones}
          />
          <SettingsRow
            label="Full Duplex"
            description="Show decodes of your own transmission instead of hiding them. For satellite work: check your own signal strength and the frequency it comes back on. Not yet available on desktop -- needs the decode pipeline to track self-transmission echoes."
            toggle={false}
            onToggleChange={() => {}}
            disabled
          />
          <SettingsRow
            label="TX Volume"
            description="Transmit audio level (hardware buttons ±5%)"
            value={`${txGain}%`}
            onClick={() => setOpenDialog("txVolume")}
          />
          <SettingsRow
            label="Show Volume Slider"
            description="Display TX volume slider on main screen"
            toggle={showVolumeSlider}
            onToggleChange={onShowVolumeSlider}
          />
          {/* "Restore Media Volume" (Android row between Show Volume Slider
              and Controls Drawer) is genuinely Android-only -- it saves/
              restores the *phone OS's* media volume around each TX, since
              some Android phones auto-duck it. Desktop has no equivalent
              concept (no OS media-volume ducking to work around), so unlike
              Full Duplex/Controls Drawer it isn't even shown as a stub --
              same "naturally drops, nothing to port" bucket as USB
              diagnostics/Bluetooth/the locale picker from Phase 1's plan. */}
          <SettingsRow
            label="Controls Drawer"
            description="Collapse the operate controls into a bottom sheet you pull up. Off shows the full control strip at all times."
            toggle={true}
            onToggleChange={() => {}}
            disabled
          />
          <SettingsRow
            label="Save Output Level per Band"
            description="Remember the TX volume separately for each band and restore it on band change"
            toggle={perBandOutputEnabled}
            onToggleChange={togglePerBandOutput}
          />
          <SettingsRow
            label="Save Input Volume per Band"
            description="Remember the RX input volume separately for each band and restore it on band change"
            toggle={perBandInputEnabled}
            onToggleChange={togglePerBandInput}
          />
        </div>
      </div>

      {openDialog === "rigModel" && (
        <ListPickerDialog
          title="Rig Model"
          searchable={hamlibRigs.length > 20}
          options={rigOptions(hamlibRigs)}
          value={configToSelection(rigCfg)}
          onSelect={(v) => { setRigCfg(applySelection(rigCfg, v)); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "controlMode" && (
        <ListPickerDialog
          title="Control Mode"
          options={CONTROL_MODE_OPTIONS}
          value={rigCfg.control_mode}
          onSelect={(v) => { setRigCfg({ ...rigCfg, control_mode: v }); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "connectionMode" && (
        <ListPickerDialog
          title="Connection Mode"
          options={[
            { value: "serial", label: "Serial port (COM)" },
            { value: "network", label: "Network (TCP host:port)" },
          ]}
          value={connectionModeValue}
          onSelect={(v) => { setRigCfg({ ...rigCfg, hamlib_network: v === "network" }); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "serialPort" && (
        <ListPickerDialog
          title="Serial Port"
          options={[
            { value: "", label: rigCfg.backend === "g106" ? "(select port)" : "(none — e.g. Dummy)" },
            ...ports.map((p) => ({ value: p.name, label: `${p.name} (${p.kind})` })),
          ]}
          value={rigCfg.port}
          onSelect={(v) => { setRigCfg({ ...rigCfg, port: v }); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "baudRate" && (
        <ListPickerDialog
          title="Baud Rate"
          options={BAUD_RATES.map((b) => ({ value: b, label: String(b) }))}
          value={rigCfg.baud}
          onSelect={(v) => { setRigCfg({ ...rigCfg, baud: v }); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "bandFrequency" && (
        <BandModeSheet
          bands={bands}
          dialHz={dialHz}
          onBand={(hz) => { onBand(hz); setOpenDialog(null); }}
          mode={operatingMode}
          onMode={onMode}
          onClose={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "enabledBands" && (
        <div className="modal-overlay" onClick={() => setOpenDialog(null)}>
          <div className="modal list-picker-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h3>Enabled Bands</h3>
              <button className="modal-close" aria-label="Close" onClick={() => setOpenDialog(null)}>✕</button>
            </div>
            <div className="muted" style={{ fontSize: 12 }}>Hidden bands won't appear in the frequency pickers.</div>
            {bandToggleError && <div style={{ color: "var(--tx)", fontSize: 12 }}>⚠ {bandToggleError}</div>}
            <div className="list-picker-options">
              {nonCustomBands.map((b) => (
                <SettingsRow
                  key={b.name}
                  label={b.name}
                  toggle={!excludedBands.has(b.name)}
                  onToggleChange={(v) => toggleBandEnabled(b.name, v)}
                />
              ))}
            </div>
          </div>
        </div>
      )}
      {openDialog === "audioFrequency" && (
        <NumberInputDialog
          title="Audio Frequency"
          description="TX tone frequency within the passband."
          value={baseFreq}
          min={200}
          max={2800}
          unit="Hz"
          onSave={(v) => { setBaseFreq(v); api.setBaseFreq(v); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "spectrumWidth" && (
        <NumberInputDialog
          title="Spectrum Width"
          value={displayHz}
          min={2500}
          max={5000}
          step={100}
          unit="Hz"
          onSave={(v) => { onDisplayHz(v); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "audioInput" && (
        <ListPickerDialog
          title="Audio Input"
          options={[
            { value: "", label: "System default" },
            ...inputs.map((d) => ({ value: d.id, label: audioDeviceLabel(d) + (d.is_default ? " (default)" : "") })),
          ]}
          value={input}
          onSelect={(v) => { setInput(v); api.setInputDevice(v || null); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "audioOutput" && (
        <ListPickerDialog
          title="Audio Output"
          options={[
            { value: "", label: "System default" },
            ...outputs.map((d) => ({ value: d.id, label: audioDeviceLabel(d) + (d.is_default ? " (default)" : "") })),
          ]}
          value={output}
          onSelect={(v) => { setOutput(v); api.setOutputDevice(v || null); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "inputVolume" && (
        <NumberInputDialog
          title="Input Volume"
          description="RX audio gain applied before decoding (100% = unchanged)"
          value={inputGain}
          min={0}
          max={100}
          unit="%"
          onSave={(v) => { onInputGain(v); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "txVolume" && (
        <NumberInputDialog
          title="TX Volume"
          description="Transmit audio level (hardware buttons ±5%)"
          value={txGain}
          min={0}
          max={100}
          unit="%"
          onSave={(v) => { onTxGain(v); setOpenDialog(null); }}
          onCancel={() => setOpenDialog(null)}
        />
      )}

      <CustomBandsPanel onStatus={onStatus} onBandsChanged={pushFilteredBands} />

      <CallsignGridPanel onUpdate={onUpdateGridTable} busy={gridSeedBusy} />

      <div className="panel">
        <h3>Developer — waterfall FFT</h3>
        <div className="col">
          <div className="muted">
            Spectrum/waterfall rendering experiments (issue #428) — affects the
            display only, never decoding. Defaults: Hann, 2048, 6 averages.
          </div>
          <div className="field">
            <label>Spectrum width: {Math.round(displayHz)} Hz</label>
            <input
              type="range"
              aria-label="Spectrum width, Hz"
              min={2500}
              max={5000}
              step={100}
              value={displayHz}
              onChange={(e) => onDisplayHz(parseInt(e.target.value, 10))}
            />
            <span className="muted" style={{ fontSize: 12 }}>
              Same setting as the Width control on the Decode waterfall strip.
            </span>
          </div>
          <div className="field">
            <label>Window function</label>
            <select value={wfCfg.window} onChange={(e) => applyWfCfg({ ...wfCfg, window: e.target.value as WfWindow })}>
              {WF_WINDOWS.map((w) => (
                <option key={w} value={w}>
                  {WF_WINDOW_LABELS[w]}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>FFT size</label>
            <select value={wfCfg.fft_size} onChange={(e) => applyWfCfg({ ...wfCfg, fft_size: parseInt(e.target.value, 10) })}>
              {WF_FFT_SIZES.map((n) => (
                <option key={n} value={n}>
                  {n} — {(12000 / n).toFixed(1)} Hz/bin{n === 2048 ? " (default)" : ""}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>Averaging (Welch segments per row)</label>
            <select value={wfCfg.avg} onChange={(e) => applyWfCfg({ ...wfCfg, avg: parseInt(e.target.value, 10) })}>
              {WF_AVG_OPTIONS.map((n) => (
                <option key={n} value={n}>
                  {n}
                  {n === 6 ? " (default)" : n === 1 ? " (off)" : ""}
                </option>
              ))}
            </select>
          </div>
        </div>
      </div>

      <div className="panel">
        <h3>WSJT-X UDP</h3>
        <div className="col">
          <div className="muted">
            Broadcast decodes, status and logged QSOs over UDP in the WSJT-X
            protocol so companion apps (GridTracker, JTAlert, N1MM+, Log4OM) can
            plot and log them. Default target 127.0.0.1:2237.
          </div>
          <div className="field">
            <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={udp.enabled}
                onChange={(e) => applyUdp({ ...udp, enabled: e.target.checked })}
                style={{ width: "auto" }}
              />
              Enable UDP broadcast
            </label>
          </div>
          <div className="field">
            <label>Server address (host)</label>
            <input
              type="text"
              value={udp.host}
              disabled={!udp.enabled}
              onChange={(e) => setUdp({ ...udp, host: e.target.value })}
              onBlur={() => applyUdp(udp)}
              placeholder="127.0.0.1"
            />
          </div>
          <div className="field">
            <label>Server port</label>
            <input
              type="number"
              min={1}
              max={65535}
              value={udp.port}
              disabled={!udp.enabled}
              onChange={(e) => setUdp({ ...udp, port: parseInt(e.target.value, 10) || 0 })}
              onBlur={() => applyUdp(udp)}
              placeholder="2237"
            />
          </div>
          <div className="field">
            <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={udp.accept_requests}
                disabled={!udp.enabled}
                onChange={(e) => applyUdp({ ...udp, accept_requests: e.target.checked })}
                style={{ width: "auto" }}
              />
              Accept UDP requests (let companion apps reply/halt/free-text)
            </label>
          </div>
        </div>
      </div>
    </>
  );
}

function CustomBandsPanel(props: { onStatus: (s: string) => void; onBandsChanged: (bands: BandInfo[]) => void }) {
  const { onStatus, onBandsChanged } = props;
  const [custom, setCustom] = useState<CustomBand[]>([]);
  const [name, setName] = useState("");
  const [freq, setFreq] = useState("");
  const [error, setError] = useState("");

  function refresh() {
    api.listCustomBands().then(setCustom);
  }
  useEffect(refresh, []);

  async function add() {
    setError("");
    try {
      const merged = await api.addCustomBand(freq, name);
      onBandsChanged(merged);
      refresh();
      onStatus(`Custom band saved: ${name || "(band)"} ${freq} Hz`);
      setName("");
      setFreq("");
    } catch (e) {
      setError(String(e));
    }
  }

  async function edit(b: CustomBand) {
    setName(b.name);
    setFreq(String(b.dial_hz));
    setError("");
  }

  async function remove(dialHz: number) {
    const merged = await api.deleteCustomBand(dialHz);
    onBandsChanged(merged);
    refresh();
  }

  return (
    <div className="panel">
      <h3>Custom dial frequencies</h3>
      <div className="col">
        <div className="muted">
          Add off-plan dials (e.g. DXpedition frequencies) in whole Hz. They
          appear in the band picker marked with a ★ and persist across restarts.
        </div>
        <div className="row">
          <div className="field">
            <label>Label (optional)</label>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. 3B9 DX" style={{ width: 120 }} />
          </div>
          <div className="field">
            <label>Dial frequency (Hz)</label>
            <input value={freq} onChange={(e) => setFreq(e.target.value)} placeholder="e.g. 14090000" style={{ width: 130 }} />
          </div>
          <button className="primary" onClick={add}>
            Add / update
          </button>
        </div>
        {error && <span style={{ color: "var(--tx)" }}>⚠ {error}</span>}
        {custom.length === 0 ? (
          <span className="muted">No custom dials yet.</span>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Label</th>
                <th>Dial (MHz)</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {custom.map((b) => (
                <tr key={b.dial_hz}>
                  <td>{b.name}</td>
                  <td style={{ fontVariantNumeric: "tabular-nums" }}>{(b.dial_hz / 1e6).toFixed(6)}</td>
                  <td>
                    <button onClick={() => edit(b)} title="Load into the form to edit">
                      ✎
                    </button>
                    <button onClick={() => remove(b.dial_hz)} title="Delete">
                      ✕
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

function CallsignGridPanel(props: { onUpdate: () => void; busy: boolean }) {
  const { onUpdate, busy } = props;
  const [count, setCount] = useState<number | null>(null);

  function refresh() {
    api.callsignQthCount().then(setCount);
  }
  useEffect(refresh, []);
  useEffect(() => {
    if (!busy) refresh();
  }, [busy]);

  return (
    <div className="panel">
      <h3>US callsign/grid table</h3>
      <div className="col">
        <div className="muted">
          Seeds the Map screen's callsign-to-grid lookup from the FCC
          amateur-license database (~200MB download, cross-referenced
          against a bundled ZIP-code table) — the first-run download also
          happens automatically once, and otherwise fills in from decodes
          and the logbook as you operate. The FCC data changes weekly.
        </div>
        <div className="row">
          <span>{count == null ? "Checking…" : `${count.toLocaleString()} station${count === 1 ? "" : "s"} known`}</span>
          <button className="primary" onClick={onUpdate} disabled={busy}>
            {busy ? "Updating…" : "Update now"}
          </button>
        </div>
      </div>
    </div>
  );
}
