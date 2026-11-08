import { Fragment, useEffect, useMemo, useRef, useState, type ChangeEvent, type MouseEvent } from "react";
import {
  api,
  onEngineEvent,
  onImportProgress,
  type AudioDevice,
  type BandInfo,
  type ClockSyncEvent,
  type CustomBand,
  type CycleTick,
  type EngineEvent,
  type HamlibRig,
  type ImportProgress,
  type ImportSummary,
  type QsoRecord,
  type RigConfig,
  type RigStatusEvent,
  type SerialPortInfo,
  type TxStage,
  type TxStateEvent,
  type UdpConfig,
  type UiMessage,
  type WaterfallConfig,
  type WfWindow,
} from "./ipc";
import {
  adifToDateInput,
  buildExportArgs,
  buildQsoRecord,
  computeVirtualWindow,
  displayReport,
  formatHhmm,
  formFromQso,
  isLogFiltered,
  logbookPlace,
  qslMarks,
  qsoCountLabel,
  qsoStatusLabel,
  QSO_COL_DATE,
  QSO_TABLE_COLUMNS,
  sortQsoTable,
  type ConfirmFilter,
  type QsoEditForm,
} from "./logbook";
import { applySelection, configToSelection, rigName, rigOptions } from "./rig";
import { debounce } from "./debounce";
import { audioDeviceLabel } from "./audioDevice";
import {
  countRepliesTo,
  huntPriorityFromConfig,
  isPotaCq,
  maidenheadDistanceKm,
  selectHuntCandidate,
  HUNT_PRIORITIES,
  type HuntCandidate,
  type HuntPriority,
} from "./hunt";
import { formatPowerWatts, formatSwrRatio, swrBand } from "./meters";
import { wpxPrefixOf } from "./wpxPrefix";

type Tab = "decode" | "waterfall" | "log" | "settings" | "map" | "pota";
type Filter = "all" | "cq" | "pota" | "dxcc" | "zone" | "state" | "grid" | "prefix" | "needed" | "tome";
const MAX_MESSAGES = 800;

// "No limit" for the Logbook fetch: Number.MAX_SAFE_INTEGER round-trips
// cleanly to Rust's i64 and SQLite treats a LIMIT this large as unbounded.
// The table sorts client-side over the full fetched set, so a smaller cap
// here would silently make sorting wrong on a log bigger than the cap.
const LOG_FETCH_ALL = Number.MAX_SAFE_INTEGER;

// Quiet period a gain slider must see before its value is sent to the
// backend. Long enough to collapse a drag gesture, short enough that a
// deliberate single adjustment still feels immediate.
const GAIN_DEBOUNCE_MS = 120;

// The live waterfall draws imperatively to a canvas (registered by WaterfallView)
// so per-row spectrum events never go through React state. Backing-buffer
// height, in rows -- one row is emitted per ~100ms tick (see engine.rs's
// emit_waterfall_row), so a 15s FT8 cycle is ~150 rows (plus the boundary
// band's extra few px once per cycle). 500 rows keeps at least 3 full cycles
// of history visible before the oldest row scrolls off (the previous 160
// barely held one cycle). The canvas's on-screen CSS size is independent of
// this (it fills its panel -- see WaterfallView), so a taller buffer just
// packs more history into the same screen space.
const WF_HEIGHT = 500;
const WF_TICK_STEP_HZ = 500;
// Top of the displayed audio band; matches the backend's WF_MAX_HZ.
const WF_TOP_HZ = 3500;
const TX_MIN_HZ = 200;
const TX_MAX_HZ = 2900;
// Height (px) of the slot-boundary band: tall enough to bake a readable
// HH:MM:SS label into, baked into the canvas pixels themselves (not a
// separate DOM overlay) so the label scrolls down with its own boundary
// line by construction -- no separate scroll-rate tracking/animation needed
// to keep it in sync. Every other row is still exactly 1px.
const WF_BOUNDARY_BAND_HEIGHT = 12;
let wfCanvas: HTMLCanvasElement | null = null;
function registerWfCanvas(c: HTMLCanvasElement | null) {
  wfCanvas = c;
  if (c) c.height = WF_HEIGHT;
}
function drawWaterfallRow(bins: number[], boundary = false) {
  const cv = wfCanvas;
  if (!cv || bins.length === 0) return;
  const cols = bins.length;
  if (cv.width !== cols) cv.width = cols;
  const ctx = cv.getContext("2d");
  if (!ctx) return;
  const h = cv.height;
  const rows = boundary ? WF_BOUNDARY_BAND_HEIGHT : 1;
  // Scroll the existing image down by `rows`, then paint the new band on top.
  ctx.drawImage(cv, 0, 0, cols, h - rows, 0, rows, cols, h - rows);
  const img = ctx.createImageData(cols, 1);
  for (let i = 0; i < cols; i++) {
    const [r, g, b] = colormap(bins[i]);
    img.data[i * 4] = r;
    img.data[i * 4 + 1] = g;
    img.data[i * 4 + 2] = b;
    img.data[i * 4 + 3] = 255;
  }
  // Repeat the new spectrum row to fill the whole band, so a boundary band
  // looks like the waterfall's normal texture, not a blank/black gap.
  for (let row = 0; row < rows; row++) ctx.putImageData(img, 0, row);
  // 15 s UTC cycle grid line on the same fixed clock as the decoder, with
  // the slot's start time baked right into the same band so it travels
  // with the line as the waterfall scrolls.
  if (boundary) {
    ctx.fillStyle = "rgba(255,255,255,0.35)";
    ctx.fillRect(0, 0, cols, 1);
    const label = new Date().toISOString().substring(11, 19);
    ctx.font = "9px monospace";
    const textWidth = ctx.measureText(label).width;
    ctx.fillStyle = "rgba(0,0,0,0.6)";
    ctx.fillRect(0, 1, textWidth + 4, WF_BOUNDARY_BAND_HEIGHT - 1);
    ctx.fillStyle = "#fff";
    ctx.textBaseline = "top";
    ctx.fillText(label, 2, 2);
  }
}

// Olive/yellow label color for inline waterfall callsign labels -- matches
// the Android app's look (distinct from the cyan spectral trace and the red
// TX marker, readable against both).
const SIGNAL_LABEL_COLOR = "#c7c04a";
// How much vertical waterfall history (px) each label spans before it's
// scrolled past -- long enough to stay legible for a few seconds of history,
// short enough that two cycles' labels at a similar frequency don't fully
// overlap.
const SIGNAL_LABEL_HEIGHT = 120;

// Bakes each CQ caller's callsign into the waterfall at its decode frequency,
// written vertically (bottom-to-top) so it reads alongside its own signal
// trace rather than across neighboring ones -- same "paint once, let it
// scroll with the rest of the bitmap" technique drawWaterfallRow's boundary
// time-label already uses, since this canvas is a persistent scrolling
// bitmap, not redrawn from scratch each frame.
function drawSignalLabels(messages: UiMessage[]) {
  const cv = wfCanvas;
  if (!cv || cv.width === 0) return;
  const ctx = cv.getContext("2d");
  if (!ctx) return;
  ctx.font = "11px monospace";
  ctx.fillStyle = SIGNAL_LABEL_COLOR;
  ctx.textBaseline = "bottom";
  const y = Math.min(SIGNAL_LABEL_HEIGHT, cv.height);
  for (const m of messages) {
    if (!m.is_cq) continue;
    const x = Math.round((m.freq_hz / WF_TOP_HZ) * cv.width);
    ctx.save();
    ctx.translate(x + 4, y);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText(`CQ ${m.call_from}`, 0, 0);
    ctx.restore();
  }
}

