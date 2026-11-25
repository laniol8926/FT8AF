import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { ParsedSettingsBackup } from "../ipc";
import { defaultBackupFileName } from "../settingsBackup";
import { SettingsRow, NumberInputDialog, ListPickerDialog } from "./SettingsPrimitives";

type DecodeTier = "fast" | "balanced" | "deep";
const DECODE_TIER_OPTIONS: { value: DecodeTier; label: string }[] = [
  { value: "fast", label: "Fast Decode" },
  { value: "balanced", label: "Balanced Decode" },
  { value: "deep", label: "Deep Decode" },
];

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
 *   wired everywhere. The "Include sensitive data" checkbox's own on/off
 *   choice is itself remembered now (`settingsBackupIncludeSensitive`) --
 *   a genuine desktop-only enhancement beyond what Android does (its
 *   equivalent checkbox is plain unpersisted Compose state, reset every
 *   time that screen reopens) -- requested directly after an export/
 *   import between two machines "lost" QRZ credentials that were never
 *   actually in the file because this box was left unchecked.
 * - **Decode Depth**: real, a Fast/Balanced/Deep 3-way picker matching
 *   Android's own labels, and now with Android's own real DISTINCTION
 *   between Balanced and Deep too (previously disclosed as identical --
 *   see git history). Two independent knobs: `decodeDeep`/`Decoder::
 *   set_deep` (more LDPC iterations + a lower sync threshold -- Balanced
 *   and Deep both set this) and `deepDecodeSic` (the real subtract-and-
 *   redecode loop -- successive interference cancellation: strip each
 *   decoded signal from the slot's audio and redecode the residual to
 *   dig out a weaker/overlapping one -- Deep only). The underlying C
 *   implementation (`ft8af_glue/ft8_subtract.c`) was already shipped on
 *   Android; this wires the same proven algorithm into desktop's decoder
 *   thread (see `engine.rs`'s `spawn_decoder_thread` doc comment). A
 *   separate `decodeDepthTier` key remembers which *label* the operator
 *   picked, since the two booleans alone don't carry a label. An install
 *   from before this 3-tier picker existed (tier key unset) migrates its
 *   old boolean 1:1 onto the label that preserves its prior meaning
 *   (off -> Fast, on -> Deep, not Balanced -- "Deep Decode" was the only
 *   on-state that setting's name ever had); the migration predates
 *   `deepDecodeSic` existing at all, so it defaults to off regardless
 *   until the operator explicitly picks Deep again.
 * - **PTT Delay / Late-start Tolerance**: real. Checked carefully against
 *   this project's own `AGENTS.md` (which documents two real historical
 *   TX-timing bugs from exactly this class of calculation) before
 *   touching anything: desktop's existing `Mode::tx_latest_ms`/
 *   `tx_slot_eligible` gate already uses the *correct* lateness formula
 *   (`max(0, into_cycle - audio_slack_ms)`, confirmed via its own
 *   `// AGENTS.md gotcha` comment -- not the naive modulo that caused the
 *   historical bug), and a real, already-tested leading-symbol/sample clip
 *   (`cat_tones::symbols_to_skip`/`audio::output::skip_cursor`) already
 *   existed but was structurally unreachable (the eligibility gate never
 *   let a late-enough start through to need it). PTT Delay generalizes the
 *   previously-fixed `PTT_DELAY_MS=100` into `Engine::ptt_delay_ms`
 *   (Android's own `pttDelay`, same 0-190ms range) -- default unchanged,
 *   so every existing install's real-world timing is identical until this
 *   is actually opened and changed. Late-start Tolerance widens the
 *   eligibility window by a configured amount, letting the already-correct
 *   clip math engage for whatever lateness that lets through -- but its
 *   *default* is `0`, deliberately **not** Android's `2000`: desktop's
 *   prior behavior (refuse to start at all once too late, never clip) is
 *   strictly safer than "clip some leading audio and go anyway", and every
 *   existing install has only ever had that safer behavior -- introducing
 *   this knob must not silently start clipping real transmissions for
 *   someone who never touched the setting (same reasoning as
 *   `autoCqAfterQso` keeping desktop's own prior default). See
 *   `tx_protection.rs`'s own doc comment for the full clamp/default
 *   rationale.
 * - **TX Delay**: disclosed stub, but not for a safety reason -- checked
 *   Android's real `GeneralVariables.transmitDelay`/`setTimer_sec` and
 *   found it exists to delay Android's own *fixed-interval polling timer*
 *   long enough that the previous slot's decode has likely finished before
 *   the TX module starts evaluating what to send. Desktop's run loop has
 *   no such fixed-delay polling timer to tune: its boundary TX trigger
 *   (`boundary_tx_ready`) is gated directly on the real decode-complete
 *   event (`!awaiting_any_decode`), which is both more precise (reacts the
 *   instant decoding actually finishes, not after a fixed guess) and
 *   already unconditionally correct. A TX Delay setting here would have
 *   nothing real to control -- wiring one anyway would either be an inert
 *   no-op or, worse, an artificial sleep that shrinks the real TX window
 *   for no benefit, undermining a mechanism that already works.
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
  const [decodeTier, setDecodeTierState] = useState<DecodeTier>(decodeDeep ? "deep" : "fast");
  const [pttDelay, setPttDelay] = useState(100);
  const [lateStartTolerance, setLateStartTolerance] = useState(0);
  const [openDialog, setOpenDialog] = useState<null | "decodeTier" | "pttDelay" | "lateStartTolerance">(null);

  useEffect(() => {
    // `decodeDepthTier` only remembers which of the 3 *labels* the operator
    // picked -- the real underlying decoder behavior is still just the one
    // boolean (`decodeDeep`/`onDecodeDeep`, already live-controlled from
    // App.tsx). An install from before this 3-tier UI existed has no tier
    // key yet; migrate its old boolean 1:1 onto the label that best
    // preserves its prior meaning (off -> Fast, on -> Deep, not Balanced --
    // "Deep Decode" was the only on-state that setting's own name ever had).
    api.getConfig("decodeDepthTier").then((v) => {
      if (v === "fast" || v === "balanced" || v === "deep") setDecodeTierState(v);
    });
    api.getConfig("pttDelay").then((v) => setPttDelay(v ? parseInt(v, 10) || 100 : 100));
    api.getConfig("lateStartTolerance").then((v) => setLateStartTolerance(v ? parseInt(v, 10) || 0 : 0));
    // Remembered across sessions -- a genuine desktop-only enhancement,
    // not a port: Android's own equivalent checkbox is plain unpersisted
    // Compose state (`remember { mutableStateOf(false) }`), reset every
    // time that screen reopens, same as this used to be here. Requested
    // directly: the checkbox's own on/off choice is itself a setting, so
    // it should survive like any other rather than silently resetting to
    // off every time (confirmed live: exporting without checking it
    // first, then importing on another machine, "lost" the QRZ
    // credentials that were never actually in the file -- expected given
    // the deliberate sensitive-key exclusion, but the operator's own
    // checkbox preference is still worth remembering for next time).
    api.getConfig("settingsBackupIncludeSensitive").then((v) => setIncludeSensitive(v === "1"));
  }, []);

  function setIncludeSensitiveAndPersist(v: boolean) {
    setIncludeSensitive(v);
    api.setConfig("settingsBackupIncludeSensitive", v ? "1" : "0");
  }

  function setDecodeTier(tier: DecodeTier) {
    setDecodeTierState(tier);
    api.setConfig("decodeDepthTier", tier);
    // Balanced and Deep both turn on the LDPC/OSD deep-decode boolean;
    // Deep alone also turns on the subtract-and-redecode loop (see this
    // row's own description).
    onDecodeDeep(tier !== "fast");
    api.setDecodeDepth(tier !== "fast");
    api.setDeepDecodeSic(tier === "deep");
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
          <SettingsRow
            label="PTT Delay"
            description="Delay after PTT before transmit audio begins -- how long your rig needs its T/R relay to settle"
            value={`${pttDelay}ms`}
            onClick={() => setOpenDialog("pttDelay")}
          />
          <SettingsRow
            label="TX Delay"
            description="Android delays its own TX timer this long to let the prior cycle's decode finish first -- desktop's TX trigger instead waits on the real decode-complete event directly (no fixed delay to tune), so there's nothing here for this setting to control"
            disabled
          />
          <SettingsRow
            label="Late-start Tolerance"
            description="Extra leading audio a late TX may clip so it still goes out this cycle instead of waiting for the next slot"
            value={`${lateStartTolerance}ms`}
            onClick={() => setOpenDialog("lateStartTolerance")}
          />
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
            description="Balanced and Deep both run more LDPC iterations and a lower sync threshold to dig out weak signals, at higher CPU cost than Fast. Deep's own subtract-and-redecode second pass (digging a weaker signal out from under a stronger one on the same frequency) doesn't exist in this decoder yet, so Balanced and Deep currently behave identically -- a real future enhancement, not silently dropped."
            value={DECODE_TIER_OPTIONS.find((o) => o.value === decodeTier)?.label ?? "Fast Decode"}
            onClick={() => setOpenDialog("decodeTier")}
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
                  onChange={(e) => setIncludeSensitiveAndPersist(e.target.checked)}
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
      {openDialog === "decodeTier" && (
        <ListPickerDialog
          title="Decode Depth"
          options={DECODE_TIER_OPTIONS}
          value={decodeTier}
          onSelect={(v) => {
            setDecodeTier(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "pttDelay" && (
        <NumberInputDialog
          title="PTT Delay"
          description="Delay (ms) after keying PTT before transmit audio begins, to let your rig's T/R relay settle. Too short can clip the leading edge of every transmission."
          value={pttDelay}
          min={0}
          max={190}
          onSave={(v) => {
            setPttDelay(v);
            api.setPttDelayMs(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "lateStartTolerance" && (
        <NumberInputDialog
          title="Late-start Tolerance"
          description="Extra leading audio (ms) a late TX may clip so it still goes out this cycle instead of waiting for the next slot. 0 (the default) never clips -- a too-late TX simply waits for the next slot, which is the safest choice; raising this trades a small amount of signal (and decodability) at the very start of the transmission for not having to wait up to a full cycle."
          value={lateStartTolerance}
          min={0}
          max={4000}
          onSave={(v) => {
            setLateStartTolerance(v);
            api.setLateStartToleranceMs(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
    </>
  );
}
