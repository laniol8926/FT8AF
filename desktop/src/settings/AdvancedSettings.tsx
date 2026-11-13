import { useState } from "react";
import { api } from "../ipc";
import type { ParsedSettingsBackup } from "../ipc";
import { defaultBackupFileName } from "../settingsBackup";
import { SettingsRow } from "./SettingsPrimitives";

/**
 * Advanced, ported from the Android fork's own `AdvancedSettings.kt` +
 * `SettingsBackup.kt`. Backup/Export/Import and Decode Depth are real;
 * everything else is a disclosed stub (see each section's own comment for
 * exactly why) rather than silently dropped, per the standing project
 * philosophy.
 *
 * - **Backup & Restore**: real, end-to-end. Export reuses the exact
 *   Blob+`<a download>` trick already proven for ADIF export (lands in the
 *   OS Downloads folder, no native dialog); import uses a native file-pick
 *   dialog (same reasoning as `pickAdifFile`) then previews before
 *   applying. Desktop has no SharedPreferences-style "prefs" files the way
 *   Android's fork does (custom bands/map style already live in the one
 *   config table), so there's no "prefs" section to write -- and if a real
 *   Android export is imported (which does have one), it's simply ignored
 *   rather than erroring (see `settings_backup.rs`'s own doc comment).
 *   Android re-hydrates its in-memory settings live after an import so
 *   most things apply with no restart; desktop's config is read at many
 *   different, scattered times across many components with no central
 *   "re-hydrate everything" hook, so this honestly tells the operator to
 *   restart instead of pretending a live re-apply that isn't actually
 *   wired everywhere.
 * - **Decode Depth**: real -- a Fast/Deep 2-tier toggle (desktop's
 *   `Decoder::set_deep`, found already ported but never wired to anything
 *   while researching this screen). Own config key (`decodeDeep`, plain
 *   boolean) rather than Android's `decodeDepth` (0/1/2 Fast/Balanced/
 *   Deep): desktop has no separate BALANCED tier (no subtract-and-redecode
 *   pass exists), so reusing Android's 3-value encoding would be a false
 *   equivalence.
 * - **PTT Delay / TX Delay / Late-start Tolerance**: disclosed stubs.
 *   These feed directly into Android's TX-timing safety math
 *   (`FT8TransmitSignal.keyUpHoldLimitMs`/`lateStartSkipMs`) -- this
 *   project's own `AGENTS.md` documents two real historical TX-timing bugs
 *   from exactly this class of calculation, and a real stuck-TX incident
 *   is on record for a sibling project. Wiring these needs its own
 *   careful, dedicated pass (confirm desktop's TX pipeline has an
 *   equivalent late-start-skip mechanism at all, replicate the safety math
 *   exactly, unit-test it, then live-test deliberately) -- not something
 *   to rush alongside a dozen other settings in one sitting.
 * - **FFT Averaging / Spectrum Bin Combining**: disclosed stubs -- distinct
 *   from the FFT Window/FFT size/Welch-averaging knobs already real under
 *   Radio & Audio's "Developer — waterfall FFT" panel (which cover a
 *   different axis: how the FFT itself is computed, not how frames/bins
 *   are smoothed afterward). Neither EMA frame-averaging nor max/avg/rms
 *   bin combining exists anywhere in `wf.rs` yet -- real new DSP work.
 * - **Theme**: disclosed stub -- desktop has exactly one color theme, no
 *   theme-switching infrastructure at all.
 * - **Language**: disclosed stub -- Android supports 17 languages; desktop
 *   has zero localization infrastructure (every string is hardcoded
 *   English throughout the whole app). A real port is a whole-app
 *   i18n effort, not a Settings row.
 * - **Keep Screen On / Diagnostics (crash reporting)**: not shown at all,
 *   not even as a stub. Keep Screen On has no desktop equivalent in
 *   principle (a desktop monitor sleeping doesn't pause the app the way an
 *   Android device sleeping can) -- same "genuinely inapplicable" bucket
 *   as the "Restore Media Volume" toggle omitted from Decode & Filters.
 *   Crash reporting (Sentry) was already explicitly decided against for
 *   desktop earlier in this port (a real infra decision, not an oversight).
 */
