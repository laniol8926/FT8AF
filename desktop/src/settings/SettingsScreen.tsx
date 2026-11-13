import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { BandInfo, ClockSyncEvent, OperatingMode, RigStatusEvent } from "../ipc";
import type { HuntPriority } from "../hunt";
import { HUNT_PRIORITIES } from "../hunt";
import { SettingsDetailScaffold, SettingsRow } from "./SettingsPrimitives";
import { OperatorCard } from "./OperatorCard";
import { EditOperatorDialog } from "./EditOperatorDialog";
import { RadioAudioSettings } from "./RadioAudioSettings";
import { AboutSettings } from "./AboutSettings";
import { TimeSyncSettings } from "./TimeSyncSettings";
import { DecodeFilterSettings } from "./DecodeFilterSettings";
import type { HighlightConfig } from "../qsoStatus";
import type { WorkedMode, WorkedScope } from "../decodeFilters";

type Category = "radio" | "about" | "timesync" | "decodefilters";

/**
 * Settings landing page, matching the real Android fork's own
 * `SettingsScreen.kt`/`SettingsLanding` layout exactly (confirmed against a
 * live screenshot of the real app): a pinned `OperatorCard` (callsign/grid/
 * rig/antenna/power, tap to edit via `EditOperatorDialog` -- there is no
 * separate "Station" category, the card *is* Station editing), a standalone
 * location-service toggle row right below it, then a single flat list card
 * of categories (no "Setup"/"Operation"/"More" sub-headers -- Android has
 * just one list). Only Radio & Audio is real in Phase 1; the rest are
 * stubbed as disabled rows, enabled as each later phase lands (see
 * `project_ft8af_desktop_parity_roadmap`).
 */