export default function App() {
  const [tab, setTab] = useState<Tab>("decode");
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [cycle, setCycle] = useState<CycleTick | null>(null);
  const [txState, setTxState] = useState<TxStateEvent | null>(null);
  const [rig, setRig] = useState<RigStatusEvent | null>(null);
  const [clock, setClock] = useState<ClockSyncEvent | null>(null);
  const [decoding, setDecoding] = useState(false);
  const [audio, setAudio] = useState<{ db: number; silent: boolean } | null>(null);
  const [status, setStatus] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  // Whether the "More" controls panel (Tx stage picker + Stop TX, see TxBar)
  // is open -- off by default, matching Android's operate-controls drawer
  // starting collapsed. Toggled from the tab bar's More button rather than
  // shown as a persistent bar, which is what it replaced.
  const [moreOpen, setMoreOpen] = useState(false);
  const [txFreq, setTxFreq] = useState(1500);
  const [txGain, setTxGain] = useState(90); // TX level as a percentage (0–100)
  const [inputGain, setInputGain] = useState(100); // RX input gain as a percentage (0–100)
  const [rigLabel, setRigLabel] = useState(""); // optional display-name override
  const [bands, setBands] = useState<BandInfo[]>([]);
  const [dialHz, setDialHz] = useState<number>(14074000);
  // Calls already worked on the current band (upper-cased), for the Decode
  // list's worked-station strikethrough. Desktop only ever logs "FT8" today
  // (see qso.rs), so that's the only mode checked for now.
  const [workedCalls, setWorkedCalls] = useState<Set<string>>(new Set());
  // Grid squares already worked (any band/mode), for the New Grid filter chip
  // and Hunt's new_grid_first priority.
  const [workedGrids, setWorkedGrids] = useState<Set<string>>(new Set());
  // CQ WPX prefixes already worked (any band/mode, see wpxPrefix.ts), for the
  // New Prefix filter chip.
  const [workedPrefixes, setWorkedPrefixes] = useState<Set<string>>(new Set());
  // DXCC entity prefixes already worked (any band/mode, see dxcc.rs), for
  // the New DXCC filter chip.
  const [workedDxccPrefixes, setWorkedDxccPrefixes] = useState<Set<string>>(new Set());
  // CQ zones already worked (any band/mode), for the New Zone filter chip.
  const [workedCqZones, setWorkedCqZones] = useState<Set<number>>(new Set());
  // US states already worked on the current band/mode (WAS is per band and
  // mode, unlike DXCC/Zone above -- see db.rs's worked_states), for the New
  // State filter chip.
  const [workedStates, setWorkedStates] = useState<Set<string>>(new Set());
  const [myGrid, setMyGrid] = useState("");
  // Last QMX SWR/power meter reading (see qmx_meters.rs); null until a rig
  // that supports the query has reported one, matching Android's chips only
  // appearing once a reading exists.
  const [meter, setMeter] = useState<{ swr_ratio: number | null; power_watts: number | null } | null>(
    null
  );

  // Hunt: auto-answer the best eligible CQ each cycle (see src/hunt.ts).
  // Persisted the same way every other sticky setting on this screen is.
  const [huntEnabled, setHuntEnabled] = useState(false);
  const [huntPriority, setHuntPriorityState] = useState<HuntPriority>("latest");
  const [huntAvoidPileups, setHuntAvoidPileups] = useState(false);
  const [huntMinSnr, setHuntMinSnr] = useState<number | null>(null);

  function refreshWorkedCalls() {
    api.workedCalls("FT8").then((calls) => setWorkedCalls(new Set(calls)));
  }
  // Re-fetch whenever the tuned band changes -- "worked" is always
  // relative to the band currently being operated.
  useEffect(refreshWorkedCalls, [dialHz]);

  function refreshWorkedGrids() {
    api.workedGrids().then((grids) => setWorkedGrids(new Set(grids)));
  }
  useEffect(refreshWorkedGrids, []);

  function refreshWorkedPrefixes() {
    api.workedPrefixes().then((prefixes) => setWorkedPrefixes(new Set(prefixes)));
  }
  useEffect(refreshWorkedPrefixes, []);

  function refreshWorkedDxccPrefixes() {
    api.workedDxccPrefixes().then((prefixes) => setWorkedDxccPrefixes(new Set(prefixes)));
  }
  useEffect(refreshWorkedDxccPrefixes, []);

  function refreshWorkedCqZones() {
    api.workedCqZones().then((zones) => setWorkedCqZones(new Set(zones)));
  }
  useEffect(refreshWorkedCqZones, []);

  function refreshWorkedStates() {
    api.workedStates("FT8").then((states) => setWorkedStates(new Set(states)));
  }
  // Re-fetch whenever the tuned band changes -- WAS is per band/mode, same
  // reason refreshWorkedCalls re-fetches on dialHz.
  useEffect(refreshWorkedStates, [dialHz]);

  // keep latest decoding flag for the toggle without stale closure
  const decodingRef = useRef(decoding);
  decodingRef.current = decoding;

  // handleEvent below is registered once at mount (see the engine-event
  // effect), so any state it reads directly would be frozen at its mount-time
  // value -- these refs are the same "always current" escape hatch
  // decodingRef already uses, kept in sync on every render.
  const workedCallsRef = useRef(workedCalls);
  workedCallsRef.current = workedCalls;
  const workedGridsRef = useRef(workedGrids);
  workedGridsRef.current = workedGrids;
  const myGridRef = useRef(myGrid);
  myGridRef.current = myGrid;
  const huntEnabledRef = useRef(huntEnabled);
  huntEnabledRef.current = huntEnabled;
  const huntPriorityRef = useRef(huntPriority);
  huntPriorityRef.current = huntPriority;
  const huntAvoidPileupsRef = useRef(huntAvoidPileups);
  huntAvoidPileupsRef.current = huntAvoidPileups;
  const huntMinSnrRef = useRef(huntMinSnr);
  huntMinSnrRef.current = huntMinSnr;
  const txStateRef = useRef<TxStateEvent | null>(null);

  // Debounce IPC calls from the TX/RX gain sliders: a drag gesture can fire
  // dozens of onChange events (confirmed live -- "the slider is really
  // touchy"), each one a full round-trip to the Rust backend stacking on top
  // of the waterfall's own already-frequent canvas redraws. That combined
  // load correlates with a real WebKitGTK renderer crash seen live (the
  // WebKitWebProcess disappeared entirely, leaving a blank window, while the
  // Rust backend kept running fine) -- not proven as the sole cause, but a
  // clear, safe mitigation regardless. The slider's own displayed value
  // updates immediately (setTxGain/setInputGain, not debounced) for smooth
  // visual feedback; only the backend call waits for a short pause in
  // dragging. Safe to delay for both: TX gain is only read when the *next*
  // transmission starts (never applied to audio already playing), and RX
  // gain landing ~120ms late has no practical effect either. This does change
  // TX gain from "applied on the spot" to "applied after the drag settles" --
  // deliberate, and invisible unless a slot boundary lands inside that window,
  // in which case the previous level is used for one transmission.
  // Held in refs so the debouncers survive re-renders (a fresh one per render
  // would never accumulate a burst), and cancelled on unmount so a drag still
  // in flight cannot fire IPC after teardown. See src/debounce.ts.
  const debouncedSetTxGain = useRef(
    debounce((pct: number) => api.setTxGain(pct / 100), GAIN_DEBOUNCE_MS)
  ).current;
  const debouncedSetRxGain = useRef(
    debounce((pct: number) => api.setRxGain(pct / 100), GAIN_DEBOUNCE_MS)
  ).current;
  useEffect(
    () => () => {
      debouncedSetTxGain.cancel();
      debouncedSetRxGain.cancel();
    },
    [debouncedSetTxGain, debouncedSetRxGain]
  );

  useEffect(() => {
    // Register the event listener with cancellation-safe cleanup. Without this,
    // React StrictMode's mount/unmount/mount in dev leaks the first listener
    // (its async `listen()` resolves after the first cleanup ran), so every
    // event would be handled twice — duplicating each decode batch.
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onEngineEvent(handleEvent).then((u) => {
      if (cancelled) u();
      else unlisten = u;
      // Pull current rig/TX status now that we're listening. The engine
      // auto-reconnects the saved rig at startup and emits its status before this
      // listener exists, so without this the corner would show "no rig" until the
      // next status change.
      api.refreshStatus();
    });
    api.listBands().then(setBands);
    api.getConfig("dial_hz").then((v) => {
      if (v) setDialHz(parseInt(v, 10));
    });
    api.getConfig("base_freq").then((v) => {
      if (v) setTxFreq(parseInt(v, 10));
    });
    api.getConfig("tx_gain").then((v) => {
      if (v == null || v === "") return;
      const pct = Math.round(parseFloat(v) * 100);
      if (!Number.isNaN(pct)) setTxGain(pct); // keep a persisted 0% (not just falsy-skip)
    });
    api.getConfig("rx_gain").then((v) => {
      if (v == null || v === "") return;
      const pct = Math.round(parseFloat(v) * 100);
      if (!Number.isNaN(pct)) setInputGain(pct);
    });
    api.getConfig("rig_label").then((v) => {
      if (v) setRigLabel(v);
    });
    api.getConfig("my_grid").then((v) => {
      if (v) setMyGrid(v);
    });
    api.getConfig("hunt_enabled").then((v) => setHuntEnabled(v === "true"));
    api.getConfig("hunt_priority").then((v) => setHuntPriorityState(huntPriorityFromConfig(v)));
    api.getConfig("hunt_avoid_pileups").then((v) => setHuntAvoidPileups(v === "true"));
    api.getConfig("hunt_min_snr").then((v) => {
      const n = v == null || v === "" ? NaN : parseInt(v, 10);
      setHuntMinSnr(Number.isNaN(n) ? null : n);
    });
    maybeAutoStartDecode();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  function handleEvent(e: EngineEvent) {
    switch (e.type) {
      case "decoded":
        if (e.data.length) {
          setMessages((prev) => {
            // Safety net: drop any incoming message already present for the same
            // slot (same utc_ms + text), so a stray duplicate event can't double rows.
            const seen = new Set(prev.map((m) => `${m.utc_ms}|${m.text}`));
            const fresh = e.data.filter((m) => !seen.has(`${m.utc_ms}|${m.text}`));
            if (fresh.length === 0) return prev;
            return [...fresh, ...prev].slice(0, MAX_MESSAGES);
          });
          drawSignalLabels(e.data);
          runHunt(e.data);
        }
        break;
      case "cycle":
        setCycle(e.data);
        break;
      case "tx_state":
        txStateRef.current = e.data;
        setTxState(e.data);
        break;
      case "rig_status":
        setRig(e.data);
        break;
      case "clock_sync":
        setClock(e.data);
        break;
      case "waterfall_row":
        drawWaterfallRow(e.data.bins, e.data.boundary);
        break;
      case "input_level":
        setAudio(e.data);
        break;
      case "qso_completed":
        setStatus(`QSO logged: ${e.data.call} ${e.data.rst_sent}/${e.data.rst_rcvd}`);
        refreshWorkedCalls();
        refreshWorkedGrids();
        refreshWorkedPrefixes();
        refreshWorkedDxccPrefixes();
        refreshWorkedCqZones();
        refreshWorkedStates();
        break;
      case "meter":
        setMeter(e.data);
        break;
      case "info":
        setStatus(e.data);
        break;
      case "error":
        setStatus("⚠ " + e.data);
        break;
    }
  }

  // Hunt: from this decode batch's CQ callers (not already worked), pick the
  // best one per the operator's priority/avoid-pileups/min-SNR options (see
  // src/hunt.ts) and answer it -- but never while already in a QSO, so Hunt
  // only ever starts a new one, it doesn't step on an in-progress exchange.
  // Reads every setting through a ref (see the comment above workedCallsRef):
  // this function is defined fresh each render, but the *registered* listener
  // is captured once at mount, so direct state reads here would be frozen at
  // their mount-time (empty/default) values forever.
  function runHunt(batch: UiMessage[]) {
    if (!huntEnabledRef.current) return;
    if (txStateRef.current?.status.active) return;
    const worked = workedCallsRef.current;
    const priority = huntPriorityRef.current;
    const avoidPileups = huntAvoidPileupsRef.current;
    const minSnr = huntMinSnrRef.current;
    const workedGridsNow = workedGridsRef.current;
    const myGridNow = myGridRef.current;

    const eligible = batch.filter((m) => m.is_cq && !worked.has(m.call_from.toUpperCase()));
    if (eligible.length === 0) return;

    const candidates: HuntCandidate[] = eligible.map((m, index) => ({
      index,
      snr: m.snr,
      distanceKm: priority === "farthest" ? maidenheadDistanceKm(myGridNow, m.grid) : null,
      isActivator: priority === "pota_first" && isPotaCq(m.is_cq, m.text),
      isNewGrid:
        priority === "new_grid_first" &&
        m.grid.length >= 4 &&
        !workedGridsNow.has(m.grid.toUpperCase()),
      pileupCalls: avoidPileups ? countRepliesTo(m.call_from, batch) : 0,
    }));

    const chosen = selectHuntCandidate(candidates, priority, avoidPileups, minSnr);
    if (!chosen) return;
    const target = eligible[chosen.index];
    api.answer({ call_from: target.call_from, grid: target.grid, snr: target.snr });
  }

  // Decode starts itself once the operator has done the two things it
  // actually needs -- a callsign and a configured rig -- instead of making
  // "Start decode" a button someone has to remember to press every launch
  // (matching the Android app, which has no such button at all: it just
  // decodes once it has what it needs). Checked once at mount (persisted
  // config from a previous session, via `getConfig`) and again right after
  // Settings saves the station or connects a rig (via `known`, the values
  // just submitted -- `api.setStation`/`selectRig` hand off to the engine
  // over an async channel, so re-reading `getConfig` immediately after would
  // race the write). A no-op once already decoding, so it's safe to call
  // repeatedly.
  async function maybeAutoStartDecode(known?: { call: string; rigBackend: string }) {
    if (decodingRef.current) return;
    const call = known?.call ?? (await api.getConfig("my_call"));
    if (!call) return;
    let backend = known?.rigBackend;
    if (backend == null) {
      const rigJson = await api.getConfig("rig_config");
      backend = "none";
      if (rigJson) {
        try {
          backend = (JSON.parse(rigJson) as { backend?: string }).backend ?? "none";
        } catch {
          // malformed persisted config -- treat as unconfigured, same as "none"
        }
      }
    }
    if (backend === "none") return;
    api.startDecode();
    setDecoding(true);
  }

  function onBand(hz: number) {
    setDialHz(hz);
    api.setBand(hz);
  }

  const visible = messages.filter((m) => {
    switch (filter) {
      case "all": return true;
      case "cq": return m.is_cq;
      case "pota": return isPotaCq(m.is_cq, m.text);
      case "dxcc": return m.is_cq && m.dxcc_prefix != null && !workedDxccPrefixes.has(m.dxcc_prefix);
      case "zone": return m.is_cq && m.cq_zone != null && !workedCqZones.has(m.cq_zone);
      case "state": return m.is_cq && m.state != null && !workedStates.has(m.state);
      case "grid": return m.is_cq && m.grid.length >= 4 && !workedGrids.has(m.grid.toUpperCase());
      case "prefix": {
        if (!m.is_cq) return false;
        const prefix = wpxPrefixOf(m.call_from);
        return prefix != null && !workedPrefixes.has(prefix);
      }
      // Mirrors Android's "Needed" filter: any message (not just CQs) from a
      // callsign not yet worked on the current band -- same "worked" set the
      // Decode list already strikes through with (workedCalls), not a
      // separate QSL-confirmed list (see db.rs's worked_calls doc comment).
      case "needed": return !workedCalls.has(m.call_from.toUpperCase());
      case "tome": return m.to_me;
    }
  });

  function setHuntPriority(p: HuntPriority) {
    setHuntPriorityState(p);
    api.setConfig("hunt_priority", p);
  }

  return (
    <>
      <TopBar
        cycle={cycle}
        bands={bands}
        dialHz={dialHz}
        onBand={onBand}
        rig={rig}
        clock={clock}
        rigLabel={rigLabel}
        txState={txState}
        meter={meter}
        onCq={() => api.startCq()}
        onFreeText={(text) => api.freeText(text)}
        onToggleTune={() => (txState?.tuning ? api.stopTune() : api.startTune())}
        huntEnabled={huntEnabled}
        onToggleHunt={() => {
          const next = !huntEnabled;
          setHuntEnabled(next);
          api.setConfig("hunt_enabled", String(next));
        }}
        onClearDecodes={() => setMessages([])}
        onRefresh={() => api.refreshStatus()}
      />
      <div
        className={
          "content" + (tab === "decode" || tab === "log" || tab === "waterfall" ? " decode" : "")
        }
      >
        {tab === "decode" && (
          <DecodeScreen
            messages={visible}
            filter={filter}
            setFilter={setFilter}
            workedCalls={workedCalls}
            txFreq={txFreq}
            onSelectFreq={(hz) => {
              setTxFreq(hz);
              api.setBaseFreq(hz);
            }}
            onAnswer={(m) => api.answer({ call_from: m.call_from, grid: m.grid, snr: m.snr })}
            inputGain={inputGain}
            onInputGain={(v) => {
              setInputGain(v);
              debouncedSetRxGain(v);
            }}
            txGain={txGain}
            onTxGain={(v) => {
              setTxGain(v);
              debouncedSetTxGain(v);
            }}
            decoding={decoding}
            audio={audio}
          />
        )}
        {tab === "waterfall" && (
          <div className="waterfall-tab">
            <WaterfallView
              txFreq={txFreq}
              onSelectFreq={(hz) => {
                setTxFreq(hz);
                api.setBaseFreq(hz);
              }}
            />
          </div>
        )}
        {tab === "log" && <LogScreen />}
        {tab === "settings" && (
          <SettingsScreen
            onStatus={setStatus}
            clock={clock}
            onSettingsSaved={maybeAutoStartDecode}
            onRigLabel={(v) => {
              setRigLabel(v);
              api.setConfig("rig_label", v);
            }}
            onBandsChanged={setBands}
            huntPriority={huntPriority}
            onHuntPriority={setHuntPriority}
            huntAvoidPileups={huntAvoidPileups}
            onHuntAvoidPileups={(v) => {
              setHuntAvoidPileups(v);
              api.setConfig("hunt_avoid_pileups", String(v));
            }}
            huntMinSnr={huntMinSnr}
            onHuntMinSnr={(v) => {
              setHuntMinSnr(v);
              api.setConfig("hunt_min_snr", v == null ? "" : String(v));
            }}
          />
        )}
        {(tab === "map" || tab === "pota") && (
          <ComingSoonScreen
            title={tab === "map" ? "Map" : "POTA"}
            detail={
              tab === "map"
                ? "A world map of decoded/worked stations, matching the Android app's map, is planned but not built yet."
                : "Parks-on-the-Air park search, activation tracking, and self-spotting, matching the Android app's POTA tab, are planned but not built yet."
            }
          />
        )}
      </div>
      {tab !== "waterfall" && moreOpen && (
        <TxBar
          txState={txState}
          decoding={decoding}
          onStage={(s) => api.setStage(s)}
          onStop={() => api.stopTx()}
        />
      )}
      <div className="status-line">{status}</div>
      <TabBar
        tab={tab}
        onTab={setTab}
        moreOpen={moreOpen}
        onToggleMore={() => setMoreOpen((v) => !v)}
      />
    </>
  );
}

// Honest placeholder for a tab whose real screen isn't built yet (Map, POTA)
// -- says so plainly rather than showing fake content.
function ComingSoonScreen({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="coming-soon">
      <h3>{title}</h3>
      <p className="muted">{detail}</p>
    </div>
  );
}

function TabBar({
  tab,
  onTab,
  moreOpen,
  onToggleMore,
}: {
  tab: Tab;
  onTab: (t: Tab) => void;
  moreOpen: boolean;
  onToggleMore: () => void;
}) {
  const items: { id: Tab; label: string; icon: string }[] = [
    { id: "decode", label: "Decode", icon: "☰" },
    { id: "map", label: "Map", icon: "🌐" },
    { id: "waterfall", label: "Waterfall", icon: "📶" },
    { id: "pota", label: "POTA", icon: "🌲" },
    { id: "log", label: "Logbook", icon: "📘" },
    { id: "settings", label: "Settings", icon: "⚙" },
  ];
  return (
    <div className="tabbar">
      {items.map((it) => (
        <button
          key={it.id}
          className={"tabbar-item" + (tab === it.id ? " active" : "")}
          onClick={() => onTab(it.id)}
        >
          <span className="tabbar-icon">{it.icon}</span>
          <span className="tabbar-label">{it.label}</span>
        </button>
      ))}
      {/* Not a tab -- opens the Tx stage/Stop TX panel over the current
          screen rather than switching screens, matching Android's
          operate-controls drawer toggle ("More controls" in TabBar.kt). */}
      <button
        className={"tabbar-item" + (moreOpen ? " active" : "")}
        onClick={onToggleMore}
        title="More: TX stage picker and Stop TX"
      >
        <span className="tabbar-icon">🎚</span>
        <span className="tabbar-label">More</span>
      </button>
    </div>
  );
}

function TopBar(props: {
  cycle: CycleTick | null;
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  rig: RigStatusEvent | null;
  clock: ClockSyncEvent | null;
  rigLabel: string;
  txState: TxStateEvent | null;
  meter: { swr_ratio: number | null; power_watts: number | null } | null;
  onCq: () => void;
  onFreeText: (text: string) => void;
  onToggleTune: () => void;
  huntEnabled: boolean;
  onToggleHunt: () => void;
  onClearDecodes: () => void;
  onRefresh: () => void;
}) {
  const {
    cycle, bands, dialHz, onBand, rig, clock, rigLabel, txState, meter,
    onCq, onFreeText, onToggleTune, huntEnabled, onToggleHunt, onClearDecodes, onRefresh,
  } = props;
  const [cqMoreOpen, setCqMoreOpen] = useState(false);
  const [freeTextDraft, setFreeTextDraft] = useState("");

  const band = bands.find((b) => b.dial_hz === dialHz);
  const secondsLeft = cycle ? Math.max(0, Math.ceil(15 - cycle.ms_into_cycle / 1000)) : null;
  const pct = cycle ? (cycle.ms_into_cycle / 15000) * 100 : 0;
  const transmitting = txState?.transmitting ?? false;
  const tuning = txState?.tuning ?? false;
  // Which half of the 15s cycle this slot falls in -- purely informational
  // (desktop has no manual slot-parity override), matching WSJT-X/Android's
  // "Odd"/"Even" slot readout.
  const slotParity = cycle ? (cycle.sequential % 2 === 0 ? "Even" : "Odd") : null;

  return (
    <div className="topbar-android">
      <div className="topbar-row">
        <div className="band-picker">
          <span className="band-picker-label">{band ? band.name : "--"} · FT8</span>
          <span className="chevron">⌄</span>
          <select
            className="band-picker-select"
            aria-label="Band"
            value={dialHz}
            onChange={(e) => onBand(parseInt(e.target.value, 10))}
          >
            {bands.map((b) => (
              <option key={b.dial_hz} value={b.dial_hz}>
                {b.custom ? "★ " : ""}{b.name} · {(b.dial_hz / 1e6).toFixed(3)}
              </option>
            ))}
          </select>
        </div>

        <div className="cq-pill-wrap">
          <button className="pill pill-cq" onClick={onCq} disabled={tuning}>
            <span className="pill-label-2line">Call<br />CQ</span>
          </button>
          <button
            className="pill-more-badge"
            title="More: send a free-text message"
            onClick={() => setCqMoreOpen((v) => !v)}
          >
            MORE
          </button>
          {cqMoreOpen && (
            <div className="cq-more-popover">
              <label>Free-text message</label>
              <input
                value={freeTextDraft}
                onChange={(e) => setFreeTextDraft(e.target.value.toUpperCase())}
                placeholder="FIELD DAY K1ABC FN42"
                maxLength={13}
              />
              <div className="row">
                <button
                  className="primary"
                  onClick={() => {
                    if (freeTextDraft.trim()) onFreeText(freeTextDraft.trim());
                    setCqMoreOpen(false);
                  }}
                >
                  Send
                </button>
                <button onClick={() => setCqMoreOpen(false)}>Close</button>
              </div>
            </div>
          )}
        </div>

        <button
          className={"pill pill-dark" + (huntEnabled ? " pill-hunt-active" : "")}
          onClick={onToggleHunt}
          title="Hunt: auto-answer the best eligible CQ each cycle (priority/options in Settings)"
        >
          Hunt
        </button>

        <span className="rx-tx-indicator">
          <span className={"rx-tx-dot" + (transmitting ? " tx" : "")} />
          <span className="rx-tx-main">
            <span className="rx-tx-label">{transmitting ? "TX" : "RX"}</span>
            {!transmitting && secondsLeft != null && (
              <span className="rx-tx-sub">TX in {secondsLeft}s</span>
            )}
          </span>
        </span>

        <span
          className={"pill pill-dark pill-static" + (rig?.connected ? "" : " pill-disconnected")}
          title={rig?.connected ? `${rigLabel || rig.model}${rig.ptt ? " · PTT" : ""}` : "No rig connected"}
        >
          <span className={"status-dot" + (rig?.connected ? " ok" : "")} />
          <span className="pill-label-2line">Radio<br />{rig?.connected ? "linked" : "offline"}</span>
        </span>

        <button
          className={"pill pill-dark" + (tuning ? " pill-tune-active" : "")}
          onClick={onToggleTune}
          disabled={!tuning && transmitting}
          title="Transmit a steady carrier to tune the antenna, without calling CQ"
        >
          TUNE
        </button>

        {slotParity && <span className="pill pill-dark pill-static">{slotParity}</span>}

        {/* QMX SWR/power meter chips, matching Android's CompactSwrChip/
            CompactPowerChip -- only placed once a reading has actually come
            back (see engine.rs's poll_meters, gated on an in-flight TX), not
            a permanent zeroed placeholder. */}
        {meter?.swr_ratio != null && (
          <span
            className="meter-chip"
            title={`SWR ${formatSwrRatio(meter.swr_ratio)} to 1`}
            style={{
              color:
                swrBand(meter.swr_ratio) === "good"
                  ? "var(--ok)"
                  : swrBand(meter.swr_ratio) === "caution"
                  ? "var(--yellow)"
                  : "var(--tx)",
            }}
          >
            <span className="meter-chip-label">SWR</span>
            <span className="meter-chip-value">{formatSwrRatio(meter.swr_ratio)}</span>
          </span>
        )}
        {meter?.power_watts != null && (
          <span className="meter-chip" title={`Output power ${formatPowerWatts(meter.power_watts)}`}>
            <span className="meter-chip-label">PWR</span>
            <span className="meter-chip-value">{formatPowerWatts(meter.power_watts)}</span>
          </span>
        )}

        <div className="spacer" />

        <span className="topbar-clock">
          {cycle ? new Date(cycle.utc_ms).toISOString().substring(11, 19) : "--:--:--"}
        </span>
        <button className="icon-button" title="Refresh rig/engine status" onClick={onRefresh}>⟳</button>
        <button className="icon-button" title="Clear the decode list" onClick={onClearDecodes}>🗑</button>
      </div>

      <div className="topbar-row sync-row">
        <ClockSyncBadge clock={clock} />
        <div className="cycle-bar">
          <div style={{ width: `${pct}%` }} />
        </div>
        <span className="seq muted">{secondsLeft != null ? `${secondsLeft}s` : "--"}</span>
      </div>
    </div>
  );
}

// RX input-level indicator. Shows the captured level in dBFS while decoding, and
// a clear "no audio" warning when the input is silent — the case where the source
// (e.g. a DAX/soundcard channel) isn't actually routed and the waterfall is blank.
function AudioBadge({
  decoding,
  audio,
}: {
  decoding: boolean;
  audio: { db: number; silent: boolean } | null;
}) {
  if (!decoding) return null;
  if (!audio) {
    return <span className="muted" title="Waiting for the first input-level reading">audio …</span>;
  }
  if (audio.silent) {
    return (
      <span
        style={{ color: "var(--tx)", fontVariantNumeric: "tabular-nums" }}
        title="No audio is reaching the app. Check that the selected input device is the right one and that its source is routed (e.g. SmartSDR slice → DAX RX channel, soundcard input not muted)."
      >
        ⚠ no audio
      </span>
    );
  }
  return (
    <span
      className="muted"
      style={{ fontVariantNumeric: "tabular-nums" }}
      title="RX input level (dBFS) of the captured audio"
    >
      audio {audio.db.toFixed(0)} dB
    </span>
  );
}

// Format an NTP offset (ms) as a signed seconds string, e.g. "+0.12s".
function fmtOffset(ms: number): string {
  const s = ms / 1000;
  return `${s >= 0 ? "+" : ""}${s.toFixed(2)}s`;
}

// Small clock-health indicator next to the UTC clock. Green when the offset is
// tight enough for clean FT8 (<0.5 s), amber when it's getting risky, grey until
// the first sync lands.
function ClockSyncBadge({ clock }: { clock: ClockSyncEvent | null }) {
  if (!clock || !clock.synced) {
    return <span className="muted" title="Waiting for NTP time sync">sync …</span>;
  }
  const mag = Math.abs(clock.offset_ms);
  const color = mag < 500 ? "var(--ok)" : mag < 1500 ? "var(--tome)" : "var(--tx)";
  return (
    <span
      style={{ color, fontVariantNumeric: "tabular-nums" }}
      title={`Clock offset from NTP: ${clock.offset_ms} ms (applied to decode timing, system clock untouched)`}
    >
      sync {fmtOffset(clock.offset_ms)}
    </span>
  );
}

// Frequency scale above the waterfall: a tick + label every WF_TICK_STEP_HZ
// (0, 500, 1000, ... up to WF_TOP_HZ), positioned by the same hz/WF_TOP_HZ
// fraction the waterfall canvas and the TX marker already use, so they stay
// aligned. The first/last ticks are pulled in from the edge instead of
// centered, so "0" and the top label don't overflow the container.
function WaterfallScale() {
  const ticks: number[] = [];
  for (let hz = 0; hz <= WF_TOP_HZ; hz += WF_TICK_STEP_HZ) ticks.push(hz);
  return (
    <div style={{ position: "relative", height: 14, marginBottom: 2 }}>
      {ticks.map((hz) => {
        const pct = (hz / WF_TOP_HZ) * 100;
        const transform = hz === 0 ? "none" : hz === WF_TOP_HZ ? "translateX(-100%)" : "translateX(-50%)";
        return (
          <span
            key={hz}
            style={{
              position: "absolute", left: `${pct}%`, transform,
              color: "#fff", fontSize: 10, fontVariantNumeric: "tabular-nums",
            }}
          >
            {hz}
          </span>
        );
      })}
    </div>
  );
}

function WaterfallView({
  txFreq,
  onSelectFreq,
}: {
  txFreq: number;
  onSelectFreq: (hz: number) => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    registerWfCanvas(ref.current);
    return () => registerWfCanvas(null);
  }, []);

  function handleClick(e: MouseEvent<HTMLDivElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    const hz = Math.round(frac * WF_TOP_HZ);
    onSelectFreq(Math.min(TX_MAX_HZ, Math.max(TX_MIN_HZ, hz)));
  }

  const leftPct = (txFreq / WF_TOP_HZ) * 100;

  return (
    <div
      // Fills its parent (`.decode-waterfall`, itself 50% of the window --
      // see DecodeScreen's left/right split), both horizontally and now
      // vertically too: the canvas flex-grows to the full panel height
      // (its backing pixel buffer stays WF_HEIGHT rows -- only the CSS
      // display size changes, so the canvas just scales up like any other
      // image). The scale ruler and the TX marker below are positioned as
      // percentages of this same box, so they stay aligned at any size. The
      // slot-time label is baked into the canvas itself (drawWaterfallRow),
      // not a DOM overlay here -- it needs to scroll down with its own
      // boundary line.
      style={{
        position: "relative", width: "100%", height: "100%",
        display: "flex", flexDirection: "column",
        marginBottom: 8, cursor: "crosshair",
      }}
      onClick={handleClick}
      title="Click to set TX frequency"
    >
      <WaterfallScale />
      <canvas
        ref={ref}
        style={{
          width: "100%",
          flex: "1 1 auto",
          minHeight: 0,
          imageRendering: "pixelated",
          background: "#000",
          borderRadius: 6,
          display: "block",
        }}
      />
      {/* TX frequency marker */}
      <div
        style={{
          position: "absolute",
          top: 0,
          bottom: 0,
          left: `${leftPct}%`,
          width: 2,
          background: "#ff3b30",
          pointerEvents: "none",
        }}
      />
      <div
        style={{
          position: "absolute",
          top: 2,
          left: `calc(${leftPct}% + 4px)`,
          color: "#ff8f87",
          fontSize: 11,
          fontVariantNumeric: "tabular-nums",
          pointerEvents: "none",
          textShadow: "0 0 3px #000",
        }}
      >
        TX {txFreq} Hz
      </div>
    </div>
  );
}

// Map a 0..255 magnitude to a dark-blue -> cyan -> yellow -> white ramp.
function colormap(v: number): [number, number, number] {
  const t = v / 255;
  if (t < 0.25) return [0, 0, Math.round(60 + t * 4 * 140)];
  if (t < 0.5) return [0, Math.round((t - 0.25) * 4 * 200), 200];
  if (t < 0.75) return [Math.round((t - 0.5) * 4 * 230), 230, Math.round(200 - (t - 0.5) * 4 * 200)];
  return [255, Math.round(230 + (t - 0.75) * 4 * 25), Math.round((t - 0.75) * 4 * 200)];
}

// Same order as Android's DecodeScreen.kt filterOptions.
const DECODE_FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "cq", label: "CQ Calls" },
  { id: "pota", label: "CQ POTA" },
  { id: "dxcc", label: "New DXCC" },
  { id: "zone", label: "New Zone" },
  { id: "state", label: "New State" },
  { id: "grid", label: "New Grid" },
  { id: "prefix", label: "New Prefix" },
  { id: "needed", label: "Needed" },
  { id: "tome", label: "For Me" },
];

