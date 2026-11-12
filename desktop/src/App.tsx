import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type PointerEvent,
  type WheelEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  api,
  onEngineEvent,
  onImportProgress,
  onFccGridProgress,
  onFccGridDone,
  type AudioDevice,
  type BandInfo,
  type BestDx,
  type ClockSyncEvent,
  type CustomBand,
  type CycleTick,
  type EngineEvent,
  type FccSeedProgress,
  type FccSeedSummary,
  type HamlibRig,
  type ImportProgress,
  type ImportSummary,
  type InputLevelEvent,
  type LogStats,
  type OperatingMode,
  type PskReporterSpot,
  type QsoRecord,
  type RigConfig,
  type RigStatusEvent,
  type SerialPortInfo,
  type TxStateEvent,
  type UdpConfig,
  type UiMessage,
  type WaterfallConfig,
  type WfWindow,
  WF_DEFAULT_DISPLAY_HZ,
  WF_MIN_DISPLAY_HZ,
  WF_MAX_DISPLAY_HZ,
} from "./ipc";
import {
  adifToDateInput,
  bandColor,
  buildExportArgs,
  buildGridHeatmapCells,
  buildQsoRecord,
  computeVirtualWindow,
  displayReport,
  formatDistanceMi,
  formatHhmm,
  formFromQso,
  isLogFiltered,
  logbookPlace,
  neededStates,
  neededStatesPreview,
  NEEDED_STATES_PREVIEW_MAX,
  qslMarks,
  qsoCountLabel,
  qsoStatusLabel,
  qsoSyncLabels,
  QSO_COL_DATE,
  QSO_TABLE_COLUMNS,
  QSO_TABLE_WEIGHTS,
  wasStatesWorked,
  workedAllContinents,
  type ConfirmFilter,
  type QsoEditForm,
  type WacProgress,
} from "./logbook";
import { applySelection, configToSelection, rigName, rigOptions } from "./rig";
import { debounce } from "./debounce";
import { audioDeviceLabel } from "./audioDevice";
import {
  countRepliesTo,
  gridToLatLon,
  huntPriorityFromConfig,
  isPotaCq,
  maidenheadDistanceKm,
  selectHuntCandidate,
  HUNT_PRIORITIES,
  type HuntCandidate,
  type HuntPriority,
} from "./hunt";
import { formatPowerWatts, formatSwrRatio, swrBand } from "./meters";
import { formatDecodeDt, formatDecodeFreq, isDecodeDtNotable } from "./decodeRow";
import { wpxPrefixOf } from "./wpxPrefix";
import { resolveQsoStatus, QSO_STATUS_META } from "./qsoStatus";
import { collapseByStation, nextSortMode, SORT_MODE_LABEL, SORT_MODE_TITLE, type DecodeSortMode } from "./decodeSort";
import { EquirectViewport, minScaleForLatitude, scalePanForZoom, type Viewport } from "./mapViewport";
import { AzimuthalViewport } from "./azimuthalViewport";
import { COMPASS_DIRECTIONS, compassVector, RANGE_RING_KMS, rangeRingRadiusPx } from "./azimuthalProjection";
import { isNewZoneStation, markerColorForWorkedStatus, snrTint, workedBandStatus } from "./markerStyle";
import { tileLonLatBounds, tileZoomForViewport } from "./mapProjection";
import {
  buildGridTrackerSlot,
  currentSlotMessages,
  isStationVisible,
  type SlotMessage,
} from "./gridTrackerSlot";
import { buildAccumulatedStations } from "./stationAccumulator";
import { buildConnectionLines } from "./connectionLines";
import { nightPolygonLatLon, subsolarPoint } from "./terminator";
import {
  applyPskBandFilter,
  initialPskFilter,
  pskTimeLabel,
  resolvePskModeString,
  PSK_TIME_PRESETS,
  type PskDirection,
  type PskFilter,
  type PskMode,
} from "./pskFilter";
import { summarizeSignalReach } from "./signalReach";
import { forEachRingVertex, parseGeoJsonRings, parseStateLabels, stateBorderAlpha } from "./worldOutlines";
import worldLandGeoJson from "./map-data/world_land.json";
import worldCountriesGeoJson from "./map-data/world_countries.json";
import usStatesGeoJson from "./map-data/us_states.json";

type Tab = "decode" | "waterfall" | "log" | "settings" | "map" | "pota";
type Filter = "all" | "cq" | "pota" | "dxcc" | "zone" | "state" | "grid" | "prefix" | "needed" | "tome";
const MAX_MESSAGES = 800;