export function SettingsScreen(props: {
  onStatus: (s: string) => void;
  clock: ClockSyncEvent | null;
  onSettingsSaved: (known?: { call?: string; rigBackend?: string }) => void;
  onRigLabel: (v: string) => void;
  onBandsChanged: (bands: BandInfo[]) => void;
  operatingMode: OperatingMode;
  rig: RigStatusEvent | null;
  bands: BandInfo[];
  dialHz: number;
  onBand: (hz: number) => void;
  onMode: (m: OperatingMode) => void;
  showVolumeSlider: boolean;
  onShowVolumeSlider: (v: boolean) => void;
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
  displayHz: number;
  onDisplayHz: (hz: number) => void;
  onUpdateGridTable: () => void;
  gridSeedBusy: boolean;
  onAudioBandConfigChanged: () => void;
  highlightConfig: HighlightConfig;
  onHighlightConfig: (c: HighlightConfig) => void;
  workedMode: WorkedMode;
  onWorkedMode: (m: WorkedMode) => void;
  workedScope: WorkedScope;
  onWorkedScope: (s: WorkedScope) => void;
  workedSameMode: boolean;
  onWorkedSameMode: (v: boolean) => void;
  workedListCsv: string;
  onWorkedListCsv: (v: string) => void;
  blockedExact: string;
  onBlockedExact: (v: string) => void;
  blockedPrefixes: string;
  onBlockedPrefixes: (v: string) => void;
  blockedKeywords: string;
  onBlockedKeywords: (v: string) => void;
  filterShowOnlyCQ: boolean;
  onFilterShowOnlyCQ: (v: boolean) => void;
  filterNeededOnly: boolean;
  onFilterNeededOnly: (v: boolean) => void;
  watchCallsigns: string;
  onWatchCallsigns: (v: string) => void;
  alertNewDxcc: boolean;
  onAlertNewDxcc: (v: boolean) => void;
  alertNewState: boolean;
  onAlertNewState: (v: boolean) => void;
  alertOnCqReply: boolean;
  onAlertOnCqReply: (v: boolean) => void;
  alertOnQsoComplete: boolean;
  onAlertOnQsoComplete: (v: boolean) => void;
}) {
  const {
    onStatus, clock, onSettingsSaved, onRigLabel, onBandsChanged, operatingMode, rig,
    bands, dialHz, onBand, onMode, showVolumeSlider, onShowVolumeSlider,
    huntPriority, onHuntPriority, huntAvoidPileups, onHuntAvoidPileups, huntMinSnr, onHuntMinSnr,
    inputGain, onInputGain, txGain, onTxGain, displayHz, onDisplayHz,
    onUpdateGridTable, gridSeedBusy, onAudioBandConfigChanged,
    highlightConfig, onHighlightConfig, workedMode, onWorkedMode, workedScope, onWorkedScope,
    workedSameMode, onWorkedSameMode, workedListCsv, onWorkedListCsv,
    blockedExact, onBlockedExact, blockedPrefixes, onBlockedPrefixes, blockedKeywords, onBlockedKeywords,
    filterShowOnlyCQ, onFilterShowOnlyCQ, filterNeededOnly, onFilterNeededOnly,
    watchCallsigns, onWatchCallsigns, alertNewDxcc, onAlertNewDxcc, alertNewState, onAlertNewState,
    alertOnCqReply, onAlertOnCqReply, alertOnQsoComplete, onAlertOnQsoComplete,
  } = props;

  const [category, setCategory] = useState<Category | null>(null);
  const [call, setCall] = useState("");
  const [grid, setGrid] = useState("");
  const [antenna, setAntenna] = useState("");
  const [power, setPower] = useState("");
  const [locEnabled, setLocEnabled] = useState(false);
  const [locating, setLocating] = useState(false);
  const [locError, setLocError] = useState<string | null>(null);
  const [showEditOperator, setShowEditOperator] = useState(false);

  useEffect(() => {
    api.getConfig("my_call").then((v) => v && setCall(v));
    api.getConfig("my_grid").then((v) => v && setGrid(v));
    api.getConfig("antenna").then((v) => v && setAntenna(v));
    api.getConfig("powerWatts").then((v) => v && setPower(v));
    api.getConfig("location_service_enabled").then((v) => setLocEnabled(v === "true"));
  }, []);

  function saveOperator(savedCall: string, savedGrid: string, savedAntenna: string, savedPower: string) {
    setCall(savedCall);
    setGrid(savedGrid);
    setAntenna(savedAntenna);
    setPower(savedPower);
    api.setStation(savedCall, savedGrid);
    api.setConfig("antenna", savedAntenna);
    api.setConfig("powerWatts", savedPower);
    onStatus(`Station set: ${savedCall} ${savedGrid}`);
    onSettingsSaved({ call: savedCall });
    setShowEditOperator(false);
  }

  function toggleLocEnabled(on: boolean) {
    setLocEnabled(on);
    setLocError(null);
    api.setConfig("location_service_enabled", on ? "true" : "false");
  }

  async function useMyLocation() {
    setLocating(true);
    setLocError(null);
    try {
      await api.setConfig("location_service_enabled", "true");
      const loc = await api.getOsLocation();
      setGrid(loc.grid);
      api.setStation(call, loc.grid);
      onSettingsSaved({ call });
    } catch (e) {
      setLocError(typeof e === "string" ? e : String(e));
    } finally {
      setLocating(false);
    }
  }

  if (category === "radio") {
    return (
      <SettingsDetailScaffold title="Radio & Audio" onBack={() => setCategory(null)}>
        <RadioAudioSettings
          onStatus={onStatus}
          onRigLabel={onRigLabel}
          onBandsChanged={onBandsChanged}
          onRigConnected={(rigBackend) => onSettingsSaved({ rigBackend })}
          operatingMode={operatingMode}
          rig={rig}
          bands={bands}
          dialHz={dialHz}
          onBand={onBand}
          onMode={onMode}
          showVolumeSlider={showVolumeSlider}
          onShowVolumeSlider={onShowVolumeSlider}
          inputGain={inputGain}
          onInputGain={onInputGain}
          txGain={txGain}
          onTxGain={onTxGain}
          displayHz={displayHz}
          onDisplayHz={onDisplayHz}
          onUpdateGridTable={onUpdateGridTable}
          gridSeedBusy={gridSeedBusy}
          onAudioBandConfigChanged={onAudioBandConfigChanged}
        />
      </SettingsDetailScaffold>
    );
  }

  if (category === "about") {
    return (
      <SettingsDetailScaffold title="About" onBack={() => setCategory(null)}>
        <AboutSettings call={call} />
      </SettingsDetailScaffold>
    );
  }

  if (category === "timesync") {
    return (
      <SettingsDetailScaffold title="Time Sync" onBack={() => setCategory(null)}>
        <TimeSyncSettings clock={clock} onStatus={onStatus} />
      </SettingsDetailScaffold>
    );
  }

  if (category === "decodefilters") {
    return (
      <SettingsDetailScaffold title="Decode & Filters" onBack={() => setCategory(null)}>
        <DecodeFilterSettings
          highlightConfig={highlightConfig}
          onHighlightConfig={onHighlightConfig}
          workedMode={workedMode}
          onWorkedMode={onWorkedMode}
          workedScope={workedScope}
          onWorkedScope={onWorkedScope}
          workedSameMode={workedSameMode}
          onWorkedSameMode={onWorkedSameMode}
          workedListCsv={workedListCsv}
          onWorkedListCsv={onWorkedListCsv}
          blockedExact={blockedExact}
          onBlockedExact={onBlockedExact}
          blockedPrefixes={blockedPrefixes}
          onBlockedPrefixes={onBlockedPrefixes}
          blockedKeywords={blockedKeywords}
          onBlockedKeywords={onBlockedKeywords}
          filterShowOnlyCQ={filterShowOnlyCQ}
          onFilterShowOnlyCQ={onFilterShowOnlyCQ}
          filterNeededOnly={filterNeededOnly}
          onFilterNeededOnly={onFilterNeededOnly}
          watchCallsigns={watchCallsigns}
          onWatchCallsigns={onWatchCallsigns}
          alertNewDxcc={alertNewDxcc}
          onAlertNewDxcc={onAlertNewDxcc}
          alertNewState={alertNewState}
          onAlertNewState={onAlertNewState}
          alertOnCqReply={alertOnCqReply}
          onAlertOnCqReply={onAlertOnCqReply}
          alertOnQsoComplete={alertOnQsoComplete}
          onAlertOnQsoComplete={onAlertOnQsoComplete}
        />
      </SettingsDetailScaffold>
    );
  }

  const rigName = rig?.connected ? rig.model : "Not connected";
  const antennaDisplay = antenna || "--";
  const powerDisplay = power ? `${power}W` : "--";

  return (
    <div className="settings-landing">
      {showEditOperator && (
        <EditOperatorDialog
          initialCallsign={call}
          initialGrid={grid}
          initialAntenna={antenna}
          initialPower={power}
          onCancel={() => setShowEditOperator(false)}
          onSave={saveOperator}
        />
      )}

      <div className="settings-section-header" style={{ marginTop: 0 }}>Operator identity</div>
      <OperatorCard
        callsign={call}
        grid={grid}
        rigName={rigName}
        antenna={antennaDisplay}
        power={powerDisplay}
        onClick={() => setShowEditOperator(true)}
      />

      <div className="settings-category-list panel settings-category-card">
        <SettingsRow
          label="Use OS location service"
          description={
            locError
              ? `⚠ ${locError}`
              : "Fill the grid from your computer's location service (coarse, city-level accuracy)."
          }
          toggle={locEnabled}
          onToggleChange={toggleLocEnabled}
        />
      </div>
      {locEnabled && (
        <button type="button" onClick={useMyLocation} disabled={locating} style={{ alignSelf: "flex-start" }}>
          {locating ? "Locating…" : "📍 Use my location"}
        </button>
      )}

      <div className="settings-category-list panel settings-category-card">
        <SettingsRow label="Radio & Audio" description="Rig connection, audio devices, bands" onClick={() => setCategory("radio")} />
        <SettingsRow label="Transmission" disabled />
        <SettingsRow label="Time Sync" description="Manual clock correction for offline operating" onClick={() => setCategory("timesync")} />
        <SettingsRow label="Decode & Filters" onClick={() => setCategory("decodefilters")} />
        <SettingsRow label="Voice Assistant" description="Spoken announcements and hands-free voice commands" disabled />
        <SettingsRow label="Logging & Awards" disabled />
        <SettingsRow label="Advanced" description="Backup & restore settings, decode tuning" disabled />
        <SettingsRow label="About" onClick={() => setCategory("about")} />
      </div>

      {/* Hunt stays inline for now, same shape as before this phase --
          folded into the Transmission category once that phase lands. */}
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
                if (v === "") {
                  onHuntMinSnr(null);
                  return;
                }
                const n = parseInt(v, 10);
                if (!Number.isNaN(n)) onHuntMinSnr(n);
              }}
              placeholder="e.g. -15"
              style={{ width: 80 }}
            />
          </div>
        </div>
      </div>

    </div>
  );
}