function DecodeScreen(props: {
  messages: UiMessage[];
  filter: Filter;
  setFilter: (f: Filter) => void;
  workedCalls: Set<string>;
  txFreq: number;
  onSelectFreq: (hz: number) => void;
  onAnswer: (m: UiMessage) => void;
  inputGain: number;
  onInputGain: (v: number) => void;
  txGain: number;
  onTxGain: (v: number) => void;
  decoding: boolean;
  audio: { db: number; silent: boolean } | null;
}) {
  const {
    messages, filter, setFilter, workedCalls, txFreq, onSelectFreq, onAnswer,
    inputGain, onInputGain, txGain, onTxGain, decoding, audio,
  } = props;

  // "065815 UTC · 16 stations" header, matching the Android app: the current
  // (topmost) decode period's timestamp and how many distinct callers are in
  // the visible (filtered) list.
  const stationCount = new Set(messages.map((m) => m.call_from.toUpperCase())).size;
  const latestUtc =
    messages.length > 0
      ? new Date(messages[0].utc_ms).toISOString().substring(11, 19).replace(/:/g, "")
      : null;

  return (
    <>
      {/* Full-width, above the split: keeps the list and the waterfall
          starting at the same height below it. */}
      <div className="chips">
        {DECODE_FILTERS.map((f) => (
          <div
            key={f.id}
            className={"chip" + (filter === f.id ? " active" : "")}
            onClick={() => setFilter(f.id)}
          >
            {f.label}
          </div>
        ))}
        {/* Real controls the Android reference screen doesn't need (it has no
            drive sliders) but desktop does -- kept here, next to what they
            affect, rather than invented away. Decode itself has no manual
            start/stop control here, matching Android: it starts on its own
            once a callsign + rig are configured (see maybeAutoStartDecode
            in App()). */}
        <div className="decode-controls" style={{ marginLeft: "auto" }}>
          <span className="tx-level" title="RX input gain — some bands are noisier than others">
            <span className="muted">RX</span>
            <input
              type="range"
              aria-label="RX input gain, percent"
              min={0}
              max={100}
              step={1}
              value={inputGain}
              onChange={(e) => onInputGain(parseInt(e.target.value, 10))}
              style={{ width: 70 }}
            />
            <span className="muted" style={{ fontVariantNumeric: "tabular-nums", minWidth: 30, textAlign: "right" }}>
              {inputGain}%
            </span>
          </span>
          <span className="tx-level" title="TX output level (drive) — lower until ALC stops pinning">
            <span className="muted">TX</span>
            <input
              type="range"
              aria-label="TX output level (drive), percent"
              min={0}
              max={100}
              step={1}
              value={txGain}
              onChange={(e) => onTxGain(parseInt(e.target.value, 10))}
              style={{ width: 70 }}
            />
            <span className="muted" style={{ fontVariantNumeric: "tabular-nums", minWidth: 30, textAlign: "right" }}>
              {txGain}%
            </span>
          </span>
        </div>
      </div>
      {latestUtc && (
        <div className="decode-list-header muted">
          {latestUtc} UTC · {stationCount} station{stationCount === 1 ? "" : "s"}
        </div>
      )}
      <div className="decode-split">
        <div className="decode-detail">
          <div className="decode-list">
            {messages.map((m, i) => {
              // i === 0 also starts a new block: messages are prepended newest-first,
              // so the current decode period is always at the top of the list, and
              // Android always shows that period's header there too.
              const newBlock = i === 0 || messages[i - 1].utc_ms !== m.utc_ms;
              const worked = workedCalls.has(m.call_from.toUpperCase());
              return (
                <Fragment key={i}>
                  {newBlock && (
                    <div className="time-sep">
                      {new Date(m.utc_ms).toISOString().substring(11, 19)} UTC
                    </div>
                  )}
                  <div
                    className={
                      "decode-row" +
                      (m.is_cq ? " cq" : "") +
                      (m.to_me ? " tome" : "") +
                      (m.is_cq || m.to_me ? " clickable" : "")
                    }
                    onClick={() => (m.is_cq || m.to_me) && onAnswer(m)}
                    title={m.is_cq || m.to_me ? "Click to answer" : ""}
                  >
                    <span className={"decode-row-msg" + (worked ? " worked" : "")}>{m.text}</span>
                    <span className="decode-row-snr">{m.snr}</span>
                    <span className="decode-row-freq">{Math.round(m.freq_hz)}</span>
                    {/* DT: a signed offset, highlighted once it's large enough to be
                        worth the operator's attention (decode timing drifting). */}
                    <span className={"decode-row-dt" + (Math.abs(m.time_sec) > 1 ? " warn" : "")}>
                      {m.time_sec >= 0 ? "+" : ""}
                      {m.time_sec.toFixed(1)}
                    </span>
                    {m.is_cq && (
                      <span className="cq-badge">
                        <span className="cq-badge-dot" />
                        CQ
                      </span>
                    )}
                  </div>
                </Fragment>
              );
            })}
            {messages.length === 0 && (
              <div className="decode-row-empty muted">
                No decodes yet — set your callsign and rig in Settings; decoding starts on its own.
              </div>
            )}
          </div>
        </div>
        <div className="decode-waterfall">
          <WaterfallView txFreq={txFreq} onSelectFreq={onSelectFreq} />
          <WaterfallStatusStrip txFreq={txFreq} decoding={decoding} audio={audio} />
        </div>
      </div>
    </>
  );
}