export function AdvancedSettings({
  decodeDeep,
  onDecodeDeep,
}: {
  decodeDeep: boolean;
  onDecodeDeep: (v: boolean) => void;
}) {
  const [showExportDialog, setShowExportDialog] = useState(false);
  const [includeSensitive, setIncludeSensitive] = useState(false);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [pendingImport, setPendingImport] = useState<ParsedSettingsBackup | null>(null);
  const [importDone, setImportDone] = useState(false);

  function setDecodeDeep(v: boolean) {
    onDecodeDeep(v);
    api.setDecodeDepth(v);
  }

  async function doExport() {
    setShowExportDialog(false);
    const [json, info] = await Promise.all([api.exportSettingsBackup(includeSensitive), api.getAppInfo()]);
    const date = new Date().toISOString().slice(0, 10);
    const filename = defaultBackupFileName(date, info.version);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
    setExportStatus(`Saved ${filename} to your Downloads folder.`);
  }

  async function pickImportFile() {
    setImportError(null);
    setImportDone(false);
    const path = await api.pickSettingsBackupFile();
    if (!path) return;
    try {
      const parsed = await api.previewSettingsBackup(path);
      setPendingImport(parsed);
    } catch (e) {
      setImportError(typeof e === "string" ? e : String(e));
    }
  }

  async function confirmImport() {
    if (!pendingImport) return;
    await api.applySettingsBackup(pendingImport.config, pendingImport.follow_callsigns);
    setPendingImport(null);
    setImportDone(true);
  }

  return (
    <>
      <div className="panel">
        <h3>Advanced</h3>
        <div className="settings-category-list">
          <SettingsRow label="PTT Delay" description="Delay after PTT before transmit audio begins -- needs a dedicated TX-timing-safety review before this is wired, see this screen's own doc comment" disabled />
          <SettingsRow label="TX Delay" description="Delay before transmit to allow prior-cycle decode -- same TX-timing-safety deferral as PTT Delay" disabled />
          <SettingsRow label="Late-start Tolerance" description="Extra leading audio a late manual TX may clip so it still goes out this cycle -- same TX-timing-safety deferral" disabled />
        </div>
      </div>

      <div className="panel">
        <h3>FFT / Waterfall (Developer)</h3>
        <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
          FFT window, size, and Welch averaging are already configurable under Radio & Audio's
          "Developer — waterfall FFT" panel. The two below are a different axis (post-FFT smoothing)
          and aren't built yet.
        </div>
        <div className="settings-category-list">
          <SettingsRow label="Frame averaging" description="Smooth the display across FFT frames; Off shows each frame as-is -- not built yet" disabled />
          <SettingsRow label="Spectrum bin combining" description="How adjacent FFT bins merge into one spectrum bar -- not built yet" disabled />
        </div>
      </div>

      <div className="panel">
        <h3>Power & Heat</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Decode Depth"
            description="Deep runs more LDPC iterations and a lower sync threshold to dig out weak signals, at higher CPU cost. Fast runs coolest."
            toggle={decodeDeep}
            onToggleChange={setDecodeDeep}
          />
        </div>
      </div>

      <div className="panel">
        <h3>Appearance</h3>
        <div className="settings-category-list">
          <SettingsRow label="Theme" description="App color theme -- desktop has one theme, no theme-switching built yet" disabled />
        </div>
      </div>

      <div className="panel">
        <h3>Language</h3>
        <div className="settings-category-list">
          <SettingsRow label="Language" description="App display language -- desktop has no localization infrastructure yet; this needs a whole-app i18n effort, not a Settings row" disabled />
        </div>
      </div>

      <div className="panel">
        <h3>Backup & Restore</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Export settings"
            description="Save all settings to a backup file"
            onClick={() => setShowExportDialog(true)}
          />
          <SettingsRow
            label="Import settings"
            description="Restore settings from a backup file"
            onClick={pickImportFile}
          />
        </div>
        {exportStatus && <div className="muted" style={{ marginTop: 8, fontSize: 12 }}>{exportStatus}</div>}
        {importError && (
          <div style={{ marginTop: 8, fontSize: 12, color: "var(--tx)" }}>⚠ {importError}</div>
        )}
        {importDone && (
          <div className="muted" style={{ marginTop: 8, fontSize: 12 }}>
            Settings restored. Restart FT8AF for every setting to take effect.
          </div>
        )}
      </div>

      {showExportDialog && (
        <div className="modal-overlay" onClick={() => setShowExportDialog(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Export settings</h3>
            <div className="field">
              <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <input
                  type="checkbox"
                  checked={includeSensitive}
                  onChange={(e) => setIncludeSensitive(e.target.checked)}
                  style={{ width: "auto" }}
                />
                Include sensitive data
              </label>
              <span className="muted" style={{ display: "block", marginTop: 4 }}>
                API keys and passwords (Cloudlog, QRZ, rig login). Off by default so a shared
                backup never leaks credentials.
              </span>
            </div>
            <div className="actions">
              <button className="modal-text-btn" onClick={() => setShowExportDialog(false)}>Cancel</button>
              <button className="modal-text-btn modal-text-btn-accent" onClick={doExport}>Export</button>
            </div>
          </div>
        </div>
      )}

      {pendingImport && (
        <div className="modal-overlay" onClick={() => setPendingImport(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Import settings?</h3>
            <p className="muted">
              This will overwrite {Object.keys(pendingImport.config).length} setting(s) with values
              from the backup created {pendingImport.created_at || "?"}.
              {pendingImport.follow_callsigns.length > 0 &&
                ` It also adds ${pendingImport.follow_callsigns.length} callsign(s) to your Hunt watchlist.`}
            </p>
            <div className="actions">
              <button className="modal-text-btn" onClick={() => setPendingImport(null)}>Cancel</button>
              <button className="modal-text-btn modal-text-btn-accent" onClick={confirmImport}>Import</button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
