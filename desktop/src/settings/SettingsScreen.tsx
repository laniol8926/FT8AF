import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { BandInfo, ClockSyncEvent, OperatingMode, RigStatusEvent } from "../ipc";
import type { HuntPriority } from "../hunt";
import { SettingsDetailScaffold, SettingsRow } from "./SettingsPrimitives";
import { OperatorCard } from "./OperatorCard";
import { EditOperatorDialog } from "./EditOperatorDialog";
import { RadioAudioSettings } from "./RadioAudioSettings";
import { AboutSettings } from "./AboutSettings";
import { TimeSyncSettings } from "./TimeSyncSettings";
import { DecodeFilterSettings } from "./DecodeFilterSettings";
import { AdvancedSettings } from "./AdvancedSettings";
import { TransmissionSettings } from "./TransmissionSettings";
import { LoggingSettings } from "./LoggingSettings";
import type { HighlightConfig } from "../qsoStatus";
import type { WorkedMode, WorkedScope } from "../decodeFilters";

type Category = "radio" | "about" | "timesync" | "decodefilters" | "advanced" | "transmission" | "logging";

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
  onNtpDisciplineChange: (on: boolean) => void;
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
  huntEnabled: boolean;
  onHuntEnabled: (v: boolean) => void;
  huntPriority: HuntPriority;
  onHuntPriority: (p: HuntPriority) => void;
  huntAvoidPileups: boolean;
  onHuntAvoidPileups: (v: boolean) => void;
  huntMinSnr: number | null;
  onHuntMinSnr: (v: number | null) => void;
  holdTxFreq: boolean;
  onHoldTxFreq: (v: boolean) => void;
  pileupStrongestFirst: boolean;
  onPileupStrongestFirst: (v: boolean) => void;
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
  decodeDeep: boolean;
  onDecodeDeep: (v: boolean) => void;
  distanceMiles: boolean;
  onDistanceMiles: (v: boolean) => void;
}) {
  const {
    onStatus, clock, onNtpDisciplineChange, onSettingsSaved, onRigLabel, onBandsChanged, operatingMode, rig,
    bands, dialHz, onBand, onMode, showVolumeSlider, onShowVolumeSlider,
    huntEnabled, onHuntEnabled,
    huntPriority, onHuntPriority, huntAvoidPileups, onHuntAvoidPileups, huntMinSnr, onHuntMinSnr,
    holdTxFreq, onHoldTxFreq,
    pileupStrongestFirst, onPileupStrongestFirst,
    inputGain, onInputGain, txGain, onTxGain, displayHz, onDisplayHz,
    onUpdateGridTable, gridSeedBusy, onAudioBandConfigChanged,
    highlightConfig, onHighlightConfig, workedMode, onWorkedMode, workedScope, onWorkedScope,
    workedSameMode, onWorkedSameMode, workedListCsv, onWorkedListCsv,
    blockedExact, onBlockedExact, blockedPrefixes, onBlockedPrefixes, blockedKeywords, onBlockedKeywords,
    filterShowOnlyCQ, onFilterShowOnlyCQ, filterNeededOnly, onFilterNeededOnly,
    watchCallsigns, onWatchCallsigns, alertNewDxcc, onAlertNewDxcc, alertNewState, onAlertNewState,
    alertOnCqReply, onAlertOnCqReply, alertOnQsoComplete, onAlertOnQsoComplete,
    decodeDeep, onDecodeDeep,
    distanceMiles, onDistanceMiles,
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
        <TimeSyncSettings clock={clock} onStatus={onStatus} onNtpDisciplineChange={onNtpDisciplineChange} />
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
          distanceMiles={distanceMiles}
          onDistanceMiles={onDistanceMiles}
        />
      </SettingsDetailScaffold>
    );
  }

  if (category === "advanced") {
    return (
      <SettingsDetailScaffold title="Advanced" onBack={() => setCategory(null)}>
        <AdvancedSettings decodeDeep={decodeDeep} onDecodeDeep={onDecodeDeep} />
      </SettingsDetailScaffold>
    );
  }

  if (category === "logging") {
    return (
      <SettingsDetailScaffold title="Logging & Awards" onBack={() => setCategory(null)}>
        <LoggingSettings callsign={call} />
      </SettingsDetailScaffold>
    );
  }

  if (category === "transmission") {
    return (
      <SettingsDetailScaffold title="Transmission" onBack={() => setCategory(null)}>
        <TransmissionSettings
          huntEnabled={huntEnabled}
          onHuntEnabled={onHuntEnabled}
          huntPriority={huntPriority}
          onHuntPriority={onHuntPriority}
          huntAvoidPileups={huntAvoidPileups}
          onHuntAvoidPileups={onHuntAvoidPileups}
          huntMinSnr={huntMinSnr}
          onHuntMinSnr={onHuntMinSnr}
          holdTxFreq={holdTxFreq}
          onHoldTxFreq={onHoldTxFreq}
          pileupStrongestFirst={pileupStrongestFirst}
          onPileupStrongestFirst={onPileupStrongestFirst}
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
        <SettingsRow label="Transmission" description="TX protection, Tune, Hunt/auto-sequence" onClick={() => setCategory("transmission")} />
        <SettingsRow label="Time Sync" description="Manual clock correction for offline operating" onClick={() => setCategory("timesync")} />
        <SettingsRow label="Decode & Filters" onClick={() => setCategory("decodefilters")} />
        <SettingsRow label="Voice Assistant" description="Spoken announcements and hands-free voice commands" disabled />
        <SettingsRow label="Logging & Awards" description="LoTW, QRZ, Cloudlog" onClick={() => setCategory("logging")} />
        <SettingsRow label="Advanced" description="Backup & restore settings, decode tuning" onClick={() => setCategory("advanced")} />
        <SettingsRow label="About" onClick={() => setCategory("about")} />
      </div>

    </div>
  );
}