// Bottom info strip under the waterfall. Only shows data desktop actually
// has (current TX/cursor frequency, live/idle state, the real RX input
// level via AudioBadge) -- no Width/NR/MSG controls or a fabricated signal-
// quality percentage, since desktop has no noise-reduction/bandwidth-filter
// DSP or quality score to back them.
function WaterfallStatusStrip({
  txFreq,
  decoding,
  audio,
}: {
  txFreq: number;
  decoding: boolean;
  audio: { db: number; silent: boolean } | null;
}) {
  return (
    <div className="wf-status-strip">
      <AudioBadge decoding={decoding} audio={audio} />
      <div className="spacer" />
      <span className="wf-status-freq">{txFreq} Hz</span>
      <span className={"wf-status-live" + (decoding ? "" : " off")}>{decoding ? "LIVE" : "IDLE"}</span>
    </div>
  );
}

function LogScreen() {
  const [rows, setRows] = useState<QsoRecord[]>([]);
  const [count, setCount] = useState(0);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ConfirmFilter>("all");
  // ADIF-export date-range pickers (empty = open bound).
  const [rangeStart, setRangeStart] = useState("");
  const [rangeEnd, setRangeEnd] = useState("");
  // Edit-QSO dialog target: undefined = closed, null = new QSO, record = edit.
  const [editing, setEditing] = useState<QsoRecord | null | undefined>(undefined);
  // Import completion dialog state: exactly one of these is set once an
  // import finishes, matching the Android app's "Import Complete"/error
  // popup (dismissed with its own "Done" button, not auto-hidden).
  const [importResult, setImportResult] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importProgress, setImportProgress] = useState<ImportProgress | null>(null);
  const importInputRef = useRef<HTMLInputElement | null>(null);
  // Table sort: newest-first (Date, descending) by default.
  const [sortColumn, setSortColumn] = useState<number>(QSO_COL_DATE);
  const [sortAscending, setSortAscending] = useState(false);

  function onSort(col: number) {
    if (col === sortColumn) {
      setSortAscending((a) => !a);
    } else {
      setSortColumn(col);
      setSortAscending(false);
    }
  }

  // Row virtualization: confirmed necessary on a real ~54,500-row log, where
  // rendering every row as a real <tr> pinned the webview's renderer at 100%
  // CPU indefinitely. Only [startIndex, endIndex) are mounted; two spacer
  // rows stand in for the rest so the scrollbar still reflects the true
  // total. ROW_HEIGHT must match the CSS-enforced `.qso-table tbody tr`
  // height exactly, or the windowing math and the real scroll height disagree.
  const ROW_HEIGHT = 26;
  const OVERSCAN = 8;
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(400);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setViewportHeight(el.clientHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Memoized: with the fetch cap removed, `rows` can be many thousands of
  // QSOs, and sorting that on every render -- including the rapid re-renders
  // from import-progress ticking every 200 records -- made the whole UI feel
  // unresponsive (confirmed: clicking the Import button appeared to do
  // nothing while a big unrelated re-sort was in flight). Only recompute when
  // the data or the sort choice actually changes.
  const sortedRows = useMemo(
    () => sortQsoTable(rows, sortColumn, sortAscending),
    [rows, sortColumn, sortAscending],
  );
  const virtualWindow = computeVirtualWindow(
    sortedRows.length, ROW_HEIGHT, scrollTop, viewportHeight, OVERSCAN,
  );
  const visibleRows = sortedRows.slice(virtualWindow.startIndex, virtualWindow.endIndex);

  function refresh() {
    // No cap: the table sorts client-side over whatever's fetched, so a
    // capped page only ever sorted that arbitrary window (insertion order,
    // not chronological) instead of the whole matching log -- confirmed by
    // the user on a multi-thousand-row CQRLOG import.
    api.searchLog(search, filter, LOG_FETCH_ALL, 0).then(setRows);
    api.logCount().then(setCount);
  }
  // Re-query whenever the live search box or the confirmation filter changes.
  useEffect(refresh, [search, filter]);

  async function exportAdif() {
    const { start, end } = buildExportArgs(rangeStart, rangeEnd);
    const adif = await api.exportAdif(start, end);
    const blob = new Blob([adif], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "ft8af_log.adi";
    a.click();
    URL.revokeObjectURL(url);
  }

  async function importAdifFile(file: File) {
    // Set busy state synchronously, before the first await -- otherwise the
    // button and progress popup stay unchanged (looking like the click did
    // nothing) for however long reading the file and the IPC round-trip
    // take to even start.
    setImportResult(null);
    setImportError(null);
    setImportProgress({ processed: 0, total: 0 });
    const text = await file.text();
    const unlisten = await onImportProgress(setImportProgress);
    try {
      setImportResult(await api.importAdif(text));
    } catch (err) {
      setImportError(String(err));
    } finally {
      unlisten();
      setImportProgress(null);
    }
    refresh();
  }

  function onImportFileChosen(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-picking the same file next time
    if (file) importAdifFile(file);
  }

  async function remove(id?: number | null) {
    if (id == null) return;
    await api.deleteQso(id);
    refresh();
  }

  async function saveEdit(record: QsoRecord) {
    await api.saveQso(record);
    setEditing(undefined);
    refresh();
  }

  return (
    <>
      <div className="logbar">
        <strong>{qsoCountLabel(count, rows.length, isLogFiltered(search, filter))}</strong>{" "}
        <span className="muted">QSOs</span>
        <input
          className="search"
          placeholder="Search callsign…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select value={filter} onChange={(e) => setFilter(e.target.value as ConfirmFilter)}>
          <option value="all">All</option>
          <option value="confirmed">Confirmed (QSL)</option>
          <option value="unconfirmed">Unconfirmed</option>
        </select>
        <div className="spacer" style={{ flex: 1 }} />
        <button onClick={() => setEditing(null)}>+ Add QSO</button>
        <button onClick={refresh}>Refresh</button>
      </div>
      <div className="logbar">
        <span className="muted">Export range:</span>
        <input
          className="date"
          type="date"
          value={rangeStart}
          onChange={(e) => setRangeStart(e.target.value)}
          aria-label="Export start date"
        />
        <span className="muted">→</span>
        <input
          className="date"
          type="date"
          value={rangeEnd}
          onChange={(e) => setRangeEnd(e.target.value)}
          aria-label="Export end date"
        />
        <button className="primary" onClick={exportAdif}>Export ADIF</button>
        <span className="muted">·</span>
        <input
          ref={importInputRef}
          type="file"
          accept=".adi,.adif,.txt,text/plain"
          style={{ display: "none" }}
          onChange={onImportFileChosen}
        />
        <button onClick={() => importInputRef.current?.click()} disabled={!!importProgress}>
          Import ADIF…
        </button>
      </div>
      <div
        className="decode-list"
        ref={scrollRef}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
      <table className="qso-table">
        <thead>
          <tr>
            {QSO_TABLE_COLUMNS.map(({ col, label }) => {
              const sorted = col === sortColumn;
              return (
                <th
                  key={col}
                  className={sorted ? "sortable sorted" : "sortable"}
                  onClick={() => onSort(col)}
                >
                  {label}
                  {sorted ? (sortAscending ? " ▲" : " ▼") : ""}
                </th>
              );
            })}
            <th></th>
          </tr>
        </thead>
        <tbody>
          {virtualWindow.topSpacerPx > 0 && (
            <tr aria-hidden style={{ height: virtualWindow.topSpacerPx }}>
              <td colSpan={QSO_TABLE_COLUMNS.length + 1} style={{ border: "none", padding: 0 }} />
            </tr>
          )}
          {visibleRows.map((r) => {
            const status = qsoStatusLabel(r);
            return (
              <tr key={r.id} className="clickable">
                {/* Confirmation shows via the Status pill and QSL marks only
                    (matching Android) -- the row itself is never tinted. */}
                <td onClick={() => setEditing(r)}>{adifToDateInput(r.qso_date)}</td>
                <td onClick={() => setEditing(r)}>{formatHhmm(r.time_on)}</td>
                <td onClick={() => setEditing(r)} className="call-cell">{r.call}</td>
                <td onClick={() => setEditing(r)}>{r.gridsquare}</td>
                <td onClick={() => setEditing(r)}>{logbookPlace(r) ?? ""}</td>
                <td onClick={() => setEditing(r)}>{r.freq}</td>
                <td onClick={() => setEditing(r)}>{r.mode}</td>
                <td onClick={() => setEditing(r)}>{displayReport(r.rst_sent)}</td>
                <td onClick={() => setEditing(r)}>{displayReport(r.rst_rcvd)}</td>
                <td onClick={() => setEditing(r)}>
                  <div className="status-cell">
                    {status && <span className="status-pill conf">{status}</span>}
                  </div>
                </td>
                <td onClick={() => setEditing(r)}>
                  <div className="qsl-cell">
                    {qslMarks(r).map(({ service, mark }) => (
                      <span
                        key={service}
                        className={
                          mark === "✓" ? "qsl-mark rcvd" : mark === "↑" ? "qsl-mark sent" : "qsl-mark none"
                        }
                      >
                        {service}{mark}
                      </span>
                    ))}
                  </div>
                </td>
                <td>
                  <button onClick={() => remove(r.id)}>✕</button>
                </td>
              </tr>
            );
          })}
          {virtualWindow.bottomSpacerPx > 0 && (
            <tr aria-hidden style={{ height: virtualWindow.bottomSpacerPx }}>
              <td colSpan={QSO_TABLE_COLUMNS.length + 1} style={{ border: "none", padding: 0 }} />
            </tr>
          )}
          {sortedRows.length === 0 && (
            <tr>
              <td colSpan={QSO_TABLE_COLUMNS.length + 1} className="muted">No QSOs logged yet.</td>
            </tr>
          )}
        </tbody>
      </table>
      </div>
      {editing !== undefined && (
        <EditQsoDialog
          record={editing}
          onCancel={() => setEditing(undefined)}
          onSave={saveEdit}
        />
      )}
      {importProgress && <ImportProgressDialog progress={importProgress} />}
      {importResult && (
        <ImportCompleteDialog summary={importResult} onDone={() => setImportResult(null)} />
      )}
      {importError && (
        <ImportErrorDialog message={importError} onDone={() => setImportError(null)} />
      )}
    </>
  );
}

// Modal import-progress popup, matching the Android app's "Importing ADIF
// log..." dialog (title, progress bar, "N of M records", a working
// indicator) -- the previous inline text next to the button was too easy to
// miss, per the user's reference screenshot of Android's dialog.
function ImportProgressDialog(props: { progress: ImportProgress }) {
  const { processed, total } = props.progress;
  const pct = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0;
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>Importing ADIF log…</h3>
        <div className="bar">
          <div style={{ width: `${pct}%` }} />
        </div>
        <div className="count">
          {total > 0 ? `${processed} of ${total} records` : "Reading file…"}
        </div>
        <div className="working">Working…</div>
      </div>
    </div>
  );
}

