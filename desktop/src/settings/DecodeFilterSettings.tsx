import { useState } from "react";
import { api } from "../ipc";
import type { HighlightConfig } from "../qsoStatus";
import type { WorkedMode, WorkedScope } from "../decodeFilters";
import { SettingsRow, TextListDialog, ListPickerDialog } from "./SettingsPrimitives";

const WORKED_MODE_OPTIONS: { value: WorkedMode; label: string }[] = [
  { value: "highlight", label: "Highlight" },
  { value: "ignore", label: "Ignore" },
  { value: "hide", label: "Hide" },
];
const WORKED_SCOPE_OPTIONS: { value: WorkedScope; label: string }[] = [
  { value: "on_band", label: "Worked on this band" },
  { value: "before", label: "Worked before (any band)" },
  { value: "today", label: "Worked today or yesterday" },
  { value: "from_list", label: "In worked list" },
];
const WORKED_MODE_LABEL: Record<WorkedMode, string> = { highlight: "Highlight", ignore: "Ignore", hide: "Hide" };
const WORKED_SCOPE_LABEL: Record<WorkedScope, string> = {
  on_band: "Worked on this band", before: "Worked before (any band)",
  today: "Worked today or yesterday", from_list: "In worked list",
};

type OpenDialog =
  | null | "workedMode" | "workedScope" | "workedList"
  | "blockExact" | "blockPrefix" | "blockKeyword" | "watchlist";

/**
 * Decode & Filters, ported from the Android fork's own
 * `DecodeFilterSettings.kt` -- decode-list highlight toggles + worked-
 * station handling (real, including "Hide" actually removing rows --
 * `App.tsx`'s own `persistentlyVisible` filter), the callsign blocklist
 * (real, filters the list), two of the six Decode Filters toggles that are
 * real now (Show Only CQ / Needed Only), and Needed-DX Alerts (real --
 * watchlist + new-DXCC/new-state/CQ-reply fire through the status line,
 * since desktop has no sound/notification system yet; see `App.tsx`'s
 * `fireDecodeAlerts` for exactly why the real Android description is
 * "sound + vibrate + notify" and this isn't).
 *
 * Four Decode Filters toggles stay disabled stubs with Android's real copy:
 * **DX Only**, **Filter By Continent** (+ its continent picker), and
 * **Skip/Hide Directional CQs** all need "what continent is the operator
 * in" and/or parsed CQ-modifier ("CQ DX"/"CQ EU"/...) detection -- neither
 * exists anywhere in desktop's decode pipeline yet (confirmed: no
 * continent-of-operator derivation, and `qsoStatus.ts`'s own doc comment
 * already flags that desktop doesn't parse CQ modifiers at all). Real,
 * bounded future work, not faked here. "Distance in Miles" and "Show beam
 * heading" persist for real but have no consumer yet either -- they're Map/
 * decode-row display settings (great-circle distance/bearing per row), not
 * filters, and wiring that display is its own separate UI change.
 */