// Logbook fetch cap. A previous round removed this entirely ("No limit":
// Number.MAX_SAFE_INTEGER, SQLite treats a LIMIT that large as unbounded)
// because a capped page only ever sorted that arbitrary window instead of
// the whole matching log. That traded a real, worse bug for it: confirmed
// live, fetching and holding an entire ~45,600-row (then larger) log
// unconditionally on every refresh/keystroke ballooned WebKitWebProcess to
// 2-2.7GB RSS and nearly locked up this 3.8GB board twice, with no import
// or anything else running. The backend's `ORDER BY id DESC` (newest
// first, the table's own default sort) already matches this cap correctly
// for the common case (just opened the tab, no search, default sort);
// sorting the capped page by a different column is still only correct
// within those LOG_FETCH_LIMIT rows, not the whole log -- the count label
// shows "shown of total" (`qsoCountLabel`) whenever the fetch was
// truncated, same as it already does for an active search/filter, so this
// is visible, not a silent wrong answer.
const LOG_FETCH_LIMIT = 300;

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
// Top of the displayed audio band -- operator-adjustable ("Width"), mirroring
// the backend's WfConfig.display_hz. A plain module-level mutable (not React
// state) because drawSignalLabels is a plain function, not a component, kept
// current by the `wfDisplayHzGlobal = displayHz` effect in App() -- the same
// "module global as the escape hatch for non-component drawing code" pattern
// wfCanvas below already uses. React components (WaterfallScale/WaterfallView)
// take the live value as a prop instead, for correct re-rendering.
let wfDisplayHzGlobal = WF_DEFAULT_DISPLAY_HZ;
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
    const x = Math.round((m.freq_hz / wfDisplayHzGlobal) * cv.width);
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
  const [audio, setAudio] = useState<InputLevelEvent | null>(null);
  const [, setStatus] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  // Decode list sort/collapse mode (Time/Call/SNR/DX), matching Android's
  // decode_sort_mode -- cycled from the TIME/CALL/SNR/DX button in the top
  // bar (see TopBar, after the SWR/PWR chips).
  const [sortMode, setSortMode] = useState<DecodeSortMode>("time");
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
  // Operating mode (FT8/FT4), matching Android's Band & Mode selector --
  // shared by TopBar's title picker and the More panel's Band & Mode card
  // (see BandModePicker), same as Android reuses one bandModeLabel in both
  // its title row and its TxStrip/drawer rows.
  const [operatingMode, setOperatingModeState] = useState<OperatingMode>("ft8");
  // Waterfall "Width"/"NR"/"MSG" (WaterfallStatusStrip), matching Android's
  // spectrumWidth/deNoise/markMessage. displayHz is lifted here (not local
  // to WaterfallView) because the waterfall strip's own Width control and
  // Settings -> Audio both need to read/write the same live value -- Android
  // explicitly offers Width in both places (WaterfallScreen.kt's comment:
  // "Opens the same Spectrum Width setting as Settings -> Radio & Audio").
  // NR has no rendering-affecting state on the frontend at all (the denoise
  // math runs entirely server-side in wf.rs); this is purely the chip's
  // on/off display + persistence. MSG also needs no prop-threading to
  // WaterfallView: drawSignalLabels is a plain module function (not a
  // component), gated via showMessagesRef below.
  const [displayHz, setDisplayHzState] = useState(WF_DEFAULT_DISPLAY_HZ);
  const [denoise, setDenoiseState] = useState(false);
  const [showMessages, setShowMessages] = useState(true);
  // Calls already worked on the current band and operating mode (upper-
  // cased), for the Decode list's worked-station strikethrough -- "worked"
  // is station+band+mode (db.rs's Db::worked_calls), not station alone, so
  // a call worked only in a different mode on this band doesn't count here.
  // qso.rs's QsoEngine.mode is set live from the real operating mode (same
  // mechanism as band/freq_mhz), so this is never just "FT8" in practice.
  const [workedCalls, setWorkedCalls] = useState<Set<string>>(new Set());
  // Calls already worked in the current operating mode, any band, for the
  // decode-row status pill's New Band tier (worked elsewhere, but not the
  // currently-tuned band).
  const [workedCallsAnyBand, setWorkedCallsAnyBand] = useState<Set<string>>(new Set());
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
  const [myCall, setMyCall] = useState("");
  // Last custom CQ free-text message sent, persisted so it survives a
  // restart -- matches Android's CqOptionsSheet "SAVED" quick-select chip
  // (CqOptionsSheet.kt's savedFreeText/onArmSavedCq/onRemoveSavedCq).
  const [savedFreeText, setSavedFreeText] = useState("");
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
    api.workedCalls(operatingMode).then((calls) => setWorkedCalls(new Set(calls)));
  }
  // Re-fetch whenever the tuned band or operating mode changes -- "worked"
  // is station+band+mode (db.rs's Db::worked_calls), not station alone, so
  // a call worked only in a different mode on this same band must not count
  // here. (Previously hardcoded the literal string "FT8" regardless of
  // operatingMode and never re-fetched on a mode switch -- confirmed a real
  // bug: operating in FT4 still queried/highlighted as if on FT8.)
  useEffect(refreshWorkedCalls, [dialHz, operatingMode]);

  function refreshWorkedCallsAnyBand() {
    api.workedCallsAnyBand(operatingMode).then((calls) => setWorkedCallsAnyBand(new Set(calls)));
  }
  useEffect(refreshWorkedCallsAnyBand, [operatingMode]);

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
    api.workedStates(operatingMode).then((states) => setWorkedStates(new Set(states)));
  }
  // Re-fetch whenever the tuned band or operating mode changes -- WAS is per
  // station+band+mode, same reason refreshWorkedCalls re-fetches on both now.
  useEffect(refreshWorkedStates, [dialHz, operatingMode]);

  // Callsign/grid table seed (Map screen prerequisite, fcc_grid.rs). The
  // first-run seed is triggered from the *backend* now (main.rs's setup(),
  // narrated through native OS dialogs, not this window -- see that
  // function's doc comment for why: a real WebKitGTK repaint freeze on this
  // hardware means this window can silently stop updating shortly after
  // launch). This component only *listens* passively for
  // "fcc-grid-progress"/"fcc-grid-done" for the whole app lifetime, rather
  // than driving the seed itself, so the in-window dialog below still works
  // correctly whenever the window happens to be repainting, regardless of
  // which trigger (the backend's first-run auto-start, or a future Settings
  // "update now" button) started it -- both emit the same two events.
  const [fccSeedProgress, setFccSeedProgress] = useState<FccSeedProgress | null>(null);
  const [fccSeedSummary, setFccSeedSummary] = useState<FccSeedSummary | null>(null);
  const [fccSeedError, setFccSeedError] = useState<string | null>(null);

  // Logbook import/export state, lifted up from LogScreen: Android's three-
  // icon action row (Upload/Download/Share, LogbookScreen.kt's TopBar
  // `actions`) lives in LogbookTopBar, a sibling of LogScreen rather than its
  // parent (App()'s layout always renders a top bar above `.content`, unlike
  // Android where one composable owns both) -- so the state those icons
  // drive has to live here, the nearest common ancestor, not inside
  // LogScreen. logRefreshKey mirrors Android's own `refreshKey` pattern
  // (LogbookScreen.kt: "Bumped after an edit or delete to re-run the
  // loader") so LogScreen's own refresh() still re-runs after an import
  // completes without lifting its search/filter/rows state too.
  const [logRefreshKey, setLogRefreshKey] = useState(0);
  const [exportRangeStart, setExportRangeStart] = useState("");
  const [exportRangeEnd, setExportRangeEnd] = useState("");
  const [exportSheetOpen, setExportSheetOpen] = useState(false);
  const [importResult, setImportResult] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importProgress, setImportProgress] = useState<ImportProgress | null>(null);
  const [uploadSyncNotice, setUploadSyncNotice] = useState(false);

  async function startImportAdif() {
    // The file never crosses IPC as content -- a native dialog returns just
    // a path, and the Rust side opens and streams the file itself (see
    // ipc.ts's importAdif doc comment: the previous file.text()-then-invoke
    // approach could buffer a large real log several times over in memory
    // and locked this board up twice on a real ~80MB file).
    const path = await api.pickAdifFile();
    if (!path) return; // Cancel
    // Set busy state synchronously, before the first await -- otherwise the
    // button and progress popup stay unchanged (looking like the click did
    // nothing) for however long the IPC round-trip takes to even start.
    setImportResult(null);
    setImportError(null);
    setImportProgress({ processed: 0, total: 0 });
    const unlisten = await onImportProgress(setImportProgress);
    try {
      setImportResult(await api.importAdif(path));
      // A bulk import can add many QSOs across many entities/grids/prefixes/
      // zones/states/calls in one shot, but it never goes through the live
      // qso_completed engine event (the only other place these refresh) --
      // without this, every one of these Decode-screen "worked" sets stays
      // however it was at app launch until the next live QSO or a band/mode
      // change, so an imported log's own stations wrongly show as New DXCC /
      // New Grid / New Prefix / New Zone / New State / not-worked.
      refreshWorkedCalls();
      refreshWorkedGrids();
      refreshWorkedPrefixes();
      refreshWorkedDxccPrefixes();
      refreshWorkedCqZones();
      refreshWorkedStates();
      refreshWorkedCallsAnyBand();
    } catch (err) {
      setImportError(String(err));
    } finally {
      unlisten();
      setImportProgress(null);
    }
    setLogRefreshKey((k) => k + 1);
  }

  async function exportAdif() {
    const { start, end } = buildExportArgs(exportRangeStart, exportRangeEnd);
    const adif = await api.exportAdif(start, end);
    const blob = new Blob([adif], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "ft8af_log.adi";
    a.click();
    URL.revokeObjectURL(url);
    setExportSheetOpen(false);
  }

  useEffect(() => {
    let cancelled = false;
    let unlistenProgress: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    onFccGridProgress((p) => {
      setFccSeedProgress(p);
      setFccSeedSummary(null);
      setFccSeedError(null);
    }).then((u) => (cancelled ? u() : (unlistenProgress = u)));
    onFccGridDone((d) => {
      setFccSeedProgress(null);
      if ("ok" in d) setFccSeedSummary(d.ok);
      else setFccSeedError(d.err);
    }).then((u) => (cancelled ? u() : (unlistenDone = u)));
    return () => {
      cancelled = true;
      unlistenProgress?.();
      unlistenDone?.();
    };
  }, []);

  // For the future Settings "update now" button: kicks the backend command
  // directly; the passive listener above (not this function's return value)
  // is what actually drives the dialog, so it behaves identically to the
  // backend's own first-run auto-start.
  function startFccGridSeed() {
    setFccSeedProgress({ phase: "downloading", processed: 0, total: 0, current_callsign: "" });
    setFccSeedSummary(null);
    setFccSeedError(null);
    api.importFccGridSeed().catch((err) => setFccSeedError(String(err)));
  }

  // keep latest decoding flag for the toggle without stale closure
  const decodingRef = useRef(decoding);
  decodingRef.current = decoding;
  // Same "always current" escape hatch for the engine-event handler's MSG
  // gate (drawSignalLabels) and the module-level mirror drawSignalLabels
  // itself reads (it's a plain function, not a component -- see
  // wfDisplayHzGlobal below).
  const showMessagesRef = useRef(showMessages);
  showMessagesRef.current = showMessages;
  useEffect(() => {
    wfDisplayHzGlobal = displayHz;
  }, [displayHz]);

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
    api.getConfig("dial_hz").then((v) => {
      if (v) setDialHz(parseInt(v, 10));
    });
    api.getConfig("operating_mode").then((v) => {
      const mode: OperatingMode = modeFromConfigStr(v);
      setOperatingModeState(mode);
      // Explicit, not a reactive [operatingMode] effect (removed -- see
      // onMode's own doc comment on why that indirection was unreliable):
      // this is the one place besides onMode that needs the band list to
      // follow mode, so it fetches directly instead of depending on a
      // separate effect whose dependency-change timing turned out to be
      // fragile in practice.
      api.listBandsForMode(mode).then(setBands);
    });
    api.getConfig("wf_display_hz").then((v) => {
      const hz = v == null ? NaN : parseFloat(v);
      if (Number.isFinite(hz)) setDisplayHzState(hz);
    });
    api.getConfig("wf_denoise").then((v) => setDenoiseState(v === "true"));
    api.getConfig("wf_show_messages").then((v) => {
      // Only the exact string "false" turns it off -- matches Android's
      // markMessage default of true, and an unset/garbage value stays on.
      if (v === "false") setShowMessages(false);
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
    api.getConfig("my_call").then((v) => {
      if (v) setMyCall(v);
    });
    api.getConfig("cq_saved_free_text").then((v) => {
      if (v) setSavedFreeText(v);
    });
    api.getConfig("decode_sort_mode").then((v) => {
      if (v === "call" || v === "snr" || v === "dx" || v === "time") setSortMode(v);
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
          if (showMessagesRef.current) drawSignalLabels(e.data);
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
        refreshWorkedCallsAnyBand();
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
    setMyCall(call);
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

  // Shared by DecodeScreen's own WaterfallStatusStrip and the standalone
  // Waterfall tab's -- named here (not inlined twice) so both stay wired to
  // the same backend calls.
  function onDisplayHz(hz: number) {
    setDisplayHzState(hz);
    api.setWfDisplayHz(hz);
  }
  function onDenoise(v: boolean) {
    setDenoiseState(v);
    api.setWfDenoise(v);
  }
  function onToggleMessages() {
    const next = !showMessages;
    setShowMessages(next);
    api.setConfig("wf_show_messages", String(next));
  }

  // Shared with both BandModePicker instances (TopBar's title and the More
  // panel's Band & Mode card -- same underlying control, see BandModePicker).
  async function onMode(mode: OperatingMode) {
    // Optimistic, then corrected below: engine.rs's switch_mode rejects the
    // switch outright while actually transmitting (radio safety -- see its
    // doc comment), in which case neither operating_mode nor dial_hz config
    // actually changed. Re-reading both after the call, rather than trusting
    // the optimistic update, keeps the UI from showing a mode switch that
    // silently didn't happen.
    setOperatingModeState(mode);
    await api.setMode(mode);
    const [realModeStr, realDialHz] = await Promise.all([
      api.getConfig("operating_mode"),
      api.getConfig("dial_hz"),
    ]);
    const realMode: OperatingMode = modeFromConfigStr(realModeStr);
    setOperatingModeState(realMode);
    if (realDialHz) setDialHz(parseInt(realDialHz, 10));
    // Fetched directly here (not via a [operatingMode]-keyed effect, which
    // this app used to have): BandModeSheet's band list needs to reflect
    // realMode, the *confirmed* result of this specific switch, every time
    // this function runs -- an effect only re-fires on a value change, so it
    // would miss a case where the optimistic setOperatingModeState above
    // already happened to match realMode.
    api.listBandsForMode(realMode).then(setBands);
  }

  // Shared with Settings -> Audio and the More panel's TX-volume row (Android
  // shows TxVolumeRow in both the drawer and nowhere else -- desktop also
  // keeps it in Settings for discoverability, so this one handler drives both).
  function onTxGain(v: number) {
    setTxGain(v);
    debouncedSetTxGain(v);
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
      {fccSeedProgress && <FccGridSeedDialog progress={fccSeedProgress} />}
      {fccSeedSummary && (
        <FccGridSeedCompleteDialog summary={fccSeedSummary} onDone={() => setFccSeedSummary(null)} />
      )}
      {fccSeedError && (
        <FccGridSeedErrorDialog message={fccSeedError} onDone={() => setFccSeedError(null)} />
      )}
      {importProgress && <ImportProgressDialog progress={importProgress} />}
      {importResult && (
        <ImportCompleteDialog summary={importResult} onDone={() => setImportResult(null)} />
      )}
      {importError && (
        <ImportErrorDialog message={importError} onDone={() => setImportError(null)} />
      )}
      {exportSheetOpen && (
        <ExportLogSheet
          rangeStart={exportRangeStart}
          rangeEnd={exportRangeEnd}
          onRangeStart={setExportRangeStart}
          onRangeEnd={setExportRangeEnd}
          onExport={exportAdif}
          onClose={() => setExportSheetOpen(false)}
        />
      )}
      {uploadSyncNotice && <UploadSyncNoticeDialog onDone={() => setUploadSyncNotice(false)} />}
      {tab === "log" ? (
        <LogbookTopBar
          cycle={cycle}
          operatingMode={operatingMode}
          rig={rig}
          rigLabel={rigLabel}
          txState={txState}
          clock={clock}
          importBusy={!!importProgress}
          onUpload={() => setUploadSyncNotice(true)}
          onImport={startImportAdif}
          onExport={() => setExportSheetOpen(true)}
        />
      ) : (
      <TopBar
        cycle={cycle}
        bands={bands}
        dialHz={dialHz}
        onBand={onBand}
        operatingMode={operatingMode}
        onMode={onMode}
        myCall={myCall}
        myGrid={myGrid}
        savedFreeText={savedFreeText}
        onSaveFreeText={(text) => {
          setSavedFreeText(text);
          api.setConfig("cq_saved_free_text", text);
        }}
        rig={rig}
        clock={clock}
        rigLabel={rigLabel}
        txState={txState}
        meter={meter}
        onCq={() => api.startCq()}
        onStop={() => api.stopTx()}
        onFreeText={(text) => api.freeText(text)}
        onToggleTune={() => (txState?.tuning ? api.stopTune() : api.startTune())}
        huntEnabled={huntEnabled}
        onToggleHunt={() => {
          const next = !huntEnabled;
          setHuntEnabled(next);
          api.setConfig("hunt_enabled", String(next));
        }}
        sortMode={sortMode}
        onCycleSortMode={() => {
          const next = nextSortMode(sortMode);
          setSortMode(next);
          api.setConfig("decode_sort_mode", next);
        }}
        onClearDecodes={() => setMessages([])}
        onRefresh={() => api.refreshStatus()}
      />
      )}
      <div
        className={
          "content" + (tab === "decode" || tab === "map" || tab === "log" || tab === "waterfall" ? " decode" : "")
        }
      >
        {(tab === "decode" || tab === "map") && (
          <DecodeScreen
            rightPane={tab === "map" ? "map" : "waterfall"}
            messages={visible}
            sortMode={sortMode}
            myGrid={myGrid}
            myCall={myCall}
            dialHz={dialHz}
            txTarget={txState?.status.target ?? null}
            operatingMode={operatingMode}
            filter={filter}
            setFilter={setFilter}
            workedCalls={workedCalls}
            workedCallsAnyBand={workedCallsAnyBand}
            workedGrids={workedGrids}
            workedPrefixes={workedPrefixes}
            workedDxccPrefixes={workedDxccPrefixes}
            workedCqZones={workedCqZones}
            workedStates={workedStates}
            txFreq={txFreq}
            onSelectFreq={(hz) => {
              setTxFreq(hz);
              api.setBaseFreq(hz);
            }}
            onAnswer={(m) => api.answer({ call_from: m.call_from, grid: m.grid, snr: m.snr })}
            decoding={decoding}
            audio={audio}
            displayHz={displayHz}
            onDisplayHz={onDisplayHz}
            denoise={denoise}
            onDenoise={onDenoise}
            showMessages={showMessages}
            onToggleMessages={onToggleMessages}
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
              displayHz={displayHz}
            />
            <WaterfallStatusStrip
              txFreq={txFreq}
              decoding={decoding}
              audio={audio}
              displayHz={displayHz}
              onDisplayHz={onDisplayHz}
              denoise={denoise}
              onDenoise={onDenoise}
              showMessages={showMessages}
              onToggleMessages={onToggleMessages}
            />
          </div>
        )}
        {tab === "log" && <LogScreen refreshKey={logRefreshKey} myGrid={myGrid} />}
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
            inputGain={inputGain}
            onInputGain={(v) => {
              setInputGain(v);
              debouncedSetRxGain(v);
            }}
            txGain={txGain}
            onTxGain={onTxGain}
            displayHz={displayHz}
            onDisplayHz={(hz) => {
              setDisplayHzState(hz);
              api.setWfDisplayHz(hz);
            }}
            onUpdateGridTable={startFccGridSeed}
            gridSeedBusy={fccSeedProgress != null}
          />
        )}
        {tab === "pota" && (
          <ComingSoonScreen
            title="POTA"
            detail="Parks-on-the-Air park search, activation tracking, and self-spotting, matching the Android app's POTA tab, are planned but not built yet."
          />
        )}
      </div>
      {tab !== "waterfall" && moreOpen && (
        <TxBar
          txState={txState}
          onSetTxPeriod={(parity) => api.setTxPeriod(parity)}
          bands={bands}
          dialHz={dialHz}
          onBand={onBand}
          operatingMode={operatingMode}
          onMode={onMode}
          onToggleTune={() => (txState?.tuning ? api.stopTune() : api.startTune())}
          txGain={txGain}
          onTxGain={onTxGain}
        />
      )}
      <TabBar
        tab={tab}
        onTab={setTab}
        moreOpen={moreOpen}
        onToggleMore={() => setMoreOpen((v) => !v)}
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// Map screen -- ported from the fork's actual Android map: `ui/map/
// GridTrackerPanel.kt` wrapping `com.k1af.ft8af.grid_tracker.GridOsmMapView`
// (osmdroid), confirmed to be the default map in this fork ("the grid
// tracker (the default, fork) rather than the original FT8AF map" -- its
// own doc comment) -- **not** FT8AF upstream's own `ui/map/MapScreen.kt`.
// See `gridTrackerSlot.ts` for the ported per-pass logic this screen is
// built on, and docs/desktop-decode-android-parity.md for the full writeup
// (including why an earlier draft that plotted the whole callsign_qth table
// froze this board, and why "the current slot only" is a real Android
// behavior, not a scope cut invented here). Rendering (land/country
// outlines, the gray-line terminator, pan/zoom) is still the equirect
// canvas approach from that same earlier draft -- see mapProjection.ts/
// mapViewport.ts/terminator.ts/worldOutlines.ts, still correct and reused
// as-is. The real tile basemap (an offline MBTiles-style archive, osmdroid-
// specific) and the per-line direction-pulse animation are deferred, same
// doc.
// ---------------------------------------------------------------------------

/** Mirrors Android's land fill (0x4094A3B8). */
const MAP_LAND_FILL = "rgba(148, 163, 184, 0.25)";
/** Mirrors Android's land outline (0x9094A3B8). */
const MAP_LAND_STROKE = "rgba(148, 163, 184, 0.565)";
/** Mirrors Android's CountryBorderColor (0x7B94A3B8). */
const MAP_COUNTRY_STROKE = "rgba(148, 163, 184, 0.482)";
/** Mirrors Android's GrayLineNightFill (0x4A0B1220). */
const MAP_NIGHT_FILL = "rgba(11, 18, 32, 0.29)";
/** Mirrors Android's GrayLineTerminator (0xB3F5B44A). */
const MAP_TERMINATOR_STROKE = "rgba(245, 180, 74, 0.702)";
/** Mirrors styles.css's --tx (red) -- close to Android's real
 *  tracker_in_my_line_color (#FF1E27) for a QSO line involving my callsign,
 *  reusing an existing desktop token with the same "this is mine" weight. */
const MAP_LINE_MY_CALL = "#ef4444";
/** Mirrors styles.css's --text -- Android's tracker_line_color is plain
 *  white; this is desktop's own "default readable text" neutral instead. */
const MAP_LINE_OTHER = "#e7ecf3";
/** Mirrors styles.css's --accent -- the PSK spot overlay's marker color. */
const MAP_PSK_SPOT = "#ffaf5e";

/** PSK Reporter refetch cadence, matching Android's MapScreen.kt. */
const PSK_POLL_INTERVAL_MS = 5 * 60 * 1000;

// Parsed once at module load (not per-mount) -- see the comment on
// `landRings`/`countryRings` below for why this is a bundled import, not a
// runtime fetch.
const MAP_LAND_RINGS = parseGeoJsonRings(worldLandGeoJson);
const MAP_COUNTRY_RINGS = parseGeoJsonRings(worldCountriesGeoJson);
const MAP_STATE_RINGS = parseGeoJsonRings(usStatesGeoJson);
const MAP_STATE_LABELS = parseStateLabels(usStatesGeoJson);
const MAP_STATE_STROKE_RGB = "148, 163, 184"; // matches MAP_COUNTRY_STROKE's own color, alpha-faded separately

/** The decode-list fields `gridTrackerSlot.ts` needs, plus the raw text for
 *  a clicked marker's label and the SNR for marker tinting. */
interface MapSlotMessage extends SlotMessage {
  text: string;
  snr: number;
  dxccPrefix: string | null;
  cqZone: number | null;
}

function toSlotMessage(m: UiMessage): MapSlotMessage {
  return {
    callFrom: m.call_from, callTo: m.call_to, isCq: m.is_cq, grid: m.grid, utcMs: m.utc_ms, text: m.text,
    snr: m.snr, dxccPrefix: m.dxcc_prefix, cqZone: m.cq_zone,
  };
}

function MapScreen({
  messages,
  operatingMode,
  workedCalls,
  workedCallsAnyBand,
  workedDxccPrefixes,
  workedCqZones,
  myGrid,
  myCall,
  dialHz,
  txTarget,
}: {
  messages: UiMessage[];
  operatingMode: OperatingMode;
  workedCalls: Set<string>;
  workedCallsAnyBand: Set<string>;
  workedDxccPrefixes: Set<string>;
  workedCqZones: Set<number>;
  myGrid: string;
  myCall: string;
  dialHz: number;
  txTarget: string | null;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [userScale, setUserScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [selected, setSelected] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Layer chips: Decodes/Worked partition every CQ marker by worked status
  // (isStationVisible); Gray line toggles the terminator overlay, persisted
  // the same "true"/"false" get_config/set_config round-trip wf_denoise/
  // location_service_enabled already use -- no new Rust needed.
  const [decodesLayer, setDecodesLayer] = useState(true);
  const [workedLayer, setWorkedLayer] = useState(false);
  const [grayLineEnabled, setGrayLineEnabled] = useState(true);
  useEffect(() => {
    api.getConfig("map_gray_line_enabled").then((v) => {
      if (v !== null) setGrayLineEnabled(v === "true");
    });
  }, []);
  function toggleGrayLine() {
    const next = !grayLineEnabled;
    setGrayLineEnabled(next);
    api.setConfig("map_gray_line_enabled", String(next));
  }

  // Tracker toggle: ON (the default, matching Android's own "new installs
  // default to tracker on") keeps today's current-slot-only markers/lines
  // (buildGridTrackerSlot, unchanged); OFF switches to a cross-session
  // accumulator (buildAccumulatedStations/buildConnectionLines) -- one
  // marker per callsign ever decoded, plus a solid line to the live TX
  // target and dashed lines from anyone calling the operator. Both feed
  // the exact same drawCqMarker/isStationVisible drawing code below; only
  // the data source changes.
  const [trackerOn, setTrackerOn] = useState(true);
  useEffect(() => {
    api.getConfig("map_tracker_enabled").then((v) => {
      if (v !== null) setTrackerOn(v === "true");
    });
  }, []);
  function toggleTracker() {
    const next = !trackerOn;
    setTrackerOn(next);
    api.setConfig("map_tracker_enabled", String(next));
  }

  // Azimuthal (radar) projection toggle. "equirect" (the default) keeps
  // today's tile basemap + pan/zoom exactly as-is; "azimuthal" is a
  // great-circle-centered disc, pure vector (no tiles, matching Android's
  // own azimuthal mode by design). Pixel offsets don't carry across
  // projections -- Android resets zoom/pan on switch, so this does too.
  const [projectionMode, setProjectionMode] = useState<"equirect" | "azimuthal">("equirect");
  useEffect(() => {
    api.getConfig("map_projection_mode").then((v) => {
      if (v === "azimuthal" || v === "equirect") setProjectionMode(v);
    });
  }, []);
  useEffect(() => {
    setUserScale(1);
    setPan({ x: 0, y: 0 });
  }, [projectionMode]);
  function toggleProjection() {
    const next = projectionMode === "equirect" ? "azimuthal" : "equirect";
    setProjectionMode(next);
    api.setConfig("map_projection_mode", next);
  }

  /** Constructs whichever `Viewport` the active projection needs. Every
   *  call site below only ever calls `.clampPan`/`.projectLatLon`/
   *  `.panToCenter`/`.pxPerLonDegree` immediately after constructing, so
   *  this factory is the one place that branches on `projectionMode` --
   *  the call sites themselves stay projection-agnostic. */
  function makeViewport(scale: number, px = pan.x, py = pan.y): Viewport {
    if (projectionMode === "azimuthal") {
      return new AzimuthalViewport(size.w, size.h, myPos?.lat ?? 0, myPos?.lon ?? 0, scale, px, py);
    }
    return new EquirectViewport(size.w, size.h, scale, px, py);
  }

  // PSK Reporter overlay. The poll loop lives here (frontend-driven,
  // mirroring Android's own LaunchedEffect loop in the Compose UI layer,
  // not a Rust background thread -- see psk_reporter.rs's doc comment and
  // this file's existing per-decode gridLookup effect below, which already
  // establishes this exact pattern for this screen).
  const [pskOverlayEnabled, setPskOverlayEnabled] = useState(false);
  useEffect(() => {
    api.getConfig("map_psk_overlay_enabled").then((v) => {
      if (v !== null) setPskOverlayEnabled(v === "true");
    });
  }, []);
  const [pskFilter, setPskFilter] = useState<PskFilter>(() => initialPskFilter(dialHz, () => null));
  const [pskSpots, setPskSpots] = useState<(PskReporterSpot & { lat: number; lon: number })[]>([]);
  const [pskError, setPskError] = useState<string | null>(null);
  const [filterSheetOpen, setFilterSheetOpen] = useState(false);
  function togglePskOverlay() {
    const next = !pskOverlayEnabled;
    setPskOverlayEnabled(next);
    api.setConfig("map_psk_overlay_enabled", String(next));
    if (!next) setFilterSheetOpen(false);
  }

  useEffect(() => {
    if (!pskOverlayEnabled) {
      setPskSpots([]);
      setPskError(null);
      return;
    }
    let cancelled = false;
    let firstFetch = true;
    const modeStr = resolvePskModeString(pskFilter.mode, operatingMode);
    const byReceiver = pskFilter.direction === "WHO_I_HEARD";
    async function loop() {
      while (!cancelled) {
        const call = myCall.trim().toUpperCase();
        if (call) {
          const result = await api.pskReporterSpots(call, pskFilter.timeWindowSec, firstFetch, modeStr, byReceiver);
          firstFetch = false;
          if (cancelled) return;
          if (result.spots) {
            const withPos = result.spots
              .map((s) => {
                const pos = gridToLatLon(s.grid);
                return pos ? { ...s, lat: pos.lat, lon: pos.lon } : null;
              })
              .filter((s): s is PskReporterSpot & { lat: number; lon: number } => s !== null);
            setPskSpots(applyPskBandFilter(withPos, pskFilter.band));
            setPskError(null);
          } else {
            // No flicker: keep whatever spots are already showing.
            setPskError(result.error);
          }
        } else {
          setPskSpots([]);
        }
        await new Promise((r) => setTimeout(r, PSK_POLL_INTERVAL_MS));
      }
    }
    loop();
    return () => {
      cancelled = true;
    };
  }, [pskOverlayEnabled, pskFilter, myCall, operatingMode]);

  // GridTrackerPanel.kt redraws only the current decode slot on every pass
  // ("no grid shading: with a big log the worked/confirmed/known grids
  // cover the map. Only the current slot's lines and CQs show") -- ported
  // as-is, not just for parity but because it's also what keeps this
  // screen's data volume tiny (one cycle's decodes, never the whole
  // session or the whole callsign_qth cache -- see gridTrackerSlot.ts).
  const cycleMs = MODE_CYCLE_MS[operatingMode];
  const allSlotMessages = useMemo(() => messages.map(toSlotMessage), [messages]);
  const slotMessages = useMemo(() => currentSlotMessages(allSlotMessages, cycleMs), [allSlotMessages, cycleMs]);

  // Per-callsign grid backfill: a CQ/grid-reply decode's own grid is used
  // directly; anything else (and call_to, always -- the recipient's grid is
  // never in the message itself) is looked up one callsign at a time (never
  // a bulk fetch of the whole table: an earlier draft loaded all ~800k
  // FCC-seeded rows at once and OOM'd this board). Scans the *full*
  // message history, not just the current slot, so Tracker-off mode's
  // cross-session station accumulator (stationAccumulator.ts) has the same
  // grid-resolution completeness Tracker-on already had -- harmless when
  // Tracker is on too, since that just means resolving a superset of the
  // callsigns it actually needs. `null` means "looked up, not found" so a
  // miss isn't re-queried.
  const [gridLookup, setGridLookup] = useState<Map<string, string | null>>(new Map());
  useEffect(() => {
    const needed = new Set<string>();
    for (const m of allSlotMessages) {
      if (!m.callFrom) continue;
      if (!m.grid && !gridLookup.has(m.callFrom)) needed.add(m.callFrom);
      if (!m.isCq && m.callTo && !gridLookup.has(m.callTo)) needed.add(m.callTo);
    }
    if (needed.size === 0) return;
    let cancelled = false;
    Promise.all(Array.from(needed).map((call) => api.qthForCall(call).then((grid) => [call, grid] as const)))
      .then((pairs) => {
        if (cancelled) return;
        setGridLookup((prev) => {
          const next = new Map(prev);
          for (const [call, grid] of pairs) next.set(call, grid);
          return next;
        });
      });
    return () => {
      cancelled = true;
    };
  }, [allSlotMessages, gridLookup]);

  // The bundled Natural Earth land + admin-0 country outlines (parsed once
  // at module load, above -- not fetched at runtime: Tauri's CSP only
  // allow-lists `connect-src` for IPC, not same-origin static assets, so a
  // `fetch()` of a bundled JSON file is silently blocked).
  const landRings = MAP_LAND_RINGS;
  const countryRings = MAP_COUNTRY_RINGS;

  // The terminator moves; recompute its position once a minute (matches
  // Android's cadence -- the math is cheap, no need for anything faster).
  useEffect(() => {
    const id = setInterval(() => setNowMs(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize({ w: Math.round(width), h: Math.round(height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const resolveGrid = useMemo(() => (call: string) => gridLookup.get(call) ?? null, [gridLookup]);

  // Tracker on: today's current-slot-only markers/lines (unchanged).
  const slot = useMemo(
    () => buildGridTrackerSlot(slotMessages, myCall, resolveGrid),
    [slotMessages, myCall, resolveGrid],
  );

  // Tracker off: one marker per callsign ever decoded this session (within
  // the Decode screen's active filter and MAX_MESSAGES cap), plus a solid
  // line to the live TX target and dashed lines from anyone calling the
  // operator -- see stationAccumulator.ts/connectionLines.ts.
  const accumulatedStations = useMemo(
    () => buildAccumulatedStations(allSlotMessages, myCall, resolveGrid),
    [allSlotMessages, myCall, resolveGrid],
  );
  const accumulatedLines = useMemo(
    () => buildConnectionLines(accumulatedStations, txTarget),
    [accumulatedStations, txTarget],
  );

  // One shared marker shape for whichever mode is active, pre-filtered by
  // the Decodes/Worked layer chips -- both the click hit-test and the draw
  // loop below read this instead of duplicating the per-mode branch twice.
  // `message` is kept (not just the fields drawn today) so Tracker-on's
  // popup (below) can read SNR/grid/DXCC-zone newness and hand `call_from`/
  // `grid`/`snr` straight to `api.answer` -- the same "call this station
  // now" action a Decode-row click already triggers (`onAnswer`).
  const activeMarkers = useMemo(() => {
    const raw = trackerOn
      ? slot.markers.map((m) => ({ callsign: m.callsign, grid: m.grid, label: m.message.text, snr: m.message.snr, message: m.message }))
      : accumulatedStations.map((s) => ({ callsign: s.callsign, grid: s.grid, label: s.message.text, snr: s.message.snr, message: s.message }));
    return raw
      .map((m) => {
        const status = workedBandStatus(m.callsign, workedCalls, workedCallsAnyBand);
        const isNewZone = isNewZoneStation(
          status,
          m.message.dxccPrefix,
          m.message.cqZone,
          workedDxccPrefixes,
          workedCqZones,
        );
        return { ...m, worked: status.workedThisBand || status.workedOtherBand, ...status, isNewZone };
      })
      .filter((m) => isStationVisible(m.worked, decodesLayer, workedLayer));
  }, [
    trackerOn,
    slot.markers,
    accumulatedStations,
    workedCalls,
    workedCallsAnyBand,
    workedDxccPrefixes,
    workedCqZones,
    decodesLayer,
    workedLayer,
  ]);

  const myPos = useMemo(() => gridToLatLon(myGrid), [myGrid]);

  // Offline satellite tile basemap (src-tauri/src/map_tiles.rs). Two refs,
  // not one: `tileCacheRef` dedupes in-flight fetches (the draw effect below
  // can re-run many times per second mid-drag, and without promise-level
  // dedup the same tile key would fire duplicate IPC calls before the first
  // resolves), while `tileBitmapsRef` holds already-decoded bitmaps for the
  // synchronous draw loop to read -- a canvas draw can't await a promise.
  // `tileTick` exists purely to force a redraw when a tile finishes loading
  // asynchronously, independent of any pan/zoom/state change.
  const tileCacheRef = useRef<Map<string, Promise<ImageBitmap | null>>>(new Map());
  const tileBitmapsRef = useRef<Map<string, ImageBitmap>>(new Map());
  const [tileTick, setTileTick] = useState(0);
  function ensureTileLoaded(z: number, x: number, y: number) {
    const key = `${z}/${x}/${y}`;
    if (tileCacheRef.current.has(key)) return;
    const loaded = api
      .mapTile(z, x, y)
      .then((buf) => (buf.byteLength === 0 ? null : createImageBitmap(new Blob([buf], { type: "image/jpeg" }))))
      .then((bitmap) => {
        if (bitmap) {
          tileBitmapsRef.current.set(key, bitmap);
          setTileTick((t) => t + 1);
        }
        return bitmap;
      })
      .catch(() => null);
    tileCacheRef.current.set(key, loaded);
  }

  // Pointer drag-to-pan + wheel-to-zoom.
  const dragRef = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  function onPointerDown(e: PointerEvent<HTMLCanvasElement>) {
    dragRef.current = { x: e.clientX, y: e.clientY, moved: false };
    e.currentTarget.setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: PointerEvent<HTMLCanvasElement>) {
    const d = dragRef.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (Math.abs(dx) > 2 || Math.abs(dy) > 2) d.moved = true;
    d.x = e.clientX;
    d.y = e.clientY;
    const vp = makeViewport(userScale);
    setPan(vp.clampPan(pan.x + dx, pan.y + dy));
  }
  function onPointerUp(e: PointerEvent<HTMLCanvasElement>) {
    const d = dragRef.current;
    dragRef.current = null;
    if (d && !d.moved) {
      // A click (no drag): select the nearest CQ marker within 14px, if any.
      const rect = e.currentTarget.getBoundingClientRect();
      const vp = makeViewport(userScale);
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      let best: { call: string; d2: number } | null = null;
      for (const m of activeMarkers) {
        const pos = gridToLatLon(m.grid);
        if (!pos) continue;
        const p = vp.projectLatLon(pos.lat, pos.lon);
        const d2 = (p.x - cx) ** 2 + (p.y - cy) ** 2;
        if (d2 <= 14 * 14 && (!best || d2 < best.d2)) best = { call: m.callsign, d2 };
      }
      setSelected((prev) => (best ? best.call : prev === null ? null : null));
    }
  }
  // Shared by the scroll wheel and the +/- buttons -- neither is anchored to
  // a cursor position (this panel has no mouse on a touch-only SBC), so both
  // just scale the existing pan in place, same as `onWheel` always has.
  function zoomBy(factor: number) {
    const next = Math.min(20, Math.max(1, userScale * factor));
    const vp = makeViewport(next);
    setUserScale(next);
    const scaledPan = scalePanForZoom(pan, userScale, next);
    setPan(vp.clampPan(scaledPan.x, scaledPan.y));
  }
  function onWheel(e: WheelEvent<HTMLCanvasElement>) {
    e.preventDefault();
    zoomBy(e.deltaY < 0 ? 1.15 : 1 / 1.15);
  }
  function recenter() {
    if (!myPos) return;
    if (projectionMode === "azimuthal") {
      // Azimuthal already centers on the operator via the projection itself
      // (opLat/opLon baked into AzimuthalViewport) -- scale 1, pan {0,0} is
      // the correct "home" view, not a special case to work around.
      setUserScale(1);
      setPan({ x: 0, y: 0 });
      return;
    }
    // See minScaleForLatitude's doc comment (mapViewport.ts) -- scale 1
    // can't reach a non-equatorial latitude on this panel at all.
    const base = new EquirectViewport(size.w, size.h, 1, 0, 0).baseUniform;
    const scale = minScaleForLatitude(size.h, base, myPos.lat);
    setUserScale(scale);
    const vp = makeViewport(scale, 0, 0);
    setPan(vp.panToCenter(myPos.lat, myPos.lon));
  }

  // Draw. Called on every render-triggering change via the effect below,
  // and also directly (bypassing React) by the direction-sweep animation
  // loop further down -- the canvas itself is the only thing touched (no
  // React children to diff), so an imperative redraw is simpler and
  // cheaper than modeling the map as DOM/SVG nodes either way.
  function drawMap() {
    const canvas = canvasRef.current;
    if (!canvas || size.w === 0 || size.h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(size.w * dpr);
    canvas.height = Math.round(size.h * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);
    ctx.fillStyle = "#07090f";
    ctx.fillRect(0, 0, size.w, size.h);

    const vp = makeViewport(userScale);

    function strokeRings(rings: number[][], fill: string | null, stroke: string, width: number) {
      ctx!.beginPath();
      for (const ring of rings) {
        forEachRingVertex(ring, fill ? 3 : 2, (lon, lat, first) => {
          const p = vp.projectLatLon(lat, lon);
          if (first) ctx!.moveTo(p.x, p.y);
          else ctx!.lineTo(p.x, p.y);
        });
        ctx!.closePath();
      }
      if (fill) {
        ctx!.fillStyle = fill;
        ctx!.fill();
      }
      ctx!.strokeStyle = stroke;
      ctx!.lineWidth = width;
      ctx!.stroke();
    }

    // US state borders/labels -- shared between both projections, since
    // both read `pxPerLonDegree` off whichever `Viewport` is active.
    // Skipped entirely (not drawn at ~0 alpha) once zoomed out past the
    // fade threshold -- no point stroking/filling invisible text every
    // frame.
    function drawStateBorders() {
      const alpha = stateBorderAlpha(vp.pxPerLonDegree);
      if (alpha <= 0.02) return;
      strokeRings(MAP_STATE_RINGS, null, `rgba(${MAP_STATE_STROKE_RGB}, ${alpha * 0.6})`, 0.6);
      ctx!.font = "9px sans-serif";
      ctx!.fillStyle = `rgba(${MAP_STATE_STROKE_RGB}, ${alpha})`;
      ctx!.textAlign = "center";
      ctx!.textBaseline = "middle";
      for (const label of MAP_STATE_LABELS) {
        const p = vp.projectLatLon(label.lat, label.lon);
        ctx!.fillText(label.abbr, p.x, p.y);
      }
      ctx!.textAlign = "start";
      ctx!.textBaseline = "alphabetic";
    }

    if (projectionMode === "azimuthal" && myPos) {
      // Azimuthal is pure vector (no tiles, matching Android's own
      // azimuthal mode by design) and must clip to the disc: the whole
      // globe maps into normalized radius 1 exactly, so there is no valid
      // projection beyond it -- unlike the equirect tiles (phase 1), this
      // is the one case where clipping is a correctness requirement, not
      // a cosmetic choice.
      const azVp = vp as AzimuthalViewport;
      const opProjected = vp.projectLatLon(myPos.lat, myPos.lon);
      const clipRadius = azVp.discRadiusPx * userScale;
      ctx.save();
      ctx.beginPath();
      ctx.arc(opProjected.x, opProjected.y, clipRadius, 0, Math.PI * 2);
      ctx.clip();
      strokeRings(landRings, MAP_LAND_FILL, MAP_LAND_STROKE, 0.75);
      strokeRings(countryRings, null, MAP_COUNTRY_STROKE, 0.7);
      drawStateBorders();
      // Range rings pan/scale with the map (centered on the operator's
      // current panned position) -- the five fixed Android always draws.
      ctx.setLineDash([4, 6]);
      ctx.strokeStyle = "rgba(148, 163, 184, 0.25)";
      ctx.lineWidth = 1;
      for (const km of RANGE_RING_KMS) {
        const radius = rangeRingRadiusPx(km, azVp.discRadiusPx, userScale);
        ctx.beginPath();
        ctx.arc(opProjected.x, opProjected.y, radius, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.setLineDash([]);
      ctx.restore();

      // Compass bearings: a fixed UI reference, not a map feature, so they
      // deliberately don't pan or scale -- drawn from the canvas's own
      // fixed geometric center, length pinned to the unzoomed disc radius,
      // outside the clip region above (matches Android's own framing of
      // these as chrome, not map content).
      const fixedCx = size.w / 2;
      const fixedCy = size.h / 2;
      for (const dir of COMPASS_DIRECTIONS) {
        const v = compassVector(dir.angleDeg);
        const x2 = fixedCx + v.dx * azVp.discRadiusPx;
        const y2 = fixedCy + v.dy * azVp.discRadiusPx;
        ctx.beginPath();
        ctx.moveTo(fixedCx, fixedCy);
        ctx.lineTo(x2, y2);
        ctx.strokeStyle = dir.cardinal ? "rgba(255, 175, 94, 0.5)" : "rgba(148, 163, 184, 0.3)";
        ctx.lineWidth = dir.cardinal ? 1 : 0.5;
        ctx.stroke();
        const lx = fixedCx + v.dx * (azVp.discRadiusPx + 12);
        const ly = fixedCy + v.dy * (azVp.discRadiusPx + 12);
        ctx.font = dir.cardinal ? "bold 12px sans-serif" : "10px sans-serif";
        ctx.fillStyle = dir.cardinal ? "#ffaf5e" : "#8a96b1";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(dir.label, lx, ly);
      }
      ctx.textAlign = "start";
      ctx.textBaseline = "alphabetic";
    } else {
      strokeRings(landRings, MAP_LAND_FILL, MAP_LAND_STROKE, 0.75);
      strokeRings(countryRings, null, MAP_COUNTRY_STROKE, 0.7);

      // Real offline satellite tiles, drawn over the vector outlines above
      // -- which stay as the permanent placeholder for whatever hasn't
      // finished loading yet, not a special-cased fallback. Tiles are
      // plain equirectangular (verified empirically against the bundled
      // archive, see map_tiles.rs), so the existing projection places them
      // directly: no inverse-projection is needed, since a tile's screen
      // rect is just its two corners forward-projected through the same
      // `vp` everything else here uses. At most 2^z * 2^z candidates exist
      // (256 at the max stored zoom), cheap to brute-force intersect
      // against the canvas every frame.
      const equirectVp = vp as EquirectViewport;
      const tileZoom = tileZoomForViewport(equirectVp.worldPxW);
      const tilesPerSide = 2 ** tileZoom;
      for (let tx = 0; tx < tilesPerSide; tx++) {
        for (let ty = 0; ty < tilesPerSide; ty++) {
          const bounds = tileLonLatBounds(tileZoom, tx, ty);
          const topLeft = vp.projectLatLon(bounds.latMax, bounds.lonMin);
          const bottomRight = vp.projectLatLon(bounds.latMin, bounds.lonMax);
          const destW = bottomRight.x - topLeft.x;
          const destH = bottomRight.y - topLeft.y;
          const visible =
            topLeft.x + destW >= 0 && topLeft.x <= size.w && topLeft.y + destH >= 0 && topLeft.y <= size.h;
          if (!visible) continue;
          ensureTileLoaded(tileZoom, tx, ty);
          const bitmap = tileBitmapsRef.current.get(`${tileZoom}/${tx}/${ty}`);
          if (bitmap) ctx.drawImage(bitmap, topLeft.x, topLeft.y, destW, destH);
        }
      }

      // State borders/labels drawn AFTER the tiles, not before -- the tiles
      // are opaque JPEGs, so drawing borders first (the original order) got
      // them completely painted over the instant a tile finished loading.
      // Confirmed live: zoomed in well past the fade-in threshold over real
      // US terrain, no borders ever appeared until this was reordered.
      drawStateBorders();
    }

    // Gray-line terminator: translucent night fill + amber terminator stroke
    // (Android's GridOsmMapView.initMap also calls setGrayLine()). Gated on
    // the Gray line chip, and on equirect projection specifically: the
    // night polygon's pole-closing construction (nightPolygonLatLon) is
    // built for a rectangular equirect canvas (closing the ring through
    // whichever pole is dark at lon=±180) and doesn't make geometric sense
    // projected through the azimuthal disc -- an azimuthal-correct
    // terminator would need its own geometry, out of this phase's scope,
    // so this is skipped rather than drawn wrong.
    if (grayLineEnabled && projectionMode === "equirect") {
      const night = nightPolygonLatLon(subsolarPoint(nowMs));
      if (night.length >= 8) {
        ctx.beginPath();
        for (let i = 0; i + 1 < night.length; i += 2) {
          const p = vp.projectLatLon(night[i + 1], night[i]);
          if (i === 0) ctx.moveTo(p.x, p.y);
          else ctx.lineTo(p.x, p.y);
        }
        ctx.closePath();
        ctx.fillStyle = MAP_NIGHT_FILL;
        ctx.fill();

        ctx.beginPath();
        const curveLen = night.length - 4;
        for (let i = 0; i < curveLen; i += 2) {
          const p = vp.projectLatLon(night[i + 1], night[i]);
          if (i === 0) ctx.moveTo(p.x, p.y);
          else ctx.lineTo(p.x, p.y);
        }
        ctx.strokeStyle = MAP_TERMINATOR_STROKE;
        ctx.lineWidth = 1.5;
        ctx.stroke();
      }
    }

    // PSK Reporter spot overlay: small squares, drawn under the CQ markers
    // (the full MarkerStyle.kt shape/SNR-tint system is phase-4 polish, out
    // of scope here -- this matches the existing marker-drawing style).
    if (pskOverlayEnabled) {
      for (const spot of pskSpots) {
        const p = vp.projectLatLon(spot.lat, spot.lon);
        const half = 5;
        ctx.fillStyle = MAP_PSK_SPOT;
        ctx.fillRect(p.x - half, p.y - half, half * 2, half * 2);
        ctx.strokeStyle = "#07090f";
        ctx.lineWidth = 0.8;
        ctx.strokeRect(p.x - half, p.y - half, half * 2, half * 2);
      }
    }

    // Lines: Tracker on draws the two real stations in each directed
    // exchange this slot (see gridTrackerSlot.ts); Tracker off draws from
    // the operator's own position instead -- a solid line to the live TX
    // target, dashed lines from anyone calling the operator (see
    // connectionLines.ts; the operator end is implicit/shared there, not
    // stored per-line).
    if (trackerOn) {
      // Direction sweep: a moving highlighted segment traveling from the
      // caller (fromGrid) toward the receiver (toGrid), looping every
      // second -- ported from GridPolyLine's DirectionLineAnimator
      // (Android), which slices a green ~15px segment along the same 0-1s
      // repeating loop to suggest propagation direction (not a literal
      // arrowhead). Approximated here as a straight screen-space segment,
      // matching how the base line itself is already drawn (not a
      // geodesic curve). `drawMap` is called continuously by the rAF loop
      // below whenever there's a line to animate, so reading the fraction
      // straight off the clock (not React state) is enough to animate.
      const sweepFrac = (performance.now() % 1000) / 1000;
      const sliceFrac = 0.12;
      for (const line of slot.lines) {
        const fromPos = gridToLatLon(line.fromGrid);
        const toPos = gridToLatLon(line.toGrid);
        if (!fromPos || !toPos) continue;
        const from = vp.projectLatLon(fromPos.lat, fromPos.lon);
        const to = vp.projectLatLon(toPos.lat, toPos.lon);
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.strokeStyle = line.involvesMyCall ? MAP_LINE_MY_CALL : MAP_LINE_OTHER;
        ctx.lineWidth = line.involvesMyCall ? 2 : 1.5;
        ctx.stroke();

        const segStart = Math.max(0, sweepFrac - sliceFrac);
        const sx = from.x + (to.x - from.x) * segStart;
        const sy = from.y + (to.y - from.y) * segStart;
        const ex = from.x + (to.x - from.x) * sweepFrac;
        const ey = from.y + (to.y - from.y) * sweepFrac;
        ctx.beginPath();
        ctx.moveTo(sx, sy);
        ctx.lineTo(ex, ey);
        ctx.strokeStyle = "#22ff66";
        ctx.lineWidth = line.involvesMyCall ? 4 : 3;
        ctx.lineCap = "round";
        ctx.stroke();
        ctx.lineCap = "butt";
      }
    } else if (myPos) {
      const from = vp.projectLatLon(myPos.lat, myPos.lon);
      for (const line of accumulatedLines) {
        const toPos = gridToLatLon(line.toGrid);
        if (!toPos) continue;
        const to = vp.projectLatLon(toPos.lat, toPos.lon);
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        if (line.style === "SOLID_TX") {
          ctx.setLineDash([]);
          ctx.strokeStyle = MAP_LINE_MY_CALL;
          ctx.lineWidth = 2;
        } else {
          ctx.setLineDash([6, 4]);
          ctx.strokeStyle = MAP_LINE_OTHER;
          ctx.lineWidth = 1.5;
        }
        ctx.stroke();
      }
      ctx.setLineDash([]);
    }

    // Station markers, tinted by worked status (same sets the Decode list
    // uses) -- already filtered by the Decodes/Worked layer chips and
    // resolved to whichever mode (Tracker on/off) is active; see
    // `activeMarkers` above.
    for (const marker of activeMarkers) {
      const pos = gridToLatLon(marker.grid);
      if (!pos) continue;
      const p = vp.projectLatLon(pos.lat, pos.lon);
      const isSelected = marker.callsign === selected;
      const baseColor = markerColorForWorkedStatus(marker);
      // Selection means full opacity regardless of SNR -- matches
      // Android's own markerStyleFor priority order exactly.
      const fill = isSelected ? baseColor : snrTint(baseColor, marker.snr);
      const r = isSelected ? 9 : 6;
      drawCqMarker(ctx, p.x, p.y, fill, r);
      // Tracker-off keeps the old inline canvas label on selection -- it
      // has no popup system of its own (Android's equivalent there is a
      // separate, not-yet-built SelectedStationCard, out of scope here).
      // Tracker-on's selection instead renders a real popup box below,
      // matching GridMarkerInfoWindow.
      if (isSelected && !trackerOn) {
        ctx.font = "11px sans-serif";
        ctx.fillStyle = "#e7ecf3";
        ctx.fillText(`${marker.callsign} -- ${marker.label}`, p.x + r + 3, p.y + 3);
      }
    }
  }

  const drawMapDeps = [
    size,
    userScale,
    pan,
    landRings,
    countryRings,
    slot,
    trackerOn,
    accumulatedLines,
    activeMarkers,
    myPos,
    selected,
    nowMs,
    tileTick,
    grayLineEnabled,
    pskOverlayEnabled,
    pskSpots,
    projectionMode,
  ];
  useEffect(() => {
    drawMap();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, drawMapDeps);

  // Keeps the direction-sweep animation (above) moving for as long as
  // Tracker-on has a line to animate -- ported from DirectionLineAnimator
  // (Android), whose infinite ValueAnimator calls `mapView.invalidate()`
  // every frame for as long as the line exists. A ref (not the effect's own
  // closure) is called each tick so every frame redraws with the *current*
  // render's pan/zoom/selection, not whatever was in scope when the loop
  // started; the effect itself only needs to start/stop the loop, so it
  // depends on nothing but whether there's anything to animate right now.
  // Throttled to ~15fps (vs. Android's 60fps display refresh): this runs on
  // a weak ARM SBC and a full canvas redraw backs every tick, so this still
  // reads as a smooth sweep while costing a lot less than every frame.
  const drawMapRef = useRef(drawMap);
  drawMapRef.current = drawMap;
  const hasAnimatedLines = trackerOn && slot.lines.length > 0;
  useEffect(() => {
    if (!hasAnimatedLines) return;
    let raf = 0;
    let last = 0;
    function tick(t: number) {
      if (t - last >= 66) {
        last = t;
        drawMapRef.current();
      }
      raf = requestAnimationFrame(tick);
    }
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [hasAnimatedLines]);

  const opPos = useMemo(() => gridToLatLon(myGrid), [myGrid]);
  const signalReach = useMemo(
    () => summarizeSignalReach(pskSpots, opPos?.lat ?? null, opPos?.lon ?? null),
    [pskSpots, opPos],
  );

  // Tracker-on's CQ labels (rendered below), matching GridMarkerInfoWindow's
  // real fork customization -- confirmed directly in GridOsmMapView.java's
  // `showNewInfoWindows()`: on the Map tab panel (`isCompactInfo`), *every*
  // CQ marker's box auto-shows (not just "new" ones, unlike the full
  // GridTracker activity), and GridMarkerInfoWindow's own compact branch
  // strips each box down to one line -- message text + a small inline call
  // button, no icons/SNR/distance. That's the actual anti-clutter design:
  // always show a box per marker, but keep each one minimal, not "fewer,
  // bigger boxes shown only on tap" (what this screen built first, before
  // being corrected against the real fork source). Position is plain
  // render-time math (not a hook): `vp` already depends only on
  // already-reactive state (pan/scale/size/projection/operator position),
  // so this recomputes correctly on every render exactly the way the draw
  // effect's own `vp` does. Culled to roughly the visible canvas (with a
  // small margin) -- `.map-screen` clips overflow either way, but skipping
  // the DOM node entirely for anything off-screen matters more here than it
  // did for a single popup, since this can now be dozens of simultaneous
  // CQ labels on a busy band.
  const cqLabels = trackerOn
    ? (() => {
        const vp = makeViewport(userScale);
        const margin = 80;
        const out: { marker: (typeof activeMarkers)[number]; x: number; y: number }[] = [];
        for (const marker of activeMarkers) {
          const pos = gridToLatLon(marker.grid);
          if (!pos) continue;
          const p = vp.projectLatLon(pos.lat, pos.lon);
          if (p.x < -margin || p.x > size.w + margin || p.y < -margin || p.y > size.h + margin) continue;
          out.push({ marker, x: p.x, y: p.y });
        }
        return out;
      })()
    : [];

  return (
    <>
      <div className="chips">
        <div
          className={"chip" + (decodesLayer ? " active" : "")}
          onClick={() => setDecodesLayer((v) => !v)}
        >
          Decodes
        </div>
        <div
          className={"chip" + (pskOverlayEnabled ? " active" : "")}
          onClick={togglePskOverlay}
        >
          PSK spots
        </div>
        <div
          className={"chip" + (workedLayer ? " active" : "")}
          onClick={() => setWorkedLayer((v) => !v)}
        >
          Worked
        </div>
        <div className={"chip" + (grayLineEnabled ? " active" : "")} onClick={toggleGrayLine}>
          Gray line
        </div>
        <div className={"chip" + (trackerOn ? " active" : "")} onClick={toggleTracker}>
          Tracker
        </div>
        <div className={"chip" + (projectionMode === "azimuthal" ? " active" : "")} onClick={toggleProjection}>
          Azimuthal
        </div>
        {pskOverlayEnabled && (
          <div
            className={"chip" + (filterSheetOpen ? " active" : "")}
            onClick={() => setFilterSheetOpen((v) => !v)}
          >
            Filter
          </div>
        )}
      </div>
      <div ref={containerRef} className="map-screen">
        <canvas
          ref={canvasRef}
          style={{ width: "100%", height: "100%", display: "block", cursor: "grab", touchAction: "none" }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onWheel={onWheel}
        />
        <div className="map-overlay-top muted">
          {trackerOn ? (
            <>
              {slot.markers.length} CQ &middot; {slot.lines.length} QSO this cycle
            </>
          ) : (
            <>{activeMarkers.length} stations tracked</>
          )}
          {pskError && <div className="map-psk-error">PSK: {pskError}</div>}
        </div>
        <button type="button" className="map-recenter" onClick={recenter} disabled={!myPos} title="Center on my grid">
          Me
        </button>
        <div className="map-zoom">
          <button
            type="button"
            onClick={() => zoomBy(1.4)}
            disabled={userScale >= 20}
            title="Zoom in"
            aria-label="Zoom in"
          >
            +
          </button>
          <button
            type="button"
            onClick={() => zoomBy(1 / 1.4)}
            disabled={userScale <= 1}
            title="Zoom out"
            aria-label="Zoom out"
          >
            &minus;
          </button>
        </div>
        {pskOverlayEnabled && filterSheetOpen && (
          <PskFilterSheet filter={pskFilter} onChange={setPskFilter} />
        )}
        {pskOverlayEnabled &&
          !filterSheetOpen &&
          pskFilter.direction === "WHO_HEARD_ME" &&
          signalReach && <SignalReachCard reach={signalReach} />}
        {cqLabels.map(({ marker, x, y }) => (
          <TrackerCqLabel
            key={marker.callsign}
            marker={marker}
            screenX={x}
            screenY={y}
            myCall={myCall}
            onCall={() =>
              api.answer({
                call_from: marker.message.callFrom,
                grid: marker.message.grid,
                snr: marker.message.snr,
              })
            }
          />
        ))}
      </div>
    </>
  );
}

/**
 * Tracker-on's always-on CQ label -- ported from `GridMarkerInfoWindow.java`'s
 * *compact* branch, which is specifically the fork's own customization for
 * this embedded panel (`GridOsmMapView.isCompactInfo`): rather than the full
 * GridTracker activity's behavior (a detailed box, shown only for a "new"
 * station), the Map tab panel auto-shows a box for *every* CQ marker but
 * strips each one down to a single line -- message text (struck through
 * once worked this band, matching the Kotlin source's
 * `Paint.STRIKE_THRU_TEXT_FLAG` toggle) plus a small inline call button, no
 * icons/SNR/distance. That's the real anti-clutter design: always show,
 * but keep each box minimal, not "fewer, bigger boxes shown only on tap"
 * (what this screen built first, before being corrected against the actual
 * fork source). The call button is wired straight to `api.answer` -- the
 * same "call this station now" IPC call a Decode-row click already makes,
 * matching `doCallNow()`. A highlighted border still applies for a
 * not-yet-worked DXCC/CQ-zone station (`isNewZone`), since that styling is
 * unconditional in the source, not part of the compact/full split.
 */
function TrackerCqLabel({
  marker,
  screenX,
  screenY,
  myCall,
  onCall,
}: {
  marker: {
    callsign: string;
    label: string;
    workedThisBand: boolean;
    isNewZone: boolean;
  };
  screenX: number;
  screenY: number;
  myCall: string;
  onCall: () => void;
}) {
  const isMe = marker.callsign.toUpperCase() === myCall.trim().toUpperCase();
  return (
    <div className={"map-cq-label" + (marker.isNewZone ? " new-zone" : "")} style={{ left: screenX, top: screenY }}>
      <span className={marker.workedThisBand ? "worked" : undefined}>{marker.label}</span>
      {!isMe && (
        <button type="button" onClick={onCall} title={`Call ${marker.callsign} now`} aria-label={`Call ${marker.callsign} now`}>
          &#9658;
        </button>
      )}
    </div>
  );
}

function PskFilterSheet({ filter, onChange }: { filter: PskFilter; onChange: (f: PskFilter) => void }) {
  const modes: PskMode[] = ["CURRENT", "FT8", "FT4", "FT2"];
  const directions: { id: PskDirection; label: string }[] = [
    { id: "WHO_HEARD_ME", label: "Heard me" },
    { id: "WHO_I_HEARD", label: "I heard" },
  ];
  return (
    <div className="glass-card map-filter-sheet">
      <div className="map-filter-row">
        <span className="muted">Time</span>
        <div className="chips">
          {PSK_TIME_PRESETS.map((sec) => (
            <div
              key={sec}
              className={"chip" + (filter.timeWindowSec === sec ? " active" : "")}
              onClick={() => onChange({ ...filter, timeWindowSec: sec })}
            >
              {pskTimeLabel(sec)}
            </div>
          ))}
        </div>
      </div>
      <div className="map-filter-row">
        <span className="muted">Mode</span>
        <div className="chips">
          {modes.map((m) => (
            <div
              key={m}
              className={"chip" + (filter.mode === m ? " active" : "")}
              onClick={() => onChange({ ...filter, mode: m })}
            >
              {m === "CURRENT" ? "Current" : m}
            </div>
          ))}
        </div>
      </div>
      <div className="map-filter-row">
        <span className="muted">Direction</span>
        <div className="chips">
          {directions.map((d) => (
            <div
              key={d.id}
              className={"chip" + (filter.direction === d.id ? " active" : "")}
              onClick={() => onChange({ ...filter, direction: d.id })}
            >
              {d.label}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function SignalReachCard({ reach }: { reach: ReturnType<typeof summarizeSignalReach> }) {
  if (!reach) return null;
  return (
    <div className="glass-card map-signal-reach">
      <div>
        {reach.receivers} receiver{reach.receivers === 1 ? "" : "s"}
      </div>
      {reach.furthestCall && (
        <div className="muted">
          Furthest: {reach.furthestCall} ({formatDistanceMi(reach.furthestKm)})
        </div>
      )}
      {reach.strongestCall && (
        <div className="muted">
          Strongest: {reach.strongestCall} ({reach.strongestSnr} dB)
        </div>
      )}
    </div>
  );
}

/** The CQ marker shape -- a filled triangle, matching the old point-marker
 *  draft's "calling CQ" shape (the only marker role this screen has now). */
function drawCqMarker(ctx: CanvasRenderingContext2D, x: number, y: number, fill: string, r: number) {
  ctx.fillStyle = fill;
  ctx.beginPath();
  ctx.moveTo(x, y - r);
  ctx.lineTo(x + r, y + r);
  ctx.lineTo(x - r, y + r);
  ctx.closePath();
  ctx.fill();
}

// Honest placeholder for a tab whose real screen isn't built yet (POTA) --
// says so plainly rather than showing fake content.
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

const MODE_LABEL: Record<OperatingMode, string> = { ft8: "FT8", ft4: "FT4", ft2: "FT2" };
// TX/RX cycle length in ms per mode -- matches Rust's Mode::cycle_ms()
// (mode.rs). Drives the countdown/progress-bar timing below; the waterfall
// itself needs no separate fix -- its rows are pushed by the backend's own
// per-mode wf_boundary_row cadence, not computed from a cycle length here.
const MODE_CYCLE_MS: Record<OperatingMode, number> = { ft8: 15_000, ft4: 7_500, ft2: 3_750 };
// Cycle-time subtext shown on each mode tile in BandModeSheet, matching
// Android's BandModeSheet.kt (formatSlotSeconds: 15000->"15s", 7500->"7.5s",
// 3750->"3.75s").
const MODE_CYCLE_LABEL: Record<OperatingMode, string> = {
  ft8: "15s", ft4: "7.5s", ft2: "3.75s",
};

// Resolve a persisted `operating_mode` config string ("FT8"/"FT4"/"FT2", any
// case) the same way Rust's Mode::from_config does -- shared by the startup
// config-load effect and onMode's post-switch re-read, so both can never
// disagree about what a given backend value means.
function modeFromConfigStr(v: string | null | undefined): OperatingMode {
  const u = v?.toUpperCase();
  if (u === "FT4") return "ft4";
  if (u === "FT2") return "ft2";
  return "ft8";
}

// Shared Band & Mode control, used identically by TopBar's title (the
// upper-left picker) and the More panel's Band & Mode card -- matching
// Android, which reuses one bandModeLabel construction in both the title
// row and the TxStrip/drawer rows rather than keeping two independently-
// built pickers that could drift apart. `variant` only changes layout/sizing
// (a big borderless title vs. a bordered labeled card); both open the same
// BandModeSheet. Matches Android's actual BandModeSheet.kt design (a bottom
// sheet: a 3-way mode toggle with cycle times, over a band list with a
// checkmark on the current selection) rather than inline toggle buttons --
// mode selection lives in this one sheet, not as separate buttons next to
// the label.
function BandModePicker(props: {
  variant: "title" | "card";
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  mode: OperatingMode;
  onMode: (m: OperatingMode) => void;
}) {
  const { variant, bands, dialHz, onBand, mode, onMode } = props;
  const [open, setOpen] = useState(false);
  const band = bands.find((b) => b.dial_hz === dialHz);
  const card = variant === "card";
  return (
    <>
      <div
        className={"band-mode-picker" + (card ? " band-mode-picker-card" : "")}
        onClick={() => setOpen(true)}
        role="button"
        tabIndex={0}
      >
        {card && <span className="band-picker-row-label">BAND & MODE</span>}
        <div className="band-picker">
          <span className="band-picker-label">
            {card ? `${(dialHz / 1e6).toFixed(3)} MHz · ` : ""}
            {band ? band.name : "--"} · {MODE_LABEL[mode]}
          </span>
          <span className="chevron">⌄</span>
        </div>
      </div>
      {open && (
        <BandModeSheet
          bands={bands}
          dialHz={dialHz}
          onBand={(hz) => { onBand(hz); setOpen(false); }}
          mode={mode}
          onMode={onMode}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

// The "Band & mode" bottom sheet, matching Android's BandModeSheet.kt: a
// 3-way mode toggle (FT8/FT4/FT2, each with its cycle time) over a band
// list, each row showing name + dial frequency with a checkmark on the
// current selection. All three tiles are selectable -- FT2 decode works the
// same way FT4's does (desktop's FFI links the shared ft8_lib monitor/
// decoder directly rather than going through Android's JNI wrappers, so it
// only needed the third `ftx_protocol_t` constant plumbed through, see
// mode.rs). FT2 has no encoder anywhere (confirmed against ft8af.app, which
// only ever claims decode for it: "decodes in milliseconds — now across
// three modes") -- engine.rs's start_transmit refuses to key up in FT2
// regardless of what the UI allows, and TopBar/TxBar grey out Call CQ/Tune
// while operatingMode is "ft2" to make that receive-only state visible
// rather than silently failing on click. Android's band rows also carry a
// short descriptive subtitle ("Local, evenings", "Daytime, worldwide") that
// desktop's BandInfo has no data for -- omitted rather than inventing text
// that might not match Android's actual wording.
function BandModeSheet(props: {
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  mode: OperatingMode;
  onMode: (m: OperatingMode) => void;
  onClose: () => void;
}) {
  const { bands, dialHz, onBand, mode, onMode, onClose } = props;
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal band-mode-sheet" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Band &amp; Mode</h3>
          <button className="modal-close" aria-label="Close" onClick={onClose}>✕</button>
        </div>
        <div className="mode-tile-row">
          {(["ft8", "ft4", "ft2"] as const).map((m) => (
            <button
              key={m}
              className={"mode-tile" + (mode === m ? " active" : "")}
              title={m === "ft2" ? "FT2: receive-only, matching ft8af.app (no FT2 TX anywhere)" : undefined}
              onClick={() => onMode(m as OperatingMode)}
            >
              <span className="mode-tile-name">{m.toUpperCase()}</span>
              <span className="mode-tile-cycle">{MODE_CYCLE_LABEL[m]}</span>
            </button>
          ))}
        </div>
        <div className="band-sheet-list">
          {bands.map((b) => {
            const active = b.dial_hz === dialHz;
            return (
              <div
                key={b.dial_hz}
                className={"band-sheet-row" + (active ? " active" : "")}
                onClick={() => onBand(b.dial_hz)}
              >
                <span className="band-sheet-name">{b.custom ? "★ " : ""}{b.name}</span>
                <span className="band-sheet-freq">{(b.dial_hz / 1e6).toFixed(3)} MHz</span>
                {active && <span className="band-sheet-check">✓</span>}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function TopBar(props: {
  cycle: CycleTick | null;
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  operatingMode: OperatingMode;
  onMode: (m: OperatingMode) => void;
  myCall: string;
  myGrid: string;
  savedFreeText: string;
  onSaveFreeText: (text: string) => void;
  rig: RigStatusEvent | null;
  clock: ClockSyncEvent | null;
  rigLabel: string;
  txState: TxStateEvent | null;
  meter: { swr_ratio: number | null; power_watts: number | null } | null;
  onCq: () => void;
  onStop: () => void;
  onFreeText: (text: string) => void;
  onToggleTune: () => void;
  huntEnabled: boolean;
  onToggleHunt: () => void;
  sortMode: DecodeSortMode;
  onCycleSortMode: () => void;
  onClearDecodes: () => void;
  onRefresh: () => void;
}) {
  const {
    cycle, bands, dialHz, onBand, operatingMode, onMode, myCall, myGrid, savedFreeText, onSaveFreeText,
    rig, clock, rigLabel, txState, meter,
    onCq, onStop, onFreeText, onToggleTune, huntEnabled, onToggleHunt,
    sortMode, onCycleSortMode, onClearDecodes, onRefresh,
  } = props;
  const [cqOptionsOpen, setCqOptionsOpen] = useState(false);
  const [cqModifier, setCqModifier] = useState(""); // "" = plain CQ, else e.g. "DX" or a custom 1-4 letter code
  const [customModifierDraft, setCustomModifierDraft] = useState("");
  const [freeTextDraft, setFreeTextDraft] = useState("");

  const cycleMs = MODE_CYCLE_MS[operatingMode];
  const secondsLeft = cycle ? Math.max(0, Math.ceil((cycleMs - cycle.ms_into_cycle) / 1000)) : null;
  const pct = cycle ? (cycle.ms_into_cycle / cycleMs) * 100 : 0;
  const transmitting = txState?.transmitting ?? false;
  const tuning = txState?.tuning ?? false;
  // The CQ/QSO auto-sequencer is running (whether keyed up right now or just
  // listening between slots for a reply) -- matches Android's TxStrip.kt
  // cqIsStop, which turns the Call CQ button into a red STOP button for the
  // whole time it's armed, not only while actually transmitting. Without
  // this, clicking Call CQ again while it's active just restarts CQ (the
  // previous behavior here), with no way to stop from the primary button --
  // confirmed live: it left the rig cycling CQ on the air with no obvious
  // way to stop it short of the separate More panel's Stop TX button.
  const cqActive = txState?.status.active ?? false;
  // Which half of the 15s cycle this slot falls in -- purely informational
  // (desktop has no manual slot-parity override), matching WSJT-X/Android's
  // "Odd"/"Even" slot readout.
  const slotParity = cycle ? (cycle.sequential % 2 === 0 ? "Even" : "Odd") : null;
  // FT2 has no encoder anywhere (see BandModeSheet's doc comment) --
  // engine.rs's start_transmit refuses to key up regardless, but greying out
  // every TX entry point here makes the receive-only state visible instead
  // of letting the operator click Call CQ/More/Hunt and get a silent no-op
  // (or, for Call CQ specifically, an error toast with no obvious cause).
  const receiveOnly = operatingMode === "ft2";

  return (
    <>
    <div className="topbar-android">
      <div className="topbar-row">
        <BandModePicker
          variant="title"
          bands={bands}
          dialHz={dialHz}
          onBand={onBand}
          mode={operatingMode}
          onMode={onMode}
        />

        <button
          className={"pill pill-cq" + (cqActive ? " pill-cq-stop" : "")}
          onClick={cqActive ? onStop : onCq}
          disabled={tuning || receiveOnly}
          title={receiveOnly ? "FT2 is receive-only -- switch mode to transmit" : cqActive ? "Stop calling CQ" : "Call CQ"}
        >
          {cqActive ? "STOP" : <span className="pill-label-2line">Call<br />CQ</span>}
        </button>
        <button
          className="pill pill-dark"
          disabled={receiveOnly}
          title={receiveOnly ? "FT2 is receive-only -- switch mode to transmit" : "More: CQ DX/NA/EU/... variants or a free-text message"}
          onClick={() => setCqOptionsOpen(true)}
        >
          More
        </button>

        <button
          className={"pill pill-dark" + (huntEnabled ? " pill-hunt-active" : "")}
          onClick={onToggleHunt}
          disabled={receiveOnly}
          title={receiveOnly ? "FT2 is receive-only -- switch mode to transmit" : "Hunt: auto-answer the best eligible CQ each cycle (priority/options in Settings)"}
        >
          Hunt
        </button>

        <RxTxRadioChips transmitting={transmitting} secondsLeft={secondsLeft} rig={rig} rigLabel={rigLabel} />

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

        {/* Decode-list sort control, matching Android's DecodeScreen.kt
            top-bar sort button: cycles TIME -> CALL -> SNR -> DX on click. */}
        <button
          className="icon-button sort-mode-button"
          onClick={onCycleSortMode}
          title={SORT_MODE_TITLE[sortMode]}
        >
          {SORT_MODE_LABEL[sortMode]}
        </button>

        <div className="spacer" />

        <span className="topbar-clock">
          {cycle ? new Date(cycle.utc_ms).toISOString().substring(11, 19) : "--:--:--"}
        </span>
        <button className="icon-button" title="Refresh rig/engine status" onClick={onRefresh}>⟳</button>
        <button className="icon-button" title="Clear the decode list" onClick={onClearDecodes}>🗑</button>
      </div>

      <SyncRow clock={clock} pct={pct} secondsLeft={secondsLeft} />
    </div>
    {cqOptionsOpen && (
      <CqOptionsModal
        myCall={myCall}
        cqModifier={cqModifier}
        onCqModifier={setCqModifier}
        customModifierDraft={customModifierDraft}
        onCustomModifierDraft={setCustomModifierDraft}
        freeTextDraft={freeTextDraft}
        onFreeTextDraft={setFreeTextDraft}
        savedFreeText={savedFreeText}
        onArmSaved={() => setFreeTextDraft(savedFreeText)}
        onRemoveSaved={() => onSaveFreeText("")}
        onClose={() => setCqOptionsOpen(false)}
        onSubmit={() => {
          const text = freeTextDraft.trim();
          if (text) {
            onFreeText(text);
            // Persist only on an explicit send, and only when it actually
            // changed -- matches Android's shouldPersistFreeText (a saved
            // value is never clobbered by a blank, and an unchanged value
            // skips a redundant write).
            if (text !== savedFreeText) onSaveFreeText(text);
          } else if (cqModifier) {
            onFreeText(buildDirectedCq(cqModifier, myCall, myGrid));
          } else {
            onCq();
          }
          setCqOptionsOpen(false);
        }}
      />
    )}
    </>
  );
}

/** RX/TX indicator + "Radio linked/offline" pill -- extracted out of
 *  TopBar so LogbookTopBar (below) can show the same operating-status
 *  chips Android embeds in its own Logbook title row, without duplicating
 *  the markup. */
function RxTxRadioChips({
  transmitting, secondsLeft, rig, rigLabel,
}: {
  transmitting: boolean;
  secondsLeft: number | null;
  rig: RigStatusEvent | null;
  rigLabel: string;
}) {
  return (
    <>
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
    </>
  );
}

/** The clock-sync badge + per-cycle countdown bar -- extracted out of
 *  TopBar for the same reason as RxTxRadioChips above. */
function SyncRow({
  clock, pct, secondsLeft,
}: {
  clock: ClockSyncEvent | null;
  pct: number;
  secondsLeft: number | null;
}) {
  return (
    <div className="topbar-row sync-row">
      <ClockSyncBadge clock={clock} />
      <div className="cycle-bar">
        <div style={{ width: `${pct}%` }} />
      </div>
      <span className="seq muted">{secondsLeft != null ? `${secondsLeft}s` : "--"}</span>
    </div>
  );
}

/**
 * Android's Logbook screen doesn't share the Decode screen's full operate
 * bar (Call CQ/More/Hunt/Tune/Odd-Even) -- confirmed directly in
 * `ComposeMainActivity.kt`/`FT8AFApp.kt`: those controls live in a
 * separate, collapsible drawer that isn't shown while viewing the
 * Logbook. What *is* shown there (`LogbookScreen.kt`'s own `TopBar` call,
 * `headerStatus`/`titleContent`) is just the screen title, a QSO-count
 * subtitle in the same row, and the same RX/TX/radio-link status chips --
 * confirmed live against a real screenshot, this was the actual source of
 * "the display looks nowhere like the android display" once the table
 * itself was fixed. The user's call: hide the full operate bar on the
 * Logbook tab only (not Map/Settings/POTA), matching this one screen.
 */
function LogbookTopBar({
  cycle, operatingMode, rig, rigLabel, txState, clock, importBusy, onUpload, onImport, onExport,
}: {
  cycle: CycleTick | null;
  operatingMode: OperatingMode;
  rig: RigStatusEvent | null;
  rigLabel: string;
  txState: TxStateEvent | null;
  clock: ClockSyncEvent | null;
  importBusy: boolean;
  onUpload: () => void;
  onImport: () => void;
  onExport: () => void;
}) {
  // Not lifted from LogScreen's own `count` state (a sibling, not a parent/
  // child of this component) -- a second, independent fetch of the same
  // cheap COUNT(*) query LogScreen already calls on its own refresh.
  const [count, setCount] = useState(0);
  useEffect(() => {
    api.logCount().then(setCount);
  }, [importBusy]);

  const cycleMs = MODE_CYCLE_MS[operatingMode];
  const secondsLeft = cycle ? Math.max(0, Math.ceil((cycleMs - cycle.ms_into_cycle) / 1000)) : null;
  const pct = cycle ? (cycle.ms_into_cycle / cycleMs) * 100 : 0;
  const transmitting = txState?.transmitting ?? false;

  return (
    <div className="topbar-android">
      <div className="topbar-row logbook-title-row">
        <span className="logbook-title">Logbook</span>
        <span className="logbook-subtitle muted">{count} QSOs &middot; All bands</span>
        <div className="spacer" />
        <RxTxRadioChips transmitting={transmitting} secondsLeft={secondsLeft} rig={rig} rigLabel={rigLabel} />
        {/* Matches LogbookScreen.kt's TopBar `actions`: Upload (CloudUpload,
            "sync to logging services"), Download (FileDownload, import), Share
            (export) -- in that order, after the status chips. Desktop has no
            Cloudlog/QRZ upload-sync integration yet (a separate, already-
            documented gap, see docs/desktop-decode-android-parity.md), so
            Upload shows an honest "not available" notice instead of either
            silently doing nothing or faking a real sync. */}
        <button className="icon-button topbar-icon" title="Sync to logging services" onClick={onUpload}>
          ↑
        </button>
        <button
          className="icon-button topbar-icon"
          title="Import QSOs"
          onClick={onImport}
          disabled={importBusy}
        >
          ↓
        </button>
        <button className="icon-button topbar-icon" title="Export QSOs" onClick={onExport}>
          ↗
        </button>
      </div>
      <SyncRow clock={clock} pct={pct} secondsLeft={secondsLeft} />
    </div>
  );
}

// Which other-direction/custom CQ variant the operator picked, matching
// Android's CqOptionsSheet.kt presets (CQ/CQ DX/CQ NA/CQ EU/CQ SA/CQ AS/
// CQ OC/CQ AF) plus a free-typed 1-4 letter custom code.
const CQ_PRESETS = ["", "DX", "NA", "EU", "SA", "AS", "OC", "AF"];

function presetLabel(modifier: string): string {
  return modifier ? `CQ ${modifier}` : "CQ";
}

// 1-4 letters, matching CqOptionsSheet.kt's isValidModifier (digits are
// stripped by the input's own sanitizer, so this only needs to reject
// anything the UI can't produce in the first place).
function isValidModifier(text: string): boolean {
  return /^[A-Z]{0,4}$/.test(text);
}

/** Assemble "CQ <modifier> <call>", trimmed and whitespace-collapsed so a
 *  blank modifier yields a plain "CQ <call>" rather than "CQ  <call>" --
 *  ported from CqOptionsSheet.kt's buildDirectedCq. myGrid is appended the
 *  same way plain CQ already does (api.startCq's own "CQ <call> <grid>"). */
function buildDirectedCq(modifier: string, call: string, grid: string): string {
  return `CQ ${modifier} ${call} ${grid}`.trim().replace(/\s+/g, " ");
}

/**
 * CQ variant picker, opened from the Call CQ pill's MORE badge -- matches
 * Android's CqOptionsSheet.kt (screenshots/07_callcq_more.png): directional
 * presets, a custom 1-4 letter modifier, and a free-text field as a
 * separate alternative. Field Day is NOT ported: it needs the pack77_fd
 * binary exchange-packing format (ARRL/RAC section index etc.) desktop's
 * TX encoder doesn't implement, and faking the toggle without it would
 * silently send a wrong/garbled exchange -- left out rather than guessed
 * at, consistent with this branch's other documented gaps (hunt.ts,
 * docs/desktop-decode-android-parity.md).
 *
 * A preset/custom modifier is sent via the same repeating free-text path as
 * the Free Text field (api.freeText), not through the CQ auto-sequencer's
 * own state machine -- it transmits and repeats correctly, and the Call CQ
 * pill still turns into STOP while it's running, but an incoming reply
 * isn't auto-detected/advanced the way a plain CQ's replies are; the
 * operator answers it by clicking the decoded row, same as any other
 * station. Plain "CQ" (blank modifier, no free text) uses the real onCq
 * path instead, which does get full auto-sequencing.
 */
// Free-text charset, matching CqOptionsSheet.kt's sanitizeFreeText exactly
// (space, digits, A-Z, and the FT8 free-text punctuation set).
const FREE_TEXT_CHARSET = /[^ 0-9A-Z+\-./?]/g;

function CqOptionsModal(props: {
  myCall: string;
  cqModifier: string;
  onCqModifier: (m: string) => void;
  customModifierDraft: string;
  onCustomModifierDraft: (v: string) => void;
  freeTextDraft: string;
  onFreeTextDraft: (v: string) => void;
  savedFreeText: string;
  onArmSaved: () => void;
  onRemoveSaved: () => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  const {
    myCall, cqModifier, onCqModifier, customModifierDraft, onCustomModifierDraft,
    freeTextDraft, onFreeTextDraft, savedFreeText, onArmSaved, onRemoveSaved, onClose, onSubmit,
  } = props;

  const buttonLabel = freeTextDraft.trim()
    ? "Send free text"
    : presetLabel(cqModifier) + " " + myCall;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-cq-options" onClick={(e) => e.stopPropagation()}>
        <h3>CQ Message</h3>
        <div className="col">
          <label className="muted" style={{ fontSize: 12 }}>CQ MESSAGE</label>
          <div className="chips">
            {CQ_PRESETS.map((p) => (
              <div
                key={p}
                className={"chip" + (cqModifier === p ? " active" : "")}
                onClick={() => {
                  onCqModifier(p);
                  onCustomModifierDraft(p);
                }}
              >
                {presetLabel(p)}
              </div>
            ))}
          </div>
          <input
            value={customModifierDraft}
            onChange={(e) => {
              const v = e.target.value.toUpperCase().replace(/[^A-Z]/g, "").slice(0, 4);
              onCustomModifierDraft(v);
              if (isValidModifier(v)) onCqModifier(v);
            }}
            placeholder="Custom (1-4 letters, e.g. POTA)"
            maxLength={4}
          />

          <div className="row" style={{ margin: "8px 0", color: "var(--muted)", fontSize: 11 }}>
            <div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
            or
            <div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
          </div>

          <label className="muted" style={{ fontSize: 12 }}>FREE TEXT (13 characters max)</label>
          <input
            value={freeTextDraft}
            onChange={(e) => onFreeTextDraft(e.target.value.toUpperCase().replace(FREE_TEXT_CHARSET, ""))}
            placeholder="FIELD DAY K1ABC FN42"
            maxLength={13}
          />

          {savedFreeText && (
            <div className="row" style={{ marginTop: 8 }}>
              <span className="muted" style={{ fontSize: 11 }}>SAVED</span>
              <div
                className={"chip" + (freeTextDraft === savedFreeText ? " active" : "")}
                onClick={onArmSaved}
                style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
              >
                {savedFreeText}
                <span
                  title="Remove saved message"
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemoveSaved();
                  }}
                >
                  ✕
                </span>
              </div>
            </div>
          )}

          <div className="actions" style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 12 }}>
            <button onClick={onClose}>Cancel</button>
            <button className="primary" onClick={onSubmit}>
              {buttonLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// Status word + color per InputLevelStatus, matching Android's
// InputLevelIndicator.kt exactly (inputLevelStatusText/inputLevelColor):
// SILENT reuses the "too low" wording (the actionable advice -- turn the
// input up / check the cable -- is the same), the muted color is what
// distinguishes a dead input from a merely-quiet one.
const INPUT_LEVEL_META: Record<InputLevelEvent["status"], { label: string; color: string }> = {
  silent: { label: "Low", color: "var(--muted)" },
  low: { label: "Low", color: "#5cd6e8" }, // Android StatusWorked
  good: { label: "Good", color: "var(--ok)" },
  high: { label: "High", color: "var(--yellow)" },
  clipping: { label: "Clip", color: "var(--tx)" },
};

// RX input-level meter: an RMS-filled bar with a peak tick, a percent
// readout of the window peak, and a color-coded status word -- ported from
// Android's InputLevelIndicator.kt (RX label / meter bar / "NN%" / status).
function InputLevelIndicator({
  decoding,
  audio,
}: {
  decoding: boolean;
  audio: InputLevelEvent | null;
}) {
  if (!decoding) return null;
  if (!audio) {
    return <span className="muted" title="Waiting for the first input-level reading">RX …</span>;
  }
  const meta = INPUT_LEVEL_META[audio.status];
  const rmsFrac = Math.min(1, Math.max(0, audio.rms));
  const peakFrac = Math.min(1, Math.max(0, audio.peak));
  const peakPercent = Math.round(peakFrac * 100);
  return (
    <span
      className="input-level-indicator"
      title={
        audio.status === "silent"
          ? "No audio is reaching the app. Check that the selected input device is the right one and that its source is routed (e.g. SmartSDR slice → DAX RX channel, soundcard input not muted)."
          : `RX input level: ${audio.db.toFixed(0)} dBFS RMS, ${peakPercent}% peak`
      }
    >
      <span className="input-level-label">RX</span>
      <span className="input-level-bar">
        <span className="input-level-bar-fill" style={{ width: `${rmsFrac * 100}%`, background: meta.color }} />
        <span className="input-level-bar-peak" style={{ left: `${peakFrac * 100}%`, background: meta.color }} />
      </span>
      <span className="input-level-percent">{peakPercent}%</span>
      <span className="input-level-status" style={{ color: meta.color }}>{meta.label}</span>
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
// (0, 500, 1000, ... up to displayHz), positioned by the same hz/displayHz
// fraction the waterfall canvas and the TX marker already use, so they stay
// aligned. The first/last ticks are pulled in from the edge instead of
// centered, so "0" and the top label don't overflow the container.
function WaterfallScale({ displayHz }: { displayHz: number }) {
  const ticks: number[] = [];
  for (let hz = 0; hz <= displayHz; hz += WF_TICK_STEP_HZ) ticks.push(hz);
  return (
    <div style={{ position: "relative", height: 14, marginBottom: 2 }}>
      {ticks.map((hz) => {
        const pct = (hz / displayHz) * 100;
        const transform = hz === 0 ? "none" : hz === ticks[ticks.length - 1] ? "translateX(-100%)" : "translateX(-50%)";
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
  displayHz,
}: {
  txFreq: number;
  onSelectFreq: (hz: number) => void;
  displayHz: number;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    registerWfCanvas(ref.current);
    return () => registerWfCanvas(null);
  }, []);

  function handleClick(e: MouseEvent<HTMLDivElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    const hz = Math.round(frac * displayHz);
    onSelectFreq(Math.min(TX_MAX_HZ, Math.max(TX_MIN_HZ, hz)));
  }

  const leftPct = (txFreq / displayHz) * 100;

  return (
    <div
      // Fills its parent (`.decode-waterfall`, itself 50% of the window --
      // see DecodeScreen's left/right split) horizontally, and vertically
      // shares that column with its sibling WaterfallStatusStrip via
      // `flex: 1` (not `height: 100%`, which claimed the whole column and
      // squeezed the strip below it down to a sliver -- the canvas's
      // backing pixel buffer stays WF_HEIGHT rows regardless; only the CSS
      // display size changes, so the canvas just scales up like any other
      // image). The scale ruler and the TX marker below are positioned as
      // percentages of this same box, so they stay aligned at any size. The
      // slot-time label is baked into the canvas itself (drawWaterfallRow),
      // not a DOM overlay here -- it needs to scroll down with its own
      // boundary line.
      style={{
        position: "relative", width: "100%", flex: 1, minHeight: 0,
        display: "flex", flexDirection: "column",
        marginBottom: 8, cursor: "crosshair",
      }}
      onClick={handleClick}
      title="Click to set TX frequency"
    >
      <WaterfallScale displayHz={displayHz} />
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

// Empty-state title/body per active filter, ported verbatim from Android's
// EmptyState composable (DecodeScreen.kt) and strings_compose.xml -- "all"
// is Android's `else` branch, whose body interpolates the current mode name
// ("Waiting for FT8 signals to appear...").
const EMPTY_STATE_TEXT: Record<Exclude<Filter, "all">, { title: string; body: string }> = {
  cq: { title: "No CQ calls", body: "No stations are calling CQ on this band right now." },
  pota: {
    title: "No POTA spots",
    body: "No park activations decoded on this band yet — open POTA → Hunt for the spot list.",
  },
  dxcc: { title: "No new DXCC", body: "No unworked DXCC entities have been decoded yet." },
  zone: {
    title: "No new zones",
    body: "No stations calling from an unworked CQ zone have been decoded yet.",
  },
  state: {
    title: "No new states",
    body: "No stations calling from an unworked US state have been decoded yet.",
  },
  grid: {
    title: "No new grids",
    body: "No stations calling from an unworked grid square have been decoded yet.",
  },
  prefix: {
    title: "No new prefixes",
    body: "No stations calling from an unworked callsign prefix (WPX) have been decoded yet.",
  },
  needed: { title: "Nothing needed", body: "No stations needing confirmation found." },
  tome: { title: "No calls for you", body: "No stations are calling your callsign right now." },
};

// Matches Android's EmptyState + EmptyStateWaves (DecodeScreen.kt /
// ui/components/EmptyStateWaves.kt): a decorative radial-glow + three
// stacked wave arcs above a title/body message that varies with the active
// filter. Android animates the waves drifting (an infinite 6s phase loop);
// this port is a static SVG of the same shape rather than a canvas
// animation loop -- the filter-aware copy is the part actually requested,
// and a static graphic is far cheaper to keep correct.
function EmptyState({ filter, operatingMode }: { filter: Filter; operatingMode: OperatingMode }) {
  const { title, body } =
    filter === "all"
      ? { title: "No signals decoded", body: `Waiting for ${MODE_LABEL[operatingMode]} signals to appear...` }
      : EMPTY_STATE_TEXT[filter];
  return (
    <div className="decode-empty-state">
      <svg width="140" height="140" viewBox="0 0 140 140" aria-hidden="true">
        <defs>
          <radialGradient id="emptyStateGlow" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.25" />
            <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
          </radialGradient>
        </defs>
        <circle cx="70" cy="70" r="63" fill="url(#emptyStateGlow)" />
        <path d="M0,56 Q17.5,42 35,56 T70,56 T105,56 T140,56" fill="none" stroke="var(--accent)" strokeOpacity="0.45" strokeWidth="1.5" />
        <path d="M0,70 Q17.5,84 35,70 T70,70 T105,70 T140,70" fill="none" stroke="var(--accent)" strokeOpacity="0.35" strokeWidth="1.3" />
        <path d="M0,84 Q17.5,70 35,84 T70,84 T105,84 T140,84" fill="none" stroke="var(--accent)" strokeOpacity="0.25" strokeWidth="1.1" />
      </svg>
      <div className="decode-empty-title">{title}</div>
      <div className="decode-empty-body">{body}</div>
    </div>
  );
}

function DecodeScreen(props: {
  // Which panel fills the right-hand pane: the waterfall (the default) or
  // the Map, in place of it -- the two are mutually exclusive in that one
  // pane (switching to the Map closes the waterfall, not just hides it).
  rightPane: "waterfall" | "map";
  messages: UiMessage[];
  sortMode: DecodeSortMode;
  myGrid: string;
  myCall: string;
  dialHz: number;
  txTarget: string | null;
  operatingMode: OperatingMode;
  filter: Filter;
  setFilter: (f: Filter) => void;
  workedCalls: Set<string>;
  workedCallsAnyBand: Set<string>;
  workedGrids: Set<string>;
  workedPrefixes: Set<string>;
  workedDxccPrefixes: Set<string>;
  workedCqZones: Set<number>;
  workedStates: Set<string>;
  txFreq: number;
  onSelectFreq: (hz: number) => void;
  onAnswer: (m: UiMessage) => void;
  decoding: boolean;
  audio: InputLevelEvent | null;
  displayHz: number;
  onDisplayHz: (hz: number) => void;
  denoise: boolean;
  onDenoise: (v: boolean) => void;
  showMessages: boolean;
  onToggleMessages: () => void;
}) {
  const {
    rightPane, messages, sortMode, myGrid, myCall, dialHz, txTarget, operatingMode, filter, setFilter,
    workedCalls, workedCallsAnyBand, workedGrids,
    workedPrefixes, workedDxccPrefixes, workedCqZones, workedStates, txFreq, onSelectFreq, onAnswer,
    decoding, audio, displayHz, onDisplayHz, denoise, onDenoise, showMessages, onToggleMessages,
  } = props;

  // Default "time" mode keeps the existing full-history, per-slot-grouped
  // list untouched; CALL/SNR/DX collapse to one row per station (latest
  // decode wins) and reorder, matching Android's collapseByStation -- see
  // decodeSort.ts for why collapsing isn't also applied to "time".
  const displayMessages = sortMode === "time" ? messages : collapseByStation(messages, sortMode, myGrid);

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
      </div>
      <div className="decode-split">
        <div className="decode-detail">
          {/* Column order matches Android's landscape compact row exactly
              (docs/landscape-layout.md: "the compact row's message slot is
              30 characters ... so SNR / Hz / DT and the badge follow right
              after the message"): the callsign lives in the message text,
              same as Android's compact row -- there's no separate Call
              column there, and no per-row Time column either (the
              time-group header above each block is Android's only "time"
              per decode period, in both layouts).
              No persistent "MESSAGE/SNR/FREQ/ΔT" column-label row either --
              confirmed against a real Android screenshot (the time-group
              divider sits directly under the filter chips, nothing above
              it) and against DecodeRow.kt/DecodeScreen.kt's source, which
              has no such composable. A previous round added one here that
              Android never actually had; removed. */}
          <div className="decode-list">
            {displayMessages.map((m, i) => {
              // Time-group dividers only make sense in "time" mode, where rows
              // stay time-ordered -- once sorted by call/SNR/distance the rows
              // interleave cycles and a divider would be noise (matches
              // Android's showTimeGroupDividers).
              // i === 0 also starts a new block: messages are prepended newest-first,
              // so the current decode period is always at the top of the list, and
              // Android always shows that period's header there too.
              const newBlock =
                sortMode === "time" && (i === 0 || displayMessages[i - 1].utc_ms !== m.utc_ms);
              // Distinct callers decoded in this slot, for the centered
              // divider -- matches Android's "UTC : 16:15:52 · 8 decoded
              // this cycle" header, just per-group instead of once for the
              // newest batch only (every group gets its own count here).
              let groupStationCount = 0;
              if (newBlock) {
                const seen = new Set<string>();
                for (let j = i; j < displayMessages.length && displayMessages[j].utc_ms === m.utc_ms; j++) {
                  seen.add(displayMessages[j].call_from.toUpperCase());
                }
                groupStationCount = seen.size;
              }
              const callUpper = m.call_from.toUpperCase();
              const worked = workedCalls.has(callUpper);
              const rowTime = new Date(m.utc_ms).toISOString().substring(11, 19);
              const prefix = wpxPrefixOf(m.call_from);
              const status = resolveQsoStatus({
                isCq: m.is_cq,
                isToMe: m.to_me,
                isPota: isPotaCq(m.is_cq, m.text),
                isNewDxcc: m.dxcc_prefix != null && !workedDxccPrefixes.has(m.dxcc_prefix),
                isNewZone: m.cq_zone != null && !workedCqZones.has(m.cq_zone),
                isNewState: m.state != null && !workedStates.has(m.state),
                isNewGrid: m.grid.length >= 4 && !workedGrids.has(m.grid.toUpperCase()),
                isNewPrefix: prefix != null && !workedPrefixes.has(prefix),
                isWorkedOtherBand: !worked && workedCallsAnyBand.has(callUpper),
                isWorked: worked,
              });
              return (
                <Fragment key={i}>
                  {newBlock && (
                    <div className="time-sep">
                      <span>
                        {rowTime} UTC · {groupStationCount} station{groupStationCount === 1 ? "" : "s"}
                      </span>
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
                    {/* Zero-padded to 4 digits (Android's getFreq_hz(), "%04.0f") so
                        this column's digits line up row to row regardless of how
                        many digits the offset itself has. */}
                    <span className="decode-row-freq">{formatDecodeFreq(m.freq_hz)}</span>
                    {/* DT: a signed offset, highlighted once it's large enough to be
                        worth the operator's attention (decode timing drifting). A
                        value that rounds to zero renders unsigned ("0.0", never
                        "+0.0"/"-0.0") -- matches Android's formatDecodeDt exactly. */}
                    <span className={"decode-row-dt" + (isDecodeDtNotable(m.time_sec) ? " warn" : "")}>
                      {formatDecodeDt(m.time_sec)}
                    </span>
                    {/* Status pill, matching Android's StatusPill.kt (compact
                        variant): colored background + border + a small dot +
                        the status label -- resolveQsoStatus's full
                        priority-ordered classification (calling me, POTA,
                        new DXCC/zone/state/grid/prefix/band, worked, plain
                        CQ), not just a binary CQ indicator. */}
                    {status && (
                      <span
                        className="status-pill-row"
                        style={{
                          color: QSO_STATUS_META[status].color,
                          background: QSO_STATUS_META[status].bg,
                          borderColor: QSO_STATUS_META[status].border,
                        }}
                      >
                        <span className="status-pill-row-dot" style={{ background: QSO_STATUS_META[status].color }} />
                        {QSO_STATUS_META[status].label}
                      </span>
                    )}
                  </div>
                </Fragment>
              );
            })}
            {displayMessages.length === 0 && (
              <EmptyState filter={filter} operatingMode={operatingMode} />
            )}
          </div>
        </div>
        <div className="decode-waterfall">
          {rightPane === "map" ? (
            <MapScreen
              messages={messages}
              operatingMode={operatingMode}
              workedCalls={workedCalls}
              workedCallsAnyBand={workedCallsAnyBand}
              workedDxccPrefixes={workedDxccPrefixes}
              workedCqZones={workedCqZones}
              myGrid={myGrid}
              myCall={myCall}
              dialHz={dialHz}
              txTarget={txTarget}
            />
          ) : (
            <>
              <WaterfallView txFreq={txFreq} onSelectFreq={onSelectFreq} displayHz={displayHz} />
              <WaterfallStatusStrip
                txFreq={txFreq}
                decoding={decoding}
                audio={audio}
                displayHz={displayHz}
                onDisplayHz={onDisplayHz}
                denoise={denoise}
                onDenoise={onDenoise}
                showMessages={showMessages}
                onToggleMessages={onToggleMessages}
              />
            </>
          )}
        </div>
      </div>
    </>
  );
}

// Bottom info strip under the waterfall, matching Android's WaterfallScreen.kt
// bottom strip layout: Width/NR/MSG toggle chips, the RX input-level meter,
// current TX/cursor frequency, and LIVE/IDLE status. All three are real now:
// Width opens a small popover (the same spectrumWidth range/editing Android's
// own Width chip offers, and the same value Settings -> Audio edits -- see
// WaterfallScreen.kt's own comment: "Opens the same Spectrum Width setting as
// Settings -> Radio & Audio"); NR toggles the backend's display-only
// band-tilt-flattening (wf.rs's WfConfig.denoise, ported from the shared
// ft8af_magnitudes_to_display's denoise branch -- never touches the decoder,
// which runs its own FFT independently, same as Android's own NR); MSG
// toggles drawSignalLabels, which already existed and ran unconditionally --
// this just makes it optional, defaulting on to match both Android's
// markMessage default and this pipeline's prior always-on behavior.
function WaterfallStatusStrip({
  txFreq,
  decoding,
  audio,
  displayHz,
  onDisplayHz,
  denoise,
  onDenoise,
  showMessages,
  onToggleMessages,
}: {
  txFreq: number;
  decoding: boolean;
  audio: InputLevelEvent | null;
  displayHz: number;
  onDisplayHz: (hz: number) => void;
  denoise: boolean;
  onDenoise: (v: boolean) => void;
  showMessages: boolean;
  onToggleMessages: () => void;
}) {
  const [widthOpen, setWidthOpen] = useState(false);
  return (
    <div className="wf-status-strip">
      <button
        className={"wf-toggle-chip" + (widthOpen ? " active" : "")}
        onClick={() => setWidthOpen(true)}
        title="Spectrum width (Hz) -- the same setting as Settings → Audio"
      >
        Width
      </button>
      {widthOpen && (
        <SpectrumWidthDialog
          displayHz={displayHz}
          onSave={(hz) => { onDisplayHz(hz); setWidthOpen(false); }}
          onCancel={() => setWidthOpen(false)}
        />
      )}
      <button
        className={"wf-toggle-chip" + (denoise ? " active" : "")}
        onClick={() => onDenoise(!denoise)}
        title="Noise reduction: flattens broadband tilt in the waterfall display only (never affects decoding)"
      >
        NR
      </button>
      <button
        className={"wf-toggle-chip" + (showMessages ? " active" : "")}
        onClick={onToggleMessages}
        title="Label decoded callsigns on the waterfall at their frequency"
      >
        MSG
      </button>
      <InputLevelIndicator decoding={decoding} audio={audio} />
      <div className="spacer" />
      <span className="wf-status-freq">{txFreq} Hz</span>
      <span className={"wf-status-live" + (decoding ? "" : " off")}>{decoding ? "LIVE" : "IDLE"}</span>
    </div>
  );
}

// Matches Android's actual "Spectrum Width" dialog (a centered Cancel/Save
// modal with a bounded number field, not a live-drag slider) -- confirmed
// against a real screenshot. Committing only on Save, rather than firing
// set_wf_display_hz on every drag tick, also avoids spamming the backend
// with a WfProcessor rebuild per pixel of drag (the previous live-slider
// popover could flood the engine with a config rebuild on every intermediate
// value during a single drag).
function SpectrumWidthDialog({
  displayHz,
  onSave,
  onCancel,
}: {
  displayHz: number;
  onSave: (hz: number) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(String(Math.round(displayHz)));
  const parsed = parseInt(draft, 10);
  const valid = Number.isFinite(parsed) && parsed >= WF_MIN_DISPLAY_HZ && parsed <= WF_MAX_DISPLAY_HZ;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal spectrum-width-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>Spectrum Width</h3>
        <label className="spectrum-width-field">
          <span className="spectrum-width-hint">{WF_MIN_DISPLAY_HZ}–{WF_MAX_DISPLAY_HZ} Hz</span>
          <input
            type="number"
            inputMode="numeric"
            min={WF_MIN_DISPLAY_HZ}
            max={WF_MAX_DISPLAY_HZ}
            step={100}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            autoFocus
          />
        </label>
        <div className="actions">
          <button className="modal-text-btn" onClick={onCancel}>Cancel</button>
          <button
            className="modal-text-btn modal-text-btn-accent"
            disabled={!valid}
            onClick={() => valid && onSave(parsed)}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

// Sum of QSO_TABLE_WEIGHTS, for turning each weight into a <col> percentage.
const QSO_TABLE_WEIGHT_TOTAL = QSO_TABLE_WEIGHTS.reduce((a, b) => a + b, 0);

/**
 * Per-row "⋮" actions menu, matching Android's `QsoActionsMenu` exactly
 * (`LogbookScreen.kt`): a three-dot button opening Edit/Delete, replacing
 * the lone "✕" delete button that was here before -- Android has no
 * standalone delete button at all, only this menu.
 *
 * The dropdown itself renders through a portal into document.body, positioned
 * from the trigger button's screen coordinates, rather than as a normal
 * absolutely-positioned child. The table's scroll container (.log-table-list)
 * is a real overflow:auto box, and a child positioned absolute/relative to an
 * ancestor inside it gets clipped at that box's edge -- a dropdown opened on
 * any row not near the very bottom of the visible area would be cut off.
 */
function QsoActionsMenu({ onEdit, onDelete }: { onEdit: () => void; onDelete: () => void }) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);

  function toggle(e: MouseEvent<HTMLButtonElement>) {
    e.stopPropagation();
    if (open) {
      setOpen(false);
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect();
    const MENU_WIDTH = 120;
    if (rect) setPos({ top: rect.bottom + 2, left: rect.right - MENU_WIDTH });
    setOpen(true);
  }

  return (
    <div className="qso-actions-menu-wrap">
      <button
        ref={triggerRef}
        className="icon-button qso-actions-trigger"
        onClick={toggle}
        title="Actions"
      >
        ⋮
      </button>
      {open && pos && createPortal(
        <>
          {/* Backdrop: closes the menu on an outside click, matching
              DropdownMenu's onDismissRequest. */}
          <div className="qso-actions-backdrop" onClick={() => setOpen(false)} />
          <div className="qso-actions-menu" style={{ top: pos.top, left: pos.left }}>
            <button
              className="qso-actions-item"
              onClick={() => {
                setOpen(false);
                onEdit();
              }}
            >
              <span className="qso-actions-icon edit">✎</span> Edit
            </button>
            <button
              className="qso-actions-item delete"
              onClick={() => {
                setOpen(false);
                onDelete();
              }}
            >
              <span className="qso-actions-icon delete">🗑</span> Delete
            </button>
          </div>
        </>,
        document.body,
      )}
    </div>
  );
}

/**
 * Logbook Stats tab, matching Android's StatsTab (`LogbookScreen.kt`): big
 * stat cards, a Best DX card, the Worked All Continents (WAC) award, a band
 * donut chart, award progress bars, a grid-square coverage heatmap, and a
 * signal trend sparkline. `stats` is null while the first fetch is in
 * flight (see LogScreen's own `useEffect`); `rows` is LogScreen's own
 * Recent-tab fetch (capped by LOG_FETCH_LIMIT), reused here only for the
 * sparkline, which doesn't need the whole log -- see SignalSparklineCard.
 */
function StatsTab({ stats, rows }: { stats: LogStats | null; rows: QsoRecord[] }) {
  if (!stats) {
    return <div className="stats-loading muted">Loading stats…</div>;
  }
  const wac = workedAllContinents(stats.continent_codes);
  const bandTotal = stats.band_counts.reduce((sum, [, n]) => sum + n, 0);
  // db.rs's worked_states_all resolves via us_states::worked_state_for, a
  // superset of the 50 WAS states that also recognizes "DC" -- filter down
  // to the real award count, matching Android's own workedStates reducer
  // (see wasStatesWorked's doc comment).
  const statesWorked = wasStatesWorked(stats.states_worked);
  return (
    <div className="stats-tab">
      <div className="stat-card-row">
        <StatCard label="Total QSOs" value={stats.total_qsos} color="var(--accent)" />
        <StatCard label="DXCC Entities" value={stats.dxcc_entities} color="var(--signal)" />
      </div>
      <div className="stat-card-row">
        <StatCard label="CQ Zones" value={stats.cq_zones} color="var(--status-new)" />
        <StatCard label="ITU Zones" value={stats.itu_zones} color="var(--yellow)" />
      </div>

      {stats.best_dx && (
        <>
          <div className="stats-section-header">Best DX</div>
          <BestDxCard bestDx={stats.best_dx} />
        </>
      )}

      <div className="stats-section-header">Worked All Continents</div>
      <WacCard wac={wac} />

      {stats.band_counts.length > 0 && (
        <>
          <div className="stats-section-header">Band Distribution</div>
          <BandDonutCard bandCounts={stats.band_counts} total={bandTotal} />
        </>
      )}

      <div className="stats-section-header">Award Progress</div>
      <AwardProgressBar
        label="DXCC Mixed"
        current={stats.dxcc_entities}
        total={340}
        gradient={["var(--signal)", "var(--ok)"]}
      />
      <AwardProgressBar
        label="Worked All States"
        current={statesWorked.length}
        total={50}
        gradient={["var(--accent)", "var(--ok)"]}
      />
      <AwardProgressBar
        label="VUCC Grid Squares"
        current={stats.grid_squares}
        total={100}
        gradient={["var(--status-new)", bandColor("12m")]}
      />
      <AwardProgressBar
        label="DXCC Challenge"
        current={stats.dxcc_entities * Math.max(stats.band_counts.length, 1)}
        total={1000}
        gradient={["var(--accent)", "var(--yellow)"]}
      />

      <div className="stats-section-header">Grid Coverage</div>
      <GridHeatmapCard workedFields={stats.worked_grid_fields} />

      <div className="stats-section-header">Signal Trend</div>
      <SignalSparklineCard rows={rows} />
    </div>
  );
}

function StatCard({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div className="glass-card stat-card">
      <div className="stat-card-value mono" style={{ color }}>{value}</div>
      <div className="stat-card-label muted">{label}</div>
    </div>
  );
}

function BestDxCard({ bestDx }: { bestDx: BestDx }) {
  return (
    <div className="glass-card best-dx-card">
      <div className="best-dx-info">
        <div className="best-dx-call mono">{bestDx.callsign}</div>
        {bestDx.grid && <div className="best-dx-grid muted">{bestDx.grid}</div>}
      </div>
      <div className="best-dx-distance mono">{formatDistanceMi(bestDx.distance_km)}</div>
    </div>
  );
}

function WacCard({ wac }: { wac: WacProgress }) {
  return (
    <div className="glass-card wac-card">
      <div className="wac-header">
        <span className={wac.isComplete ? "wac-status complete" : "wac-status"}>
          {wac.isComplete ? "All six continents worked \u{1F30D}" : `${wac.workedCount} / ${wac.total} continents`}
        </span>
        <span className="muted mono">{wac.workedCount} / {wac.total}</span>
      </div>
      <div className="wac-chips">
        {wac.chips.map((chip) => (
          <span key={chip.code} className={chip.worked ? "continent-chip worked" : "continent-chip"}>
            {chip.code}
          </span>
        ))}
      </div>
    </div>
  );
}

function BandDonutCard({ bandCounts, total }: { bandCounts: [string, number][]; total: number }) {
  const safeTotal = total || 1;
  const strokeWidth = 18;
  const r = 50;
  const circumference = 2 * Math.PI * r;
  const gapLen = (3 / 360) * circumference;
  let offset = 0;
  return (
    <div className="glass-card band-donut-card">
      <svg viewBox="0 0 120 120" className="band-donut-svg" aria-hidden>
        <g transform="rotate(-90 60 60)">
          {bandCounts.map(([band, n]) => {
            const fraction = n / safeTotal;
            const len = Math.max(0, fraction * circumference - gapLen);
            const dashoffset = -offset;
            offset += fraction * circumference;
            return (
              <circle
                key={band}
                cx={60}
                cy={60}
                r={r}
                fill="none"
                stroke={bandColor(band)}
                strokeWidth={strokeWidth}
                strokeDasharray={`${len} ${circumference - len}`}
                strokeDashoffset={dashoffset}
                strokeLinecap="round"
              />
            );
          })}
        </g>
      </svg>
      <div className="band-donut-legend">
        {bandCounts.slice(0, 7).map(([band, n]) => (
          <div key={band} className="band-donut-legend-row">
            <span className="band-donut-dot" style={{ background: bandColor(band) }} />
            <span className="mono">{band}</span>
            <span className="muted mono">{n}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function AwardProgressBar({
  label, current, total, gradient, remaining,
}: {
  label: string;
  current: number;
  total: number;
  gradient: [string, string];
  remaining?: string[];
}) {
  const fraction = Math.max(0, Math.min(1, current / Math.max(total, 1)));
  return (
    <div className="glass-card award-progress-card">
      <div className="award-progress-header">
        <span>{label}</span>
        <span className="muted mono">{current} / {total}</span>
      </div>
      <div className="award-progress-track">
        <div
          className="award-progress-fill"
          style={{ width: `${fraction * 100}%`, background: `linear-gradient(to right, ${gradient[0]}, ${gradient[1]})` }}
        />
      </div>
      {remaining && remaining.length > 0 && <NeededStatesRow remaining={remaining} accent={gradient[0]} />}
    </div>
  );
}

function NeededStatesRow({ remaining, accent }: { remaining: string[]; accent: string }) {
  const { shown, overflow } = neededStatesPreview(remaining, NEEDED_STATES_PREVIEW_MAX);
  return (
    <div className="needed-states">
      <div className="needed-states-label muted">STATES NEEDED</div>
      <div className="needed-states-chips">
        {shown.map((s) => (
          <span key={s} className="state-chip" style={{ color: accent, borderColor: accent }}>{s}</span>
        ))}
        {overflow > 0 && (
          <span className="state-chip" style={{ color: accent, borderColor: accent }}>+{overflow}</span>
        )}
      </div>
    </div>
  );
}

function GridHeatmapCard({ workedFields }: { workedFields: string[] }) {
  const cells = useMemo(() => buildGridHeatmapCells(new Set(workedFields)), [workedFields]);
  return (
    <div className="glass-card grid-heatmap-card">
      {cells.map((row, i) => (
        <div key={i} className="grid-heatmap-row">
          {row.map((cell) => (
            <span
              key={cell.field}
              className={cell.isWorked ? "grid-heatmap-cell worked" : "grid-heatmap-cell"}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

/**
 * Android's own SignalSparkline is a synthesized placeholder, not real SNR
 * data -- "QSLCallsignRecord does not carry SNR, so we synthesize a coarse
 * trend from the per-record index... until SNR is persisted on the QSO log
 * row" (LogbookScreen.kt's own doc comment). Ported with the same formula
 * and the same honesty: this is not a real signal trend on either platform
 * yet, not something desktop is faking that Android has real.
 */
function SignalSparklineCard({ rows }: { rows: QsoRecord[] }) {
  if (rows.length === 0) {
    return <div className="glass-card signal-sparkline-card signal-sparkline-empty muted mono">No QSOs yet</div>;
  }
  const data = rows.slice(-30).map((_, i) => Math.min(5, Math.max(-25, -15 + (i % 20) * 1.2)));
  const w = 300;
  const h = 72;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const range = Math.max(max - min, 1);
  const stepX = data.length > 1 ? w / (data.length - 1) : w;
  const points = data.map((v, i) => [i * stepX, h - ((v - min) / range) * h] as const);
  const linePath = points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const fillPath = `${linePath} L${w},${h} L0,${h} Z`;
  return (
    <div className="glass-card signal-sparkline-card">
      <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className="signal-sparkline-svg" aria-hidden>
        <path d={fillPath} fill="var(--signal-soft)" />
        <path d={linePath} fill="none" stroke="var(--signal)" strokeWidth={2} strokeLinecap="round" />
      </svg>
    </div>
  );
}

/**
 * Logbook Awards tab, matching Android's AwardsTab (`LogbookScreen.kt`): six
 * award cards (DXCC Mixed, WAS, WAZ, WAC, VUCC, IOTA), each a progress bar
 * plus description. IOTA's current count is always 0 -- desktop (like
 * Android, per its own "Not tracked in current DB" comment) has no
 * island-tracking data to compute it from; listed honestly as not-yet-
 * tracked, not hidden or faked. DXCC Mixed's total genuinely differs from
 * the Stats tab's own DXCC Mixed bar (100 here vs. 340 there) -- that
 * mismatch exists in Android's own source, not introduced by this port.
 */
function AwardsTab({ stats }: { stats: LogStats | null }) {
  if (!stats) {
    return <div className="stats-loading muted">Loading awards…</div>;
  }
  const wac = workedAllContinents(stats.continent_codes);
  const statesWorked = wasStatesWorked(stats.states_worked);
  const awards: {
    name: string;
    description: string;
    current: number;
    total: number;
    color: string;
    remaining?: string[];
  }[] = [
    {
      name: "DXCC Mixed",
      description: "Work and confirm 100 DXCC entities on any band/mode",
      current: stats.dxcc_entities,
      total: 100,
      color: "var(--signal)",
    },
    {
      name: "WAS",
      description: "Work all 50 US states confirmed",
      current: statesWorked.length,
      total: 50,
      color: "var(--accent)",
      remaining: neededStates(statesWorked),
    },
    {
      name: "WAZ",
      description: "Work all 40 CQ zones confirmed",
      current: stats.cq_zones,
      total: 40,
      color: "var(--status-new)",
    },
    {
      name: "WAC",
      description: "Worked All Continents -- contact all six continents",
      current: wac.workedCount,
      total: wac.total,
      color: "var(--status-new)", // Android: Band10m, same hex as StatusNew
    },
    {
      name: "VUCC",
      description: "VHF/UHF Century Club -- 100 grid squares on a single band",
      current: stats.grid_squares,
      total: 100,
      color: bandColor("12m"), // Android: Band12m
    },
    {
      name: "IOTA",
      description: "Islands on the Air -- work stations on designated islands",
      current: 0,
      total: 100,
      color: "var(--yellow)", // Android: Band17m, same hex as StatusWarn
    },
  ];
  return (
    <div className="awards-tab">
      {awards.map((award) => (
        <AwardCard key={award.name} {...award} />
      ))}
    </div>
  );
}

function AwardCard({
  name, description, current, total, color, remaining,
}: {
  name: string;
  description: string;
  current: number;
  total: number;
  color: string;
  remaining?: string[];
}) {
  const fraction = Math.max(0, Math.min(1, current / Math.max(total, 1)));
  return (
    <div className="glass-card award-card">
      <div className="award-icon-circle" style={{ color, borderColor: color }}>{name[0]}</div>
      <div className="award-card-body">
        <div className="award-card-header">
          <span>{name}</span>
          <span className="muted mono">{current} / {total}</span>
        </div>
        <div className="award-card-desc muted">{description}</div>
        <div className="award-progress-track">
          <div className="award-progress-fill" style={{ width: `${fraction * 100}%`, background: color }} />
        </div>
        {remaining && remaining.length > 0 && <NeededStatesRow remaining={remaining} accent={color} />}
      </div>
    </div>
  );
}

function LogScreen({ refreshKey, myGrid }: { refreshKey: number; myGrid: string }) {
  // Android's Logbook is three sub-tabs (Stats / Recent / Awards); desktop
  // only has Recent's equivalent (the QSO table) built. Stats and Awards
  // are real, separate bodies of work (VUCC/WAS/WAC award tracking, a
  // "Best DX" card, etc.) -- not a display-parity fix -- so they show
  // honestly as not-yet-built rather than faking content, same pattern as
  // the Map/POTA tabs before they were real.
  const [logbookTab, setLogbookTab] = useState<"stats" | "recent" | "awards">("recent");
  const [rows, setRows] = useState<QsoRecord[]>([]);
  const [count, setCount] = useState(0);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ConfirmFilter>("all");
  // Stats/Awards data: fetched independently of rows/count above, which are
  // search/filter-scoped for the Recent table -- Android's own stats loader
  // (LogbookScreen.kt's LaunchedEffect(refreshKey)) always aggregates the
  // *whole* log regardless of the Recent tab's search box, so this only
  // depends on refreshKey (bumped after an import) and myGrid (for Best DX).
  const [stats, setStats] = useState<LogStats | null>(null);
  useEffect(() => {
    api.logStats(myGrid).then(setStats);
  }, [refreshKey, myGrid]);
  // Edit-QSO dialog target: undefined = closed, null = new QSO, record = edit.
  const [editing, setEditing] = useState<QsoRecord | null | undefined>(undefined);
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
  // 32, not 26: matches Android's QsoTableCell padding (6dp horizontal, 7dp
  // vertical -- i.e. real visible row padding, not just a tight single-line
  // squeeze) now that the table actually carries per-cell borders/padding.
  const ROW_HEIGHT = 32;
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

  // Sorting now happens in SQL, before `LIMIT` truncates the result (see
  // db.rs's `Db::search_qsos` doc comment) -- `rows` already arrives in the
  // right order, so there's no client-side re-sort to memoize here anymore.
  // A client-side sort of only the fetched (capped) page was the actual bug
  // the user caught: it could only ever reorder whichever `limit` rows
  // happened to be fetched, never surface a row outside that page no matter
  // which column was sorted.
  const sortedRows = rows;
  const virtualWindow = computeVirtualWindow(
    sortedRows.length, ROW_HEIGHT, scrollTop, viewportHeight, OVERSCAN,
  );
  const visibleRows = sortedRows.slice(virtualWindow.startIndex, virtualWindow.endIndex);

  function refresh() {
    // See LOG_FETCH_LIMIT's own comment for why this is capped, not
    // unconditional, and why that's still correct-enough for the common
    // case rather than silently wrong.
    api.searchLog(search, filter, sortColumn, sortAscending, LOG_FETCH_LIMIT, 0).then(setRows);
    api.logCount().then(setCount);
  }
  // Re-query whenever the live search box, the confirmation filter, or the
  // sort column/direction changes (the backend now does the actual sort --
  // see above), or App()'s refreshKey bumps after an ADIF import completes
  // (see LogScreen's own doc comment at the top of this file for why that
  // state lives in App(), not here).
  useEffect(refresh, [search, filter, sortColumn, sortAscending, refreshKey]);

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
      <div className="log-subtabs">
        {(["stats", "recent", "awards"] as const).map((tab) => (
          <button
            key={tab}
            className={logbookTab === tab ? "log-subtab active" : "log-subtab"}
            onClick={() => setLogbookTab(tab)}
          >
            {tab === "stats" ? "Stats" : tab === "recent" ? "Recent" : "Awards"}
          </button>
        ))}
      </div>
      {logbookTab === "stats" && <StatsTab stats={stats} rows={rows} />}
      {logbookTab === "awards" && <AwardsTab stats={stats} />}
      {logbookTab === "recent" && (
        <>
      <div className="logbar">
        <strong>
          {qsoCountLabel(count, rows.length, isLogFiltered(search, filter) || rows.length < count)}
        </strong>{" "}
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
      {/* The header lives in its own table, outside the scrolling container
          below, rather than a sticky <thead> inside one shared table --
          position:sticky on a <th> inside a real <table> is unreliable in
          WebKitGTK (confirmed live, twice: the header text and the row
          scrolled underneath it painted on top of each other instead of the
          header cleanly covering it, regardless of border-collapse). Two
          tables sharing the same <colgroup> is the standard, browser-quirk-
          free way to keep a header's columns aligned with a scrolling
          body's. */}
      <table className="qso-table qso-table-header">
        <colgroup>
          {QSO_TABLE_WEIGHTS.map((w, i) => (
            <col key={i} style={{ width: `${(w / QSO_TABLE_WEIGHT_TOTAL) * 100}%` }} />
          ))}
          <col style={{ width: "36px" }} />
        </colgroup>
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
      </table>
      <div
        className="decode-list log-table-list"
        ref={scrollRef}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
      <table className="qso-table">
        <colgroup>
          {QSO_TABLE_WEIGHTS.map((w, i) => (
            <col key={i} style={{ width: `${(w / QSO_TABLE_WEIGHT_TOTAL) * 100}%` }} />
          ))}
          <col style={{ width: "36px" }} />
        </colgroup>
        <tbody>
          {virtualWindow.topSpacerPx > 0 && (
            <tr aria-hidden style={{ height: virtualWindow.topSpacerPx }}>
              <td colSpan={QSO_TABLE_COLUMNS.length + 1} style={{ border: "none", padding: 0 }} />
            </tr>
          )}
          {visibleRows.map((r) => {
            const status = qsoStatusLabel(r);
            return (
              <tr key={r.id}>
                {/* Confirmation shows via the Status pill and QSL marks only
                    (matching Android) -- the row itself is never tinted.
                    Matches Android's QsoTableRow exactly: no per-cell/whole-
                    row click-to-edit -- Edit is only reachable through the
                    three-dot menu (QsoActionsMenu) in the last column. */}
                <td>{adifToDateInput(r.qso_date)}</td>
                <td>{formatHhmm(r.time_on)}</td>
                <td className="call-cell">{r.call}</td>
                <td className="grid-cell">{r.gridsquare}</td>
                <td className="muted">{logbookPlace(r) ?? ""}</td>
                <td>{r.freq}</td>
                <td>{r.mode}</td>
                <td>{displayReport(r.rst_sent)}</td>
                <td>{displayReport(r.rst_rcvd)}</td>
                <td>
                  <div className="status-cell">
                    {status && <span className="status-pill conf">{status}</span>}
                    {qsoSyncLabels(r).map((label) => (
                      <span key={label} className="sync-chip">{label}</span>
                    ))}
                  </div>
                </td>
                <td>
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
                <td className="qso-actions-cell">
                  <QsoActionsMenu onEdit={() => setEditing(r)} onDelete={() => remove(r.id)} />
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
        </>
      )}
      {editing !== undefined && (
        <EditQsoDialog
          record={editing}
          onCancel={() => setEditing(undefined)}
          onSave={saveEdit}
        />
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

/**
 * Export sheet, opened from the Logbook top bar's Share icon -- matches
 * Android's `ExportLogSheet.kt`: a date-range filter (empty = open bound,
 * same semantics as the inline row this replaced) over one "Export ADIF"
 * action. Android offers a separate "Save" vs. "Share" button because its
 * export goes through Android's file-provider/share-sheet machinery; desktop
 * already has one working export path (a browser-style download, wired up
 * when this was still an inline `.logbar` row), so one button covers both.
 */
function ExportLogSheet(props: {
  rangeStart: string;
  rangeEnd: string;
  onRangeStart: (v: string) => void;
  onRangeEnd: (v: string) => void;
  onExport: () => void;
  onClose: () => void;
}) {
  const { rangeStart, rangeEnd, onRangeStart, onRangeEnd, onExport, onClose } = props;
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Export QSOs</h3>
          <button className="modal-close" aria-label="Close" onClick={onClose}>✕</button>
        </div>
        <div className="field">
          <label>Date range (optional)</label>
          <div className="row">
            <input
              className="date"
              type="date"
              value={rangeStart}
              onChange={(e) => onRangeStart(e.target.value)}
              aria-label="Export start date"
            />
            <span className="muted">→</span>
            <input
              className="date"
              type="date"
              value={rangeEnd}
              onChange={(e) => onRangeEnd(e.target.value)}
              aria-label="Export end date"
            />
          </div>
        </div>
        <div className="actions">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" onClick={onExport}>Export ADIF</button>
        </div>
      </div>
    </div>
  );
}

/**
 * Android's Upload icon syncs the whole log to whichever third-party logging
 * services (Cloudlog, QRZ, WRL) the operator has enabled
 * (`ThirdPartyService.syncAllQSOs`). Desktop has no such integration yet --
 * a separate, already-documented gap (see
 * docs/desktop-decode-android-parity.md) -- so the icon is honest about that
 * instead of silently doing nothing or faking a sync that never happened.
 */
function UploadSyncNoticeDialog(props: { onDone: () => void }) {
  return (
    <div className="modal-overlay" onClick={props.onDone}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Sync to logging services</h3>
        <p className="muted">
          Uploading QSOs to Cloudlog, QRZ, or WRL isn't available on desktop yet -- this is
          a planned feature, not configured here.
        </p>
        <div className="actions">
          <button className="primary" onClick={props.onDone}>OK</button>
        </div>
      </div>
    </div>
  );
}

// First-run callsign/grid table seed (fcc_grid.rs), same structural shape as
// ImportProgressDialog/ImportCompleteDialog/ImportErrorDialog above -- reuses
// the same .import-dialog/.bar/.count/.working classes. Two distinct phases:
// "downloading" (byte progress; true per-record counting isn't possible
// while the file is still a compressed blob in flight) and "processing"
// (the FCC->Gazetteer cross-reference + DB insert, where the real total
// record count is known and each tick names the callsign just processed).
function FccGridSeedDialog(props: { progress: FccSeedProgress }) {
  const { phase, processed, total, current_callsign } = props.progress;
  const pct = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0;
  const mb = (bytes: number) => (bytes / 1_000_000).toFixed(1);
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>Creating US Grid Table</h3>
        <div className="bar">
          <div style={{ width: `${pct}%` }} />
        </div>
        <div className="count">
          {phase === "downloading"
            ? total > 0
              ? `Downloading… ${mb(processed)} of ${mb(total)} MB`
              : `Downloading… ${mb(processed)} MB`
            : `Processing Callsign: ${current_callsign || "…"} (${processed.toLocaleString()} of ${total.toLocaleString()})`}
        </div>
        <div className="working">Working…</div>
      </div>
    </div>
  );
}

function FccGridSeedCompleteDialog(props: { summary: FccSeedSummary; onDone: () => void }) {
  const { summary, onDone } = props;
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>US Grid Table Ready</h3>
        <div className="import-summary">
          <div>Added: {summary.seeded.toLocaleString()} stations</div>
          <div>Active US licenses found: {summary.active_licenses.toLocaleString()}</div>
        </div>
        <button className="primary import-done" onClick={onDone}>Done</button>
      </div>
    </div>
  );
}

function FccGridSeedErrorDialog(props: { message: string; onDone: () => void }) {
  return (
    <div className="modal-overlay">
      <div className="modal import-dialog">
        <h3>US grid table download failed</h3>
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

// Row count for the callsign/grid cache (fcc_grid.rs, schema v7) + a manual
// re-run button, matching CustomBandsPanel's own local-state/refresh
// pattern. The seed's live progress/completion dialog itself is driven by
// App()'s own passive event listener (not this panel), so it shows up
// correctly regardless of whether this screen is even open when a seed
// (first-run or manual) is running.
function CallsignGridPanel(props: { onUpdate: () => void; busy: boolean }) {
  const { onUpdate, busy } = props;
  const [count, setCount] = useState<number | null>(null);

  function refresh() {
    api.callsignQthCount().then(setCount);
  }
  useEffect(refresh, []);
  // Pick up the row count once a seed (either trigger) finishes.
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
          <span>
            {count == null
              ? "Checking…"
              : `${count.toLocaleString()} station${count === 1 ? "" : "s"} known`}
          </span>
          <button className="primary" onClick={onUpdate} disabled={busy}>
            {busy ? "Updating…" : "Update now"}
          </button>
        </div>
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
  inputGain: number;
  onInputGain: (v: number) => void;
  txGain: number;
  onTxGain: (v: number) => void;
  /** Shared with the Decode waterfall strip's own Width control (same
   *  top-level state in App()) so a change made here is immediately live
   *  on the waterfall, not just on next reload -- see WaterfallStatusStrip's
   *  doc comment for why this one value is lifted instead of locally fetched. */
  displayHz: number;
  onDisplayHz: (hz: number) => void;
  /** Re-runs the FCC/Census grid-table seed (fcc_grid.rs) -- the same
   *  function the backend's first-run auto-start kicks off; this just gives
   *  the operator a manual way to refresh it later (the FCC data changes
   *  weekly). The live dialog is driven by App()'s own passive event
   *  listener, not by this call directly -- see that listener's comment. */
  onUpdateGridTable: () => void;
  /** Whether a grid-table seed (either trigger) is currently running, to
   *  grey out the button instead of letting the operator start a second
   *  one concurrently. */
  gridSeedBusy: boolean;
}) {
  const {
    onStatus, clock, onSettingsSaved, onRigLabel, onBandsChanged,
    huntPriority, onHuntPriority, huntAvoidPileups, onHuntAvoidPileups, huntMinSnr, onHuntMinSnr,
    inputGain, onInputGain, txGain, onTxGain, displayHz, onDisplayHz,
    onUpdateGridTable, gridSeedBusy,
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
  // WfConfig::default() on the Rust side: hann / 2048 / 6. display_hz and
  // denoise round-trip here (kept synced from the displayHz prop, and
  // denoise read once from config below) even with no dedicated edit control
  // for either *in this object* -- Width has its own field below bound to
  // the shared displayHz prop instead, and NR has no Settings UI at all
  // (Android doesn't duplicate it there either) -- purely so a save here
  // (window/fft_size/avg) can't silently clobber either one via the
  // full-object SetWaterfallConfig path.
  const [wfCfg, setWfCfg] = useState<WaterfallConfig>({
    window: "hann",
    fft_size: 2048,
    avg: 6,
    display_hz: displayHz,
    denoise: false,
  });
  // Keep wfCfg.display_hz mirroring the shared prop so a window/fft_size/avg
  // save always sends the live Width, even if it changed via the waterfall
  // strip's own control (or Settings' own field below) since this mounted.
  useEffect(() => {
    setWfCfg((prev) => ({ ...prev, display_hz: displayHz }));
  }, [displayHz]);
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
    // back through applyWfCfg. display_hz is NOT fetched here -- it's kept
    // synced from the shared displayHz prop (see the effect above) rather
    // than independently re-fetched, so this panel and the waterfall strip
    // can never show two different "current" values for the same setting.
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
          <div className="field">
            <label>RX input gain: {inputGain}%</label>
            <input
              type="range"
              aria-label="RX input gain, percent"
              min={0}
              max={100}
              step={1}
              value={inputGain}
              onChange={(e) => onInputGain(parseInt(e.target.value, 10))}
            />
            <span className="muted" style={{ fontSize: 12 }}>
              Post-ADC software trim — some bands are noisier than others.
            </span>
          </div>
          <div className="field">
            <label>TX output level (drive): {txGain}%</label>
            <input
              type="range"
              aria-label="TX output level (drive), percent"
              min={0}
              max={100}
              step={1}
              value={txGain}
              onChange={(e) => onTxGain(parseInt(e.target.value, 10))}
            />
            <span className="muted" style={{ fontSize: 12 }}>
              Lower until ALC stops pinning.
            </span>
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
              min={WF_MIN_DISPLAY_HZ}
              max={WF_MAX_DISPLAY_HZ}
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

// Opened from the tab bar's "More" button, matching Android's
// operate-controls drawer toggle. Call CQ/STOP and Hunt stay on TopBar
// (Android's CqHuntRow); everything else Android's drawer shows below that
// row is ported here, duplicating TopBar's own Band/TUNE controls on
// purpose -- matching Android, which shows BandModeRow and the Tune button
// in its drawer even though a band/mode control also sits in the screen's
// title row. TX volume is ported here too (TxVolumeRow) *and* stays in
// Settings -> Audio for discoverability -- Android only shows it in the
// drawer, but desktop had already put it in Settings this round and nothing
// said to remove it from there. This entirely replaces the previous
// desktop-only content (plain-text RX/TX/stage/target/message readout, a
// manual Tx1-5 stage override, a separate Stop TX button), none of which
// had an Android equivalent.
//
// DX (DXpedition/Hound mode) is shown as a real, visible button matching
// Android's layout, but disabled: Android's startHoundMode/stopHoundMode is
// a whole separate Fox/Hound TX sequencer, not a UI toggle -- same
// real-on-air-stakes category as Field Day. Showing a disabled button with
// an explanatory title is the honest way to represent "present in Android,
// not implemented here yet" -- the alternative (a working-looking button
// that silently does nothing, or inventing a half-built sequencer) would be
// worse than either omitting it or saying so (see
// docs/desktop-decode-android-parity.md).
//
// Android's TxPeriodControl has no "no preference" third state (always
// exactly one of 1st/2nd selected, defaulting to 1st/even) -- but that would
// mean every CQ here waits for a specific parity before keying at all,
// changing today's "key up at the next free slot" default. AUTO (the
// existing default, preferred_tx_parity == null) is a deliberate addition
// so the period picker is opt-in rather than a default-behavior change.
function TxBar(props: {
  txState: TxStateEvent | null;
  onSetTxPeriod: (parity: number | null) => void;
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  operatingMode: OperatingMode;
  onMode: (m: OperatingMode) => void;
  onToggleTune: () => void;
  txGain: number;
  onTxGain: (v: number) => void;
}) {
  const {
    txState, onSetTxPeriod, bands, dialHz, onBand, operatingMode, onMode,
    onToggleTune, txGain, onTxGain,
  } = props;
  const preferredParity = txState?.preferred_tx_parity ?? null;
  const tuning = txState?.tuning ?? false;
  const transmitting = txState?.transmitting ?? false;
  return (
    <div className="txbar">
      {/* Row 1 -- Band & Mode, matching Android's BandModeRow. Same
          BandModePicker as TopBar's title, just the "card" layout -- the
          full "14.074 MHz · 20m · FT8" label Android's TxStrip/drawer rows
          use (TopBar's title deliberately uses the shorter band+mode-only
          form instead, no room there for frequency -- see
          FT8AFApp.kt's decodeTitleLabel). */}
      <div className="txbar-row">
        <BandModePicker
          variant="card"
          bands={bands}
          dialHz={dialHz}
          onBand={onBand}
          mode={operatingMode}
          onMode={onMode}
        />
      </div>

      {/* Row 2 -- TX period / Tune / DX, matching Android's PeriodTuneDxRow. */}
      <div className="txbar-row">
        <div className="tx-period-control" title="Which half of the 15s cycle a new CQ/QSO starts keying in">
          <span className="tx-period-label">TX PERIOD</span>
          <div className="tx-period-segments">
            <button
              className={preferredParity === null ? "active" : ""}
              onClick={() => onSetTxPeriod(null)}
            >
              Auto
            </button>
            <button
              className={preferredParity === 0 ? "active" : ""}
              onClick={() => onSetTxPeriod(0)}
            >
              1st (even)
            </button>
            <button
              className={preferredParity === 1 ? "active" : ""}
              onClick={() => onSetTxPeriod(1)}
            >
              2nd (odd)
            </button>
          </div>
        </div>
        <button
          className={"pill pill-dark" + (tuning ? " pill-tune-active" : "")}
          onClick={onToggleTune}
          disabled={!tuning && transmitting}
          title="Transmit a steady carrier to tune the antenna, without calling CQ"
        >
          TUNE
        </button>
        <button
          className="pill pill-dark"
          disabled
          title="DXpedition/Hound mode -- not yet implemented on desktop (needs its own Fox/Hound TX sequencer, see docs/desktop-decode-android-parity.md)"
        >
          DX
        </button>
      </div>

      {/* Row 3 -- TX volume, matching Android's TxVolumeRow (-/slider/+).
          Shares the same handler as Settings -> Audio's TX output level
          slider -- Android only ever shows this one control (in the
          drawer); desktop also keeps it in Settings for discoverability. */}
      <div className="txbar-row">
        <div className="tx-volume-control">
          <button
            className="tx-volume-step"
            aria-label="Decrease TX volume"
            onClick={() => onTxGain(Math.max(0, txGain - 5))}
          >
            −
          </button>
          <input
            type="range"
            aria-label="TX output level (drive), percent"
            min={0}
            max={100}
            step={1}
            value={txGain}
            onChange={(e) => onTxGain(parseInt(e.target.value, 10))}
            className="tx-volume-slider"
          />
          <button
            className="tx-volume-step"
            aria-label="Increase TX volume"
            onClick={() => onTxGain(Math.min(100, txGain + 5))}
          >
            +
          </button>
          <span className="tx-volume-value">{txGain}%</span>
        </div>
      </div>
    </div>
  );
}