// Matches the Android app's "Import Complete" popup exactly: Added/Updated/
// Skipped as three separate lines, dismissed with its own "Done" button
// rather than auto-hiding (per the user's reference screenshot).
function ImportCompleteDialog(props: { summary: ImportSummary; onDone: () => void }) {
  const { summary, onDone } = props;
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>Import complete</h3>
        <div className="import-summary">
          <div>Added: {summary.imported} new QSOs</div>
          <div>Updated: {summary.duplicates} existing QSOs</div>
          <div>Skipped: {summary.skipped} invalid records</div>
        </div>
        <button className="primary import-done" onClick={onDone}>Done</button>
      </div>
    </div>
  );
}

function ImportErrorDialog(props: { message: string; onDone: () => void }) {
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>Import failed</h3>
        <div className="import-summary error">{props.message}</div>
        <button className="primary import-done" onClick={props.onDone}>Done</button>
      </div>
    </div>
  );
}

// ADIF fields shown in the edit dialog, in two-column layout order.
const EDIT_FIELDS: { key: keyof QsoEditForm; label: string }[] = [
  { key: "call", label: "Call" },
  { key: "gridsquare", label: "Grid" },
  { key: "mode", label: "Mode" },
  { key: "band", label: "Band" },
  { key: "freq", label: "Freq (MHz)" },
  { key: "rst_sent", label: "RST sent" },
  { key: "rst_rcvd", label: "RST rcvd" },
  { key: "qso_date", label: "Date on (YYYYMMDD)" },
  { key: "time_on", label: "Time on (HHMMSS)" },
  { key: "qso_date_off", label: "Date off" },
  { key: "time_off", label: "Time off" },
  { key: "station_callsign", label: "My call" },
  { key: "my_gridsquare", label: "My grid" },
  { key: "state", label: "State" },
  { key: "country", label: "Country" },
  { key: "comment", label: "Comment" },
];