export function DecodeFilterSettings({
  highlightConfig,
  onHighlightConfig,
  workedMode,
  onWorkedMode,
  workedScope,
  onWorkedScope,
  workedSameMode,
  onWorkedSameMode,
  workedListCsv,
  onWorkedListCsv,
  blockedExact,
  onBlockedExact,
  blockedPrefixes,
  onBlockedPrefixes,
  blockedKeywords,
  onBlockedKeywords,
  filterShowOnlyCQ,
  onFilterShowOnlyCQ,
  filterNeededOnly,
  onFilterNeededOnly,
  watchCallsigns,
  onWatchCallsigns,
  alertNewDxcc,
  onAlertNewDxcc,
  alertNewState,
  onAlertNewState,
  alertOnCqReply,
  onAlertOnCqReply,
  alertOnQsoComplete,
  onAlertOnQsoComplete,
  distanceMiles,
  onDistanceMiles,
}: {
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
  distanceMiles: boolean;
  onDistanceMiles: (v: boolean) => void;
}) {
  const [openDialog, setOpenDialog] = useState<OpenDialog>(null);

  // highlightNewDxcc/Zone/State/Grid/Prefix/Band/Pota/Worked -- each set
  // explicitly (not a generic key->configKey helper) since Android's real
  // key names aren't uniform ("highlightNewDxcc" vs "highlightPota" vs
  // "highlightWorked") and it's clearer to just say so per field.
  function setHighlightDxcc(v: boolean) {
    onHighlightConfig({ ...highlightConfig, dxcc: v });
    api.setConfig("highlightNewDxcc", v ? "1" : "0");
  }
  function setHighlightZone(v: boolean) {
    onHighlightConfig({ ...highlightConfig, zone: v });
    api.setConfig("highlightNewZone", v ? "1" : "0");
  }
  function setHighlightState(v: boolean) {
    onHighlightConfig({ ...highlightConfig, state: v });
    api.setConfig("highlightNewState", v ? "1" : "0");
  }
  function setHighlightGrid(v: boolean) {
    onHighlightConfig({ ...highlightConfig, grid: v });
    api.setConfig("highlightNewGrid", v ? "1" : "0");
  }
  function setHighlightPrefix(v: boolean) {
    onHighlightConfig({ ...highlightConfig, prefix: v });
    api.setConfig("highlightNewPrefix", v ? "1" : "0");
  }
  function setHighlightBand(v: boolean) {
    onHighlightConfig({ ...highlightConfig, band: v });
    api.setConfig("highlightNewBand", v ? "1" : "0");
  }
  function setHighlightPota(v: boolean) {
    onHighlightConfig({ ...highlightConfig, pota: v });
    api.setConfig("highlightPota", v ? "1" : "0");
  }
  function setHighlightWorked(v: boolean) {
    onHighlightConfig({ ...highlightConfig, worked: v });
    api.setConfig("highlightWorked", v ? "1" : "0");
  }

  function setWorkedMode(m: WorkedMode) {
    onWorkedMode(m);
    api.setConfig("workedStationMode", m);
  }
  function setWorkedScope(s: WorkedScope) {
    onWorkedScope(s);
    api.setConfig("workedStationScope", s);
  }
  function setWorkedSameMode(v: boolean) {
    onWorkedSameMode(v);
    api.setConfig("workedSameMode", v ? "1" : "0");
  }
  function saveWorkedList(v: string) {
    onWorkedListCsv(v);
    api.setConfig("workedStationList", v);
    setOpenDialog(null);
  }
  function saveBlockedExact(v: string) {
    onBlockedExact(v);
    api.setConfig("blockedExactCallsigns", v);
    setOpenDialog(null);
  }
  function saveBlockedPrefixes(v: string) {
    onBlockedPrefixes(v);
    api.setConfig("excludedCallsigns", v);
    setOpenDialog(null);
  }
  function saveBlockedKeywords(v: string) {
    onBlockedKeywords(v);
    api.setConfig("blockedKeywords", v);
    setOpenDialog(null);
  }
  function setFilterShowOnlyCQ(v: boolean) {
    onFilterShowOnlyCQ(v);
    api.setConfig("filterShowOnlyCQ", v ? "1" : "0");
  }
  function setFilterNeededOnly(v: boolean) {
    onFilterNeededOnly(v);
    api.setConfig("filterNeededOnly", v ? "1" : "0");
  }
  function saveWatchlist(v: string) {
    onWatchCallsigns(v);
    api.setConfig("watchCallsigns", v);
    setOpenDialog(null);
  }
  function setAlertNewDxcc(v: boolean) {
    onAlertNewDxcc(v);
    api.setConfig("alertNewDxcc", v ? "1" : "0");
  }
  function setAlertNewState(v: boolean) {
    onAlertNewState(v);
    api.setConfig("alertNewState", v ? "1" : "0");
  }
  function setAlertOnCqReply(v: boolean) {
    onAlertOnCqReply(v);
    api.setConfig("alertOnCqReply", v ? "1" : "0");
  }
  function setAlertOnQsoComplete(v: boolean) {
    onAlertOnQsoComplete(v);
    api.setConfig("alertOnQsoComplete", v ? "1" : "0");
  }

  return (
    <>
      <div className="panel">
        <h3>Decode Highlights</h3>
        <div className="settings-category-list">
          <SettingsRow label="New DXCC" description="Highlight stations from an unworked DXCC entity" toggle={highlightConfig.dxcc} onToggleChange={setHighlightDxcc} />
          <SettingsRow label="New Zone" description="Highlight not-yet-worked CQ zones (Worked All Zones)" toggle={highlightConfig.zone} onToggleChange={setHighlightZone} />
          <SettingsRow label="New State" description="Highlight not-yet-worked US states (Worked All States)" toggle={highlightConfig.state} onToggleChange={setHighlightState} />
          <SettingsRow label="New Grid" description="Highlight not-yet-worked Maidenhead grids" toggle={highlightConfig.grid} onToggleChange={setHighlightGrid} />
          <SettingsRow label="New Prefix" description="Highlight not-yet-worked callsign prefixes (Worked All Prefixes / WPX)" toggle={highlightConfig.prefix} onToggleChange={setHighlightPrefix} />
          <SettingsRow label="New Band" description="Highlight stations worked only on other bands" toggle={highlightConfig.band} onToggleChange={setHighlightBand} />
          <SettingsRow label="POTA Activators" description="Highlight spotted POTA activators; new parks stand out" toggle={highlightConfig.pota} onToggleChange={setHighlightPota} />
          <SettingsRow label="Worked Stations" description="Handle stations already in your log (highlight, ignore, or hide)" toggle={highlightConfig.worked} onToggleChange={setHighlightWorked} />
          {highlightConfig.worked && (
            <>
              <SettingsRow label="Behavior" description="What to do with worked stations" value={WORKED_MODE_LABEL[workedMode]} onClick={() => setOpenDialog("workedMode")} />
              <SettingsRow label="Match by" description="Which stations count as worked" value={WORKED_SCOPE_LABEL[workedScope]} onClick={() => setOpenDialog("workedScope")} />
              {workedScope !== "from_list" && (
                <SettingsRow label="Same mode only" description="Only count QSOs made on the current mode" toggle={workedSameMode} onToggleChange={setWorkedSameMode} disabled />
              )}
              {workedScope === "from_list" && (
                <SettingsRow label="Worked List" description="Callsigns to treat as worked (comma-separated)" value={workedListCsv || "None"} onClick={() => setOpenDialog("workedList")} />
              )}
            </>
          )}
          <SettingsRow label="Distance in Miles" description="Show distances in miles instead of kilometers" toggle={distanceMiles} onToggleChange={onDistanceMiles} />
          <SettingsRow label="Show beam heading" description="Add the great-circle bearing to each decode row, for pointing a directional antenna" toggle={false} onToggleChange={() => {}} disabled />
        </div>
      </div>

      <div className="panel">
        <h3>Callsign Blocklist</h3>
        <div className="settings-category-list">
          <SettingsRow label="Exact Callsigns" description="Block whole-call matches" value={blockedExact || "None"} onClick={() => setOpenDialog("blockExact")} />
          <SettingsRow label="Prefixes" description="Block by callsign prefix (e.g. RA)" value={blockedPrefixes || "None"} onClick={() => setOpenDialog("blockPrefix")} />
          <SettingsRow label="Keywords" description="Block by substring in call or message (e.g. POTA, /P)" value={blockedKeywords || "None"} onClick={() => setOpenDialog("blockKeyword")} />
        </div>
      </div>

      <div className="panel">
        <h3>Decode Filters</h3>
        <div className="settings-category-list">
          <SettingsRow label="Show Only CQ" description="Hide everything except CQ-type messages" toggle={filterShowOnlyCQ} onToggleChange={setFilterShowOnlyCQ} />
          <SettingsRow label="DX Only" description="Show only stations outside your own continent -- needs continent-of-operator detection, not built yet" toggle={false} onToggleChange={() => {}} disabled />
          <SettingsRow label="Needed Only" description="Show only stations not yet worked" toggle={filterNeededOnly} onToggleChange={setFilterNeededOnly} />
          <SettingsRow label="Filter By Continent" description="Show only stations from a chosen continent -- needs continent-of-operator detection, not built yet" toggle={false} onToggleChange={() => {}} disabled />
          <SettingsRow label="Skip Directional CQs (auto-reply)" description="Don't auto-answer CQ DX/EU/JA… aimed at a different DXCC -- needs CQ-modifier parsing, not built yet" toggle={false} onToggleChange={() => {}} disabled />
          <SettingsRow label="Hide Directional CQs Not For Me" description="Hide region-directed CQs that don't target your DXCC -- needs CQ-modifier parsing, not built yet" toggle={false} onToggleChange={() => {}} disabled />
        </div>
      </div>

      <div className="panel">
        <h3>Needed-DX Alerts</h3>
        <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
          Desktop has no sound/notification system yet -- these post to the status line instead.
        </div>
        <div className="settings-category-list">
          <SettingsRow label="Callsign watchlist" description="Status-line alert the moment a watched callsign or prefix is decoded, on any band" value={watchCallsigns || "None"} onClick={() => setOpenDialog("watchlist")} />
          <SettingsRow label="New DXCC" description="Status-line alert when an unworked DXCC entity calls CQ" toggle={alertNewDxcc} onToggleChange={setAlertNewDxcc} />
          <SettingsRow label="New US State" description="Status-line alert when a station from an unworked US state calls CQ (state from grid)" toggle={alertNewState} onToggleChange={setAlertNewState} />
          <SettingsRow label="CQ reply" description="Status-line alert when a station calls you (any message addressed to your callsign)" toggle={alertOnCqReply} onToggleChange={setAlertOnCqReply} />
          <SettingsRow label="QSO complete" description="Status-line alert when a QSO is logged (desktop already always shows this; the toggle is stored but doesn't change that yet)" toggle={alertOnQsoComplete} onToggleChange={setAlertOnQsoComplete} />
        </div>
      </div>

      {openDialog === "workedMode" && (
        <ListPickerDialog title="Behavior" options={WORKED_MODE_OPTIONS} value={workedMode} onSelect={(v) => { setWorkedMode(v); setOpenDialog(null); }} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "workedScope" && (
        <ListPickerDialog title="Match by" options={WORKED_SCOPE_OPTIONS} value={workedScope} onSelect={(v) => { setWorkedScope(v); setOpenDialog(null); }} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "workedList" && (
        <TextListDialog title="Worked List" description="Callsigns to treat as worked (comma-separated)" initialValue={workedListCsv} onSave={saveWorkedList} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "blockExact" && (
        <TextListDialog title="Block Exact Callsigns" description="Whole-call match. Comma- or space-separated, e.g. W1AW, K1ABC." initialValue={blockedExact} onSave={saveBlockedExact} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "blockPrefix" && (
        <TextListDialog title="Block Callsign Prefixes" description="Prefix match, e.g. RA blocks RA0AA..RA9ZZ. Comma- or space-separated." initialValue={blockedPrefixes} onSave={saveBlockedPrefixes} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "blockKeyword" && (
        <TextListDialog title="Block Keywords" description="Substring match against the call and message text, e.g. POTA, /P, QRP." initialValue={blockedKeywords} onSave={saveBlockedKeywords} onCancel={() => setOpenDialog(null)} />
      )}
      {openDialog === "watchlist" && (
        <TextListDialog title="Callsign watchlist" description="Comma-separated calls or prefixes (e.g. 3Y0, W1AW, TX7). Matched by prefix, so a DXpedition prefix catches every variant." initialValue={watchCallsigns} onSave={saveWatchlist} onCancel={() => setOpenDialog(null)} />
      )}
    </>
  );
}