/** QSL/upload checkboxes shown below EDIT_FIELDS, matching the Android
 *  Logbook's QSL column services (L/E/P/Q -- see `logbook.ts`'s `qslMarks`). */
const QSL_CHECKBOXES: { key: keyof QsoEditForm; label: string }[] = [
  { key: "confirmed", label: "Paper QSL received" },
  { key: "qsl_sent", label: "Paper QSL sent" },
  { key: "lotw_rcvd", label: "LoTW received" },
  { key: "lotw_sent", label: "LoTW sent" },
  { key: "eqsl_rcvd", label: "eQSL received" },
  { key: "eqsl_sent", label: "eQSL sent" },
  { key: "qrz_qsl", label: "Confirmed on QRZ.com" },
];

function EditQsoDialog(props: {
  record: QsoRecord | null;
  onCancel: () => void;
  onSave: (r: QsoRecord) => void | Promise<void>;
}) {
  const { record, onCancel, onSave } = props;
  const [form, setForm] = useState<QsoEditForm>(() => formFromQso(record));
  const [saveError, setSaveError] = useState<string | null>(null);

  function set<K extends keyof QsoEditForm>(key: K, value: QsoEditForm[K]) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  async function save() {
    // onSave may be async (it calls the save_qso IPC); await it so a failed save
    // surfaces here instead of becoming an unhandled promise rejection.
    try {
      setSaveError(null);
      await onSave(buildQsoRecord(form, record?.id));
    } catch (e) {
      setSaveError(String(e));
    }
  }

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{record?.id ? "Edit QSO" : "Add QSO"}</h3>
        <div className="grid2">
          {EDIT_FIELDS.map((f) => (
            <div className="field" key={f.key}>
              <label>{f.label}</label>
              <input
                value={form[f.key] as string}
                onChange={(e) => set(f.key, e.target.value as never)}
              />
            </div>
          ))}
        </div>
        <div className="qsl-checks">
          {QSL_CHECKBOXES.map((c) => (
            <label className="check" key={c.key}>
              <input
                type="checkbox"
                checked={form[c.key] as boolean}
                onChange={(e) => set(c.key, e.target.checked as never)}
              />
              {c.label}
            </label>
          ))}
        </div>
        {saveError && <div className="error" role="alert">Save failed: {saveError}</div>}
        <div className="actions">
          <button onClick={onCancel}>Cancel</button>
          <button className="primary" onClick={save}>Save</button>
        </div>
      </div>
    </div>
  );
}

// Waterfall FFT developer-knob option lists (issue #428). Single source of
// truth for both the <select> options and validation of persisted config —
// raw config strings are untrusted (hand-edited DB, older builds), so
// anything outside these lists falls back to the defaults.
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

// Add/edit/delete operator-defined dial frequencies (issue #470). Custom dials
// persist in the config store and are merged into the band picker (shown with a
// ★). Editing is re-adding at the same dial frequency; the backend upserts.
function CustomBandsPanel(props: {
  onStatus: (s: string) => void;
  onBandsChanged: (bands: BandInfo[]) => void;
}) {
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
      // The Rust command rejects bad input (non-numeric / out of range) with a
      // human-readable message; show it inline rather than as a toast.
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
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. 3B9 DX"
              style={{ width: 120 }}
            />
          </div>
          <div className="field">
            <label>Dial frequency (Hz)</label>
            <input
              value={freq}
              onChange={(e) => setFreq(e.target.value)}
              placeholder="e.g. 14090000"
              style={{ width: 130 }}
            />
          </div>
          <button className="primary" onClick={add}>Add / update</button>
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
                  <td style={{ fontVariantNumeric: "tabular-nums" }}>
                    {(b.dial_hz / 1e6).toFixed(6)}
                  </td>
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

function SettingsScreen(props: {
  onStatus: (s: string) => void;
  clock: ClockSyncEvent | null;
  /** Re-checks whether decode can now auto-start (callsign + rig configured)
   *  -- called right after saving the station or connecting a rig, with the
   *  values just submitted (not yet reflected in persisted config), so
   *  finishing setup starts decoding immediately. See maybeAutoStartDecode. */
  onSettingsSaved: (known: { call: string; rigBackend: string }) => void;
  onRigLabel: (v: string) => void;
  onBandsChanged: (bands: BandInfo[]) => void;
  huntPriority: HuntPriority;
  onHuntPriority: (p: HuntPriority) => void;
  huntAvoidPileups: boolean;
  onHuntAvoidPileups: (v: boolean) => void;
  huntMinSnr: number | null;
  onHuntMinSnr: (v: number | null) => void;
}) {
  const {
    onStatus, clock, onSettingsSaved, onRigLabel, onBandsChanged,
    huntPriority, onHuntPriority, huntAvoidPileups, onHuntAvoidPileups, huntMinSnr, onHuntMinSnr,
  } = props;
  const [call, setCall] = useState("");
  const [rigLabel, setRigLabel] = useState("");
  const [grid, setGrid] = useState("");
  // OS location service (opt-in, issue #471). Disabled by default: the button
  // only appears once locEnabled is on, and nothing requests location until the
  // user presses it.
  const [locEnabled, setLocEnabled] = useState(false);
  const [locating, setLocating] = useState(false);
  const [locError, setLocError] = useState<string | null>(null);
  const [inputs, setInputs] = useState<AudioDevice[]>([]);
  const [outputs, setOutputs] = useState<AudioDevice[]>([]);
  const [ports, setPorts] = useState<SerialPortInfo[]>([]);
  const [hamlibRigs, setHamlibRigs] = useState<HamlibRig[]>([]);
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [baseFreq, setBaseFreq] = useState(1500);
  // Live-waterfall FFT developer knobs (issue #428). Defaults mirror
  // WfConfig::default() on the Rust side: hann / 2048 / 6.
  const [wfCfg, setWfCfg] = useState<WaterfallConfig>({
    window: "hann",
    fft_size: 2048,
    avg: 6,
  });
  // WSJT-X UDP interface (outbound broadcast + inbound requests). Defaults match
  // WSJT-X: disabled, 127.0.0.1:2237. Rust persists the udp_* keys.
  const [udp, setUdp] = useState<UdpConfig>({
    enabled: false,
    host: "127.0.0.1",
    port: 2237,
    accept_requests: false,
  });
  const [rigCfg, setRigCfg] = useState<RigConfig>({
    backend: "none",
    port: "",
    baud: 38400,
    flrig_host: "127.0.0.1",
    flrig_port: 12345,
    hamlib_model: 1,
    hamlib_network: false,
    hamlib_address: "127.0.0.1:4992",
  });

  useEffect(() => {
    api.listAudioInputs().then(setInputs);
    api.listAudioOutputs().then(setOutputs);
    api.listSerialPorts().then(setPorts);
    api.listHamlibRigs().then(setHamlibRigs);
    api.getConfig("my_call").then((v) => v && setCall(v));
    api.getConfig("my_grid").then((v) => v && setGrid(v));
    // Restore the opt-in flag; only the exact string "true" enables it, so a
    // missing/garbage value keeps the feature off.
    api.getConfig("location_service_enabled").then((v) => setLocEnabled(v === "true"));
    api.getConfig("input_device").then((v) => v && setInput(v));
    api.getConfig("output_device").then((v) => v && setOutput(v));
    api.getConfig("base_freq").then((v) => v && setBaseFreq(parseInt(v, 10)));
    api.getConfig("rig_config").then((v) => {
      if (v) {
        try {
          // Normalize the persisted config through the selector mapping so a
          // legacy/removed backend (e.g. "serial") falls back to "none" while
          // keeping the connection fields — otherwise it would reach the Rust
          // side (serde error) and render "Connecting undefined".
          const parsed = JSON.parse(v) as RigConfig;
          setRigCfg(applySelection(parsed, configToSelection(parsed)));
        } catch {
          /* ignore malformed saved config */
        }
      }
    });
    api.getConfig("rig_label").then((v) => v && setRigLabel(v));
    // Restore persisted waterfall knobs (written by the engine on change),
    // keeping only values from the option lists — an unknown window string or
    // NaN size would leave the <select>s blank and push an invalid config
    // back through applyWfCfg.
    Promise.all([
      api.getConfig("wf_window"),
      api.getConfig("wf_fft_size"),
      api.getConfig("wf_avg"),
    ]).then(([w, size, avg]) => {
      const fftSize = parseInt(size ?? "", 10);
      const avgN = parseInt(avg ?? "", 10);
      setWfCfg((prev) => ({
        window: WF_WINDOWS.includes(w as WfWindow) ? (w as WfWindow) : prev.window,
        fft_size: WF_FFT_SIZES.includes(fftSize) ? fftSize : prev.fft_size,
        avg: WF_AVG_OPTIONS.includes(avgN) ? avgN : prev.avg,
      }));
    });
  }, []);

  function applyWfCfg(next: WaterfallConfig) {
    setWfCfg(next);
    api.setWaterfallConfig(next);
  }

  // Load the persisted WSJT-X UDP settings on mount (only the exact string
  // "true" enables a flag, so a missing/garbage value stays off).
  useEffect(() => {
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

  // Push UDP settings to the backend (which persists + rebinds live).
  function applyUdp(next: UdpConfig) {
    // Normalize before persisting. The port input uses parseInt(...) || 0, so clearing the
    // field yields 0 (NaN/out-of-range are possible too); fall back to the default 2237 so
    // we never persist a port the backend rejects or that can't deliver. Default an empty
    // host to loopback. Write the corrected config back to state so the field reflects it.
    const port =
      Number.isFinite(next.port) && next.port > 0 && next.port <= 65535 ? next.port : 2237;
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

  // Re-enumerate serial ports whenever the serial-connection fields become
  // visible. Ports were previously read only once at mount, so a rig attached
  // after launch — or a Hamlib radio picked before its serial port was chosen —
  // showed an empty/stale list. Selecting a serial-connected Hamlib radio now
  // re-scans, so /dev/ttyACM0 (a QMX/QDX) appears without a workaround.
  const showsPortList =
    rigCfg.backend === "g106" ||
    (rigCfg.backend === "hamlib" && !rigCfg.hamlib_network);
  useEffect(() => {
    if (showsPortList) refreshPorts();
  }, [showsPortList]);

  function saveStation() {
    api.setStation(call, grid);
    onStatus(`Station set: ${call} ${grid}`);
    onSettingsSaved({ call, rigBackend: rigCfg.backend });
  }

  // Persist the opt-in flag. Turning it off never runs location code again; it
  // does not itself request location (only the button does), so no permission
  // prompt appears here.
  function toggleLocEnabled(on: boolean) {
    setLocEnabled(on);
    setLocError(null);
    api.setConfig("location_service_enabled", on ? "true" : "false");
  }

  // One-shot on demand: query the OS location service and fill the (still
  // editable) Grid field. This press is what triggers the OS permission prompt.
  async function useMyLocation() {
    setLocating(true);
    setLocError(null);
    try {
      // The checkbox persists this flag asynchronously; the button only appears
      // once locEnabled is true, so re-assert and AWAIT the write here before
      // querying. Otherwise "enable then immediately click" can reach the backend
      // gate before the flag is committed, and get_os_location would reject.
      await api.setConfig("location_service_enabled", "true");
      const loc = await api.getOsLocation();
      setGrid(loc.grid);
      onStatus(`Location set grid to ${loc.grid}`);
    } catch (e) {
      // The manual field is left untouched on failure.
      setLocError(typeof e === "string" ? e : String(e));
    } finally {
      setLocating(false);
    }
  }

  return (
    <div className="grid2">
      <div className="panel">
        <h3>Station</h3>
        <div className="col">
          <div className="field">
            <label>Callsign</label>
            <input value={call} onChange={(e) => setCall(e.target.value.toUpperCase())} placeholder="K0XYZ" />
          </div>
          <div className="field">
            <label>Grid (Maidenhead)</label>
            <div className="row">
              <input value={grid} onChange={(e) => setGrid(e.target.value.toUpperCase())} placeholder="EN37" />
              {locEnabled && (
                <button
                  type="button"
                  onClick={useMyLocation}
                  disabled={locating}
                  title="Fill the grid from your computer's location service"
                >
                  {locating ? "Locating…" : "📍 Use my location"}
                </button>
              )}
            </div>
            {locError && (
              <span className="muted" style={{ display: "block", marginTop: 4, color: "var(--tx)" }}>
                ⚠ {locError}
              </span>
            )}
          </div>
          <div className="field">
            <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={locEnabled}
                onChange={(e) => toggleLocEnabled(e.target.checked)}
                style={{ width: "auto" }}
              />
              Use OS location service
            </label>
            <span className="muted" style={{ display: "block", marginTop: 4 }}>
              Off by default. When on, a “Use my location” button appears above to
              fill your grid from the operating system’s location service (asks for
              permission the first time). Coarse, city-level accuracy — a 6-char
              subsquare, ~2.5 × 5 km. Linux uses GeoClue; Windows uses the system
              geolocator; on macOS enter your grid manually for now.
            </span>
          </div>
          <button className="primary" onClick={saveStation}>Save station</button>
        </div>
      </div>

      <div className="panel">
        <h3>Hunt</h3>
        <div className="col">
          <div className="muted">
            When Hunt is on (toggle it from the top bar), the app auto-answers
            the best eligible CQ each cycle instead of waiting for a manual
            click — never while already in a QSO. These options control which
            CQ counts as "best." Ported from the Android app's own Hunt engine;
            two of its priorities aren't available here yet (DXCC entity
            lookup, and POTA "new park" ranking need data this app doesn't
            fetch yet).
          </div>
          <div className="field">
            <label>Priority</label>
            <select value={huntPriority} onChange={(e) => onHuntPriority(e.target.value as HuntPriority)}>
              {HUNT_PRIORITIES.map((p) => (
                <option key={p} value={p}>
                  {p === "latest" ? "Latest (most recent decode)"
                    : p === "strongest" ? "Strongest signal"
                    : p === "weakest" ? "Weakest signal"
                    : p === "farthest" ? "Farthest station"
                    : p === "pota_first" ? "POTA activators first"
                    : "New grid first"}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={huntAvoidPileups}
                onChange={(e) => onHuntAvoidPileups(e.target.checked)}
                style={{ width: "auto" }}
              />
              Avoid pileups when a pileup-free CQ is available
            </label>
          </div>
          <div className="field">
            <label>Minimum SNR (dB) — blank disables the floor</label>
            <input
              value={huntMinSnr == null ? "" : String(huntMinSnr)}
              onChange={(e) => {
                const v = e.target.value.trim();
                if (v === "") { onHuntMinSnr(null); return; }
                const n = parseInt(v, 10);
                if (!Number.isNaN(n)) onHuntMinSnr(n);
              }}
              placeholder="e.g. -15"
              style={{ width: 80 }}
            />
          </div>
        </div>
      </div>

      <div className="panel">
        <h3>Time sync</h3>
        <div className="col">
          <div className="muted">
            FT8 needs the clock within ~1 s of UTC. The app syncs to NTP on its own —
            no external time software, and your system clock is left untouched.
          </div>
          <div className="field">
            <label>Clock offset from UTC</label>
            <div>
              {clock?.synced
                ? `${fmtOffset(clock.offset_ms)} (${clock.offset_ms} ms)`
                : "waiting for first sync…"}
            </div>
          </div>
          <div className="field">
            <label>Receive timing</label>
            <div>
              {clock ? "Fixed to UTC slots (auto-calibration disabled)" : "—"}
              <span className="muted" style={{ display: "block", marginTop: 4 }}>
                Decoded DT is informational and cannot move later capture windows.
              </span>
            </div>
          </div>
          <button
            onClick={() => {
              api.resyncTime();
              onStatus("Re-syncing clock to NTP…");
            }}
          >
            Sync now
          </button>
        </div>
      </div>

      <div className="panel">
        <h3>Audio</h3>
        <div className="col">
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
          <div className="field">
            <label>Input device (RX)</label>
            <select
              id="audio-input-select"
              value={input}
              onChange={(e) => {
                setInput(e.target.value);
                api.setInputDevice(e.target.value || null);
              }}
            >
              <option value="">System default</option>
              {inputs.map((d) => (
                <option key={d.id} value={d.id}>
                  {audioDeviceLabel(d)} {d.is_default ? "(default)" : ""}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>Output device (TX)</label>
            <select
              id="audio-output-select"
              value={output}
              onChange={(e) => {
                setOutput(e.target.value);
                api.setOutputDevice(e.target.value || null);
              }}
            >
              <option value="">System default</option>
              {outputs.map((d) => (
                <option key={d.id} value={d.id}>
                  {audioDeviceLabel(d)} {d.is_default ? "(default)" : ""}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>TX audio offset: {baseFreq} Hz</label>
            <input
              type="range"
              min={200}
              max={2800}
              step={1}
              value={baseFreq}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                setBaseFreq(v);
                api.setBaseFreq(v);
              }}
            />
          </div>
        </div>
      </div>

      <div className="panel">
        <h3>Rig control</h3>
        <div className="col">
          <div className="field">
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
          <div className="field">
            <label>Rig {hamlibRigs.length > 0 ? `(${hamlibRigs.length} radios)` : ""}</label>
            <select
              id="rig-backend-select"
              value={configToSelection(rigCfg)}
              onChange={(e) => setRigCfg(applySelection(rigCfg, e.target.value))}
            >
              {rigOptions(hamlibRigs).map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            {hamlibRigs.length === 0 && (
              <span className="muted" style={{ display: "block", marginTop: 4 }}>
                The built-in choices remain available, but Hamlib radios are not listed —
                the Hamlib library didn't load, so
                no radios could be enumerated. Install Hamlib and put{" "}
                <code>hamlib-4.dll</code> / <code>libhamlib</code> next to the app or
                on PATH, then reopen.
              </span>
            )}
          </div>

          {rigCfg.backend === "g106" && (
            <div className="row">
              <div className="field">
                <label>DE-19 serial port</label>
                <div className="row">
                  <select
                    id="rig-serial-port-select"
                    value={rigCfg.port}
                    onChange={(e) => setRigCfg({ ...rigCfg, port: e.target.value })}
                  >
                    <option value="">(select port)</option>
                    {ports.map((p) => (
                      <option key={p.name} value={p.name}>
                        {p.name} ({p.kind})
                      </option>
                    ))}
                  </select>
                  <button type="button" title="Re-scan serial ports" onClick={refreshPorts}>
                    ↻
                  </button>
                </div>
              </div>
              <div className="field">
                <label>Baud</label>
                <input value="19200" readOnly style={{ width: 80 }} />
              </div>
            </div>
          )}

          {rigCfg.backend === "hamlib" && (
            <>
              <div className="field">
                <label>Connection</label>
                <select
                  id="rig-connection-select"
                  value={rigCfg.hamlib_network ? "network" : "serial"}
                  onChange={(e) => setRigCfg({ ...rigCfg, hamlib_network: e.target.value === "network" })}
                >
                  <option value="serial">Serial port (COM)</option>
                  <option value="network">Network (TCP host:port)</option>
                </select>
              </div>

              {rigCfg.hamlib_network ? (
                <div className="field">
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
              ) : (
                <div className="row">
                  <div className="field">
                    <label>Serial port</label>
                    <div className="row">
                      <select id="rig-serial-port-select" value={rigCfg.port} onChange={(e) => setRigCfg({ ...rigCfg, port: e.target.value })}>
                        <option value="">(none — e.g. Dummy)</option>
                        {ports.map((p) => (
                          <option key={p.name} value={p.name}>
                            {p.name} ({p.kind})
                          </option>
                        ))}
                      </select>
                      <button type="button" title="Re-scan serial ports" onClick={refreshPorts}>
                        ↻
                      </button>
                    </div>
                  </div>
                  <div className="field">
                    <label>Baud</label>
                    <select
                      id="rig-baud-select"
                      value={rigCfg.baud}
                      onChange={(e) => setRigCfg({ ...rigCfg, baud: parseInt(e.target.value, 10) })}
                    >
                      {[4800, 9600, 19200, 38400, 57600, 115200].map((b) => (
                        <option key={b} value={b}>{b}</option>
                      ))}
                    </select>
                  </div>
                </div>
              )}
            </>
          )}

          {rigCfg.backend === "flrig" && (
            <div className="row">
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
                  onChange={(e) =>
                    setRigCfg({ ...rigCfg, flrig_port: parseInt(e.target.value, 10) || 12345 })
                  }
                  style={{ width: 80 }}
                />
              </div>
            </div>
          )}

          <button
            className="primary"
            onClick={() => {
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
              onSettingsSaved({ call, rigBackend: rigCfg.backend });
            }}
          >
            Connect rig
          </button>
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

      <CustomBandsPanel onStatus={onStatus} onBandsChanged={onBandsChanged} />

      <div className="panel">
        <h3>Developer — waterfall FFT</h3>
        <div className="col">
          <div className="muted">
            Spectrum/waterfall rendering experiments (issue #428) — affects the
            display only, never decoding. Defaults: Hann, 2048, 6 averages.
          </div>
          <div className="field">
            <label>Window function</label>
            <select
              id="wf-window-select"
              value={wfCfg.window}
              onChange={(e) => applyWfCfg({ ...wfCfg, window: e.target.value as WfWindow })}
            >
              {WF_WINDOWS.map((w) => (
                <option key={w} value={w}>
                  {WF_WINDOW_LABELS[w]}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>FFT size</label>
            <select
              id="wf-fft-size-select"
              value={wfCfg.fft_size}
              onChange={(e) => applyWfCfg({ ...wfCfg, fft_size: parseInt(e.target.value, 10) })}
            >
              {WF_FFT_SIZES.map((n) => (
                <option key={n} value={n}>
                  {n} — {(12000 / n).toFixed(1)} Hz/bin{n === 2048 ? " (default)" : ""}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>Averaging (Welch segments per row)</label>
            <select
              id="wf-avg-select"
              value={wfCfg.avg}
              onChange={(e) => applyWfCfg({ ...wfCfg, avg: parseInt(e.target.value, 10) })}
            >
              {WF_AVG_OPTIONS.map((n) => (
                <option key={n} value={n}>
                  {n}{n === 6 ? " (default)" : n === 1 ? " (off)" : ""}
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
    </div>
  );
}

// The operator-selectable QSO messages, in WSJT-X Tx1–Tx5 order. Each rebuilds
// the outgoing message from the current target/report on the backend.
const TX_STAGES: { stage: TxStage; label: string; title: string }[] = [
  { stage: "grid", label: "Grid", title: "Tx1 — send my grid" },
  { stage: "report", label: "Rpt", title: "Tx2 — send signal report" },
  { stage: "r_report", label: "R+Rpt", title: "Tx3 — send R + report" },
  { stage: "rr73", label: "RR73", title: "Tx4 — send RR73" },
  { stage: "bye73", label: "73", title: "Tx5 — send 73" },
];

// Call CQ and Tune now live in TopBar (matching the Android layout); this bar
// keeps the detailed status + the manual Tx1-5 stage override, which Android
// doesn't surface on its main screen but desktop operators rely on.
function TxBar(props: {
  txState: TxStateEvent | null;
  decoding: boolean;
  onStage: (s: TxStage) => void;
  onStop: () => void;
}) {
  const { txState, decoding, onStage, onStop } = props;
  const tx = txState?.transmitting;
  const tuning = txState?.tuning ?? false;
  const stage = txState?.status.stage ?? "idle";
  const target = txState?.status.target ?? null;
  // Outside a QSO the auto-sequencer stage is "idle", which reads as "nothing
  // happening" even while we're decoding — show the receiver's real activity then.
  const stageLabel = stage === "idle" ? (decoding ? "listening" : "stopped") : stage;
  return (
    <div className="txbar">
      <span className={"badge " + (tx ? "tx" : "rx")}>{tx ? "TX" : "RX"}</span>
      <span className="muted">stage: {stageLabel}</span>
      {target && <span className="muted">→ {target}</span>}
      <span className="msg">{txState?.message ?? ""}</span>
      <div className="spacer" style={{ flex: 1 }} />
      <div className="stage-picker" title={target ? "" : "Pick a station first"}>
        {TX_STAGES.map((s) => (
          <button
            key={s.stage}
            className={stage === s.stage ? "active" : ""}
            disabled={!target || tuning}
            title={s.title}
            onClick={() => onStage(s.stage)}
          >
            {s.label}
          </button>
        ))}
      </div>
      <button className="danger" onClick={onStop}>Stop TX</button>
    </div>
  );
}
