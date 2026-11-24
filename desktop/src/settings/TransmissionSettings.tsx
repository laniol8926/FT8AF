import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { HuntPriority } from "../hunt";
import { HUNT_PRIORITIES } from "../hunt";
import { SettingsRow, NumberInputDialog, ListPickerDialog } from "./SettingsPrimitives";

// Mirrors qso::DEFAULT_CQ_ANSWER_LIMIT -- the removed "Same as TX Watchdog"
// (0) option's replacement floor, used here only so a config read that
// still returns a stray/un-migrated 0 shows the same real number the
// engine would actually use, never a confusing "0".
const DEFAULT_CQ_ANSWER_LIMIT = 3;

const HUNT_PRIORITY_LABEL: Record<HuntPriority, string> = {
  latest: "Latest (most recent decode)",
  strongest: "Strongest signal",
  weakest: "Weakest signal",
  farthest: "Farthest station",
  pota_first: "POTA activators first",
  new_grid_first: "New grid first",
};

type OpenDialog =
  | null | "stopAfter" | "cqAnswerLimit" | "cqWatchdog" | "max73" | "swrThreshold" | "alcLow" | "alcHigh"
  | "tuneMethod" | "tuneMaxOn" | "tuneLevel" | "huntPriority" | "huntMinSnr";

const TUNE_METHOD_OPTIONS: { value: number; label: string }[] = [
  { value: 0, label: "Automatic" },
  { value: 1, label: "Internal tuner" },
  { value: 2, label: "Low power tone" },
];

/**
 * Transmission, ported from the Android fork's own `TransmissionSettings.kt`
 * -- built per the user's explicit instruction to implement the real logic
 * for TX Protection/Tune even where desktop currently has no live feed to
 * react to ("do all the logic... better to encorporate it now than find out
 * later we missed something"), rather than stub those pieces alongside the
 * genuinely out-of-scope ones. Most rows here are self-contained (load their
 * own value via `api.getConfig` on mount, save via a dedicated Tauri command
 * that also updates the live `Engine` -- not a bare `api.setConfig`, since
 * these all have real backend state to keep in sync) because nothing else
 * in the app reads them; only the Auto-Sequence section's Hunt row is a
 * controlled prop, since Hunt itself lives in `App.tsx`/`hunt.ts`, not here.
 *
 * - **TX/RX Split**: disclosed stub. Android's version is real
 *   split-*frequency* operation (a separate TX dial frequency from RX, like
 *   working a DX pileup) -- desktop's whole architecture assumes one dial
 *   frequency shared by TX and RX (`dial_hz`), so this needs real new
 *   split-VFO rig control, not a quick toggle.
 * - **Hold TX freq**: real, and NOT actually gated behind Split the way the
 *   above might suggest -- checked against Android's own
 *   `FT8TransmitSignal.shouldFollowTargetFreq`/`holdTxFreq`: it's purely an
 *   audio-offset choice (does answering a station move your TX tone onto
 *   theirs, or keep it where it was), nothing to do with a second dial
 *   frequency. Desktop's `answer` IPC command already carries a
 *   `delta_freq` field (0 = keep current offset, matching both the
 *   existing UI click-to-answer behavior and Android's own `holdTxFreq`
 *   default of `true`) -- this toggle (Android's own `holdTxFreq` key,
 *   default on, matching current behavior exactly) is purely a frontend
 *   decision in `App.tsx`'s Call button: off sends the target's own
 *   decoded frequency as `delta_freq` instead of `0`.
 * - **Clear decodes on band/mode change**: real (`clearOnBandModeChange`,
 *   Android's own key/default) -- wipes the decode list and resets the TX
 *   target to CQ (if something was running) on a band or mode change, see
 *   `engine.rs`'s `clear_decodes_and_target`.
 * - **TX Watchdog** ("Stop After") / **Max 73 Sends**: real, new give-up
 *   counters built into `qso.rs` this pass (desktop had no retry-limit
 *   concept of any kind before) -- see `qso::should_give_up_target`/
 *   `has_reached_max_73_sends`.
 * - **CQ Watchdog**: real, and genuinely new to BOTH apps -- confirmed by
 *   reading Android's `FT8TransmitSignal.java`: `noReplyLimit`/
 *   `shouldGiveUpTarget` only ever apply once a target is picked, so a bare
 *   Call CQ with nobody answering ran forever on both platforms (caught
 *   live: 4 real CQ transmissions over 90s on a dummy load with no sign of
 *   stopping). Deliberately a SEPARATE limit from TX Watchdog, not a reuse
 *   of `noReplyLimit` -- an operator may want a flaky mid-QSO partner
 *   dropped quickly while still letting a bare CQ run far longer (or
 *   unlimited) waiting to be heard. `0` = unlimited, the default, since
 *   unattended/"robot" CQ operation depends on it never giving up on its
 *   own. See `qso::should_give_up_cq`/`QsoEngine::cq_no_reply_limit`.
 * - **TX Protection**: SWR Halt is real end-to-end (`tx_protection::
 *   should_halt_for_swr`, wired into the meter poll + `maybe_transmit`/
 *   `start_tune`), using a plain SWR ratio (e.g. "3.0") rather than
 *   Android's normalized 0-255 scale, since the QMX's own CAT reply already
 *   gives an exact ratio. ALC Auto-Volume's full accumulate/correct pure
 *   logic is built and its settings persist for real, but it is currently
 *   **inert**: the QMX's firmware CAT manual documents no ALC query at all
 *   (only `SW;`/`PC;`), so nothing feeds it yet -- built now per explicit
 *   instruction, ready for the day a real ALC source exists.
 * - **Tune**: Method/Max-on-seconds/Independent level are all real. "Internal
 *   tuner" refuses to key a surprise carrier rather than silently falling
 *   back to the tone, matching Android's own safety reasoning -- but no
 *   currently-supported desktop rig backend exposes an ATU-start CAT
 *   command, so "Automatic" and "Low power tone" behave identically today
 *   (disclosed in the method row's own description).
 * - **Auto-Sequence**: Hunt (auto-answer CQ) is desktop's existing Hunt
 *   feature, just exposed here under Android's row label/description
 *   instead of the landing page's old inline panel. Auto-CQ after QSO is
 *   real (`autoCQAfterQSO`) -- note its *default* is kept at desktop's own
 *   historical always-on behavior, not Android's off-by-default, to avoid
 *   silently changing how every existing install behaves (see `engine.rs`'s
 *   `Engine::new` doc comment). Hunt + CQ, Auto-call Followed, and Fast
 *   turnaround are disclosed stubs: each needs new auto-sequencer logic
 *   (idle-CQ/Hunt switching, decode-window timing in the same
 *   safety-sensitive class as Advanced's PTT/TX Delay) that didn't fit in
 *   this pass. Auto clear TX offset is also a disclosed stub -- it needs a
 *   real band-activity clear-spot scan (`ClearFrequencyFinder` in
 *   Android), new DSP/logic work. Work strongest caller first is now real
 *   (`pileupStrongestFirst`, Android's own key name) -- it orders the
 *   caller-queue bar's chips (`App.tsx`'s `ActiveQsoPanelView`, `qso.rs`'s
 *   `QueuedCaller`/`offer_callers`); auto-working the queue without a tap
 *   is a separate, not-yet-built follow-on (see that feature's own plan
 *   doc), so this setting only affects display order for now.
 */
export function TransmissionSettings({
  huntEnabled,
  onHuntEnabled,
  huntPriority,
  onHuntPriority,
  huntAvoidPileups,
  onHuntAvoidPileups,
  huntMinSnr,
  onHuntMinSnr,
  holdTxFreq,
  onHoldTxFreq,
  pileupStrongestFirst,
  onPileupStrongestFirst,
}: {
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
}) {
  const [openDialog, setOpenDialog] = useState<OpenDialog>(null);

  const [clearOnChange, setClearOnChange] = useState(true);
  const [stopAfter, setStopAfter] = useState(0);
  const [cqAnswerLimit, setCqAnswerLimit] = useState(DEFAULT_CQ_ANSWER_LIMIT);
  const [cqWatchdog, setCqWatchdog] = useState(0);
  const [max73, setMax73] = useState(0);
  const [swrHaltEnabled, setSwrHaltEnabled] = useState(false);
  const [swrThreshold, setSwrThreshold] = useState(3.0);
  const [alcEnabled, setAlcEnabled] = useState(false);
  const [alcLow, setAlcLow] = useState(60);
  const [alcHigh, setAlcHigh] = useState(100);
  const [tuneMethod, setTuneMethod] = useState(0);
  const [tuneMaxOn, setTuneMaxOn] = useState(10);
  const [tuneLevelIndependent, setTuneLevelIndependent] = useState(false);
  const [tuneLevelPct, setTuneLevelPct] = useState(50);
  const [autoCqAfterQso, setAutoCqAfterQso] = useState(true);

  useEffect(() => {
    api.getConfig("clearOnBandModeChange").then((v) => setClearOnChange(v !== "0"));
    api.getConfig("noReplyLimit").then((v) => setStopAfter(v ? parseInt(v, 10) || 0 : 0));
    api.getConfig("cqAnswerLimit").then((v) => {
      const n = v ? parseInt(v, 10) || 0 : 0;
      setCqAnswerLimit(n > 0 ? n : DEFAULT_CQ_ANSWER_LIMIT);
    });
    api.getConfig("cqNoReplyLimit").then((v) => setCqWatchdog(v ? parseInt(v, 10) || 0 : 0));
    api.getConfig("max73Sends").then((v) => setMax73(v ? parseInt(v, 10) || 0 : 0));
    api.getConfig("swrHaltEnabled").then((v) => setSwrHaltEnabled(v === "1"));
    api.getConfig("swrHaltThreshold").then((v) => setSwrThreshold(v ? parseFloat(v) || 3.0 : 3.0));
    api.getConfig("autoVolumeEnabled").then((v) => setAlcEnabled(v === "1"));
    api.getConfig("alcTargetLow").then((v) => setAlcLow(v ? parseInt(v, 10) || 60 : 60));
    api.getConfig("alcTargetHigh").then((v) => setAlcHigh(v ? parseInt(v, 10) || 100 : 100));
    api.getConfig("tuneMethod").then((v) => setTuneMethod(v ? parseInt(v, 10) || 0 : 0));
    api.getConfig("tuneMaxOnSeconds").then((v) => setTuneMaxOn(v ? parseInt(v, 10) || 10 : 10));
    api.getConfig("tuneLevelIndependent").then((v) => setTuneLevelIndependent(v === "1"));
    api.getConfig("tuneLevel").then((v) => setTuneLevelPct(v ? Math.round(parseFloat(v) * 100) || 50 : 50));
    api.getConfig("autoCQAfterQSO").then((v) => setAutoCqAfterQso(v == null ? true : v === "1"));
  }, []);

  return (
    <>
      <div className="panel">
        <h3>TX/RX Split</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="TX/RX Split"
            description="Transmit on a different frequency than receive -- needs real split-VFO rig control desktop doesn't have yet (one shared dial frequency drives both TX and RX today)"
            disabled
          />
          <SettingsRow
            label="Hold TX freq"
            description="Keep your TX offset when answering a station instead of moving it to theirs"
            toggle={holdTxFreq}
            onToggleChange={onHoldTxFreq}
          />
          <SettingsRow
            label="Clear decodes on band/mode change"
            description="Wipe the decode list and reset the TX target to CQ when you change band or mode"
            toggle={clearOnChange}
            onToggleChange={(v) => {
              setClearOnChange(v);
              api.setClearOnBandModeChange(v);
            }}
          />
        </div>
      </div>

      <div className="panel">
        <h3>TX Watchdog</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="TX Watchdog"
            description="Stop calling after N unanswered attempts"
            value={stopAfter === 0 ? "Off" : `${stopAfter} tries`}
            onClick={() => setOpenDialog("stopAfter")}
          />
          <SettingsRow
            label="CQ Answer Attempts"
            description="Stop trying to answer a station's CQ after N unanswered attempts -- separate from (and never more than) TX Watchdog, which governs retries once a QSO is actually under way."
            value={`${cqAnswerLimit} tries`}
            onClick={() => setOpenDialog("cqAnswerLimit")}
          />
          <SettingsRow
            label="CQ Watchdog"
            description="Stop calling CQ after N unanswered cycles with nobody replying -- 0/unlimited for unattended operation"
            value={cqWatchdog === 0 ? "Unlimited" : `${cqWatchdog} cycles`}
            onClick={() => setOpenDialog("cqWatchdog")}
          />
          <SettingsRow
            label="Max 73 Sends"
            description="Maximum RR73/73 transmissions per contact before moving on"
            value={max73 === 0 ? "Auto" : String(max73)}
            onClick={() => setOpenDialog("max73")}
          />
        </div>
      </div>

      <div className="panel">
        <h3>TX Protection</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="SWR Halt"
            description="Stop transmitting and lock out TX/Tune when SWR exceeds the threshold, until explicitly cleared -- never trips during a Tune (that's what Tune is for)"
            toggle={swrHaltEnabled}
            onToggleChange={(v) => {
              setSwrHaltEnabled(v);
              api.setSwrHaltEnabled(v);
            }}
          />
          <SettingsRow
            label="SWR threshold"
            description="SWR ratio that triggers the halt"
            value={`${swrThreshold.toFixed(1)}:1`}
            onClick={() => setOpenDialog("swrThreshold")}
          />
          <SettingsRow
            label="ALC Auto-Volume"
            description="Automatically trim TX volume to keep ALC in range -- the QMX's own CAT firmware has no ALC reading at all yet, so this has nothing to react to until a future rig/feed adds one. Settings persist now so nothing is missed later."
            toggle={alcEnabled}
            onToggleChange={(v) => {
              setAlcEnabled(v);
              // No dedicated engine command exists for this yet (see this
              // row's own description -- nothing in engine.rs reads it);
              // the config row is still written so the choice survives a
              // future build that does.
              api.setConfig("autoVolumeEnabled", v ? "1" : "0");
            }}
          />
          <SettingsRow
            label="ALC target low"
            description="Lower bound of the target ALC window (0-255)"
            value={String(alcLow)}
            onClick={() => setOpenDialog("alcLow")}
          />
          <SettingsRow
            label="ALC target high"
            description="Upper bound of the target ALC window (0-255)"
            value={String(alcHigh)}
            onClick={() => setOpenDialog("alcHigh")}
          />
        </div>
      </div>

      <div className="panel">
        <h3>Tune</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Tune method"
            description="Automatic starts the rig's internal tuner over CAT when available, otherwise plays the low power tone -- no connected rig backend exposes an ATU-start CAT command yet, so Automatic and Low power tone behave the same for now; Internal tuner refuses rather than surprise you with a carrier"
            value={TUNE_METHOD_OPTIONS.find((o) => o.value === tuneMethod)?.label ?? "Automatic"}
            onClick={() => setOpenDialog("tuneMethod")}
          />
          <SettingsRow
            label="Tune max-on time"
            description="Hard cap on a single Tune carrier, even if you forget to stop it"
            value={`${tuneMaxOn}s`}
            onClick={() => setOpenDialog("tuneMaxOn")}
          />
          <SettingsRow
            label="Independent tune level"
            description="Use a separate TX level for Tune instead of your normal TX volume"
            toggle={tuneLevelIndependent}
            onToggleChange={(v) => {
              setTuneLevelIndependent(v);
              api.setTuneLevelIndependent(v);
            }}
          />
          <SettingsRow
            label="Tune level"
            description="TX level used for Tune when independent"
            value={`${tuneLevelPct}%`}
            disabled={!tuneLevelIndependent}
            onClick={tuneLevelIndependent ? () => setOpenDialog("tuneLevel") : undefined}
          />
        </div>
      </div>

      <div className="panel">
        <h3>Auto-Sequence</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Hunt (auto-answer CQ)"
            description="Proactively call stations calling CQ. Same as the HUNT button on the TX bar. Off = run CQ and work only stations that answer you."
            toggle={huntEnabled}
            onToggleChange={(v) => {
              onHuntEnabled(v);
              api.setConfig("hunt_enabled", String(v));
            }}
          />
          <SettingsRow
            label="Hunt priority"
            description="Which eligible CQ counts as &quot;best&quot; each cycle"
            value={HUNT_PRIORITY_LABEL[huntPriority]}
            onClick={() => setOpenDialog("huntPriority")}
          />
          <SettingsRow
            label="Avoid pileups"
            description="Prefer CQs no one else is answering this cycle"
            toggle={huntAvoidPileups}
            onToggleChange={onHuntAvoidPileups}
          />
          <SettingsRow
            label="Minimum SNR"
            description="Floor below which Hunt won't answer a CQ -- blank disables it"
            value={huntMinSnr == null ? "Off" : `${huntMinSnr} dB`}
            onClick={() => setOpenDialog("huntMinSnr")}
          />
          <SettingsRow
            label="Hunt + CQ"
            description="Call CQ when Hunt is idle (no stations heard). When a CQ is detected, automatically stop CQ and answer it -- needs a new idle-CQ/Hunt auto-switcher not built yet"
            disabled
          />
          <SettingsRow
            label="Auto-call Followed"
            description="Continue calling followed callsigns automatically -- the Hunt watchlist table exists (follow_callsigns) but isn't wired into Hunt's candidate ranking yet"
            disabled
          />
          <SettingsRow
            label="Fast turnaround"
            description="Decode ~1s earlier so you can answer a CQ on the next slot instead of waiting -- deferred with Advanced's PTT/TX Delay timing knobs, same TX-timing-safety review needed first"
            disabled
          />
          <SettingsRow
            label="Auto-CQ after QSO"
            description="Keep calling CQ automatically after each completed contact -- no re-tap. Stays on your frequency (ignores Hunt)."
            toggle={autoCqAfterQso}
            onToggleChange={(v) => {
              setAutoCqAfterQso(v);
              api.setAutoCqAfterQso(v);
            }}
          />
          <SettingsRow
            label="Work strongest caller first"
            description="Orders the caller-queue bar's chips by signal strength instead of who called first. Only affects the chip order and this setting's own display order -- auto-working the queue without a tap isn't built yet, so this doesn't change anything on its own until you tap a chip."
            toggle={pileupStrongestFirst}
            onToggleChange={onPileupStrongestFirst}
          />
          <SettingsRow
            label="Auto clear TX offset"
            description="When calling CQ, pick a clear spot from recent band activity and move away if it becomes occupied -- needs a new band-activity scan, not built yet"
            disabled
          />
        </div>
      </div>

      {openDialog === "stopAfter" && (
        <NumberInputDialog
          title="TX Watchdog"
          description="Stop calling after this many unanswered attempts. 0 = unlimited."
          value={stopAfter}
          min={0}
          max={30}
          onSave={(v) => {
            setStopAfter(v);
            api.setTxWatchdog(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "cqAnswerLimit" && (
        <NumberInputDialog
          title="CQ Answer Attempts"
          description="Stop trying to answer a station's CQ (before he's ever replied) after this many unanswered attempts. Capped at TX Watchdog's own value when that's set -- a CQ-answer limit higher than TX Watchdog is clamped down, not honored as typed."
          value={cqAnswerLimit}
          min={1}
          max={30}
          onSave={(v) => {
            setCqAnswerLimit(v);
            api.setCqAnswerLimit(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "cqWatchdog" && (
        <NumberInputDialog
          title="CQ Watchdog"
          description="Stop calling CQ after this many unanswered cycles (~15s each) with nobody replying. 0 = unlimited -- required for unattended/robot operation."
          value={cqWatchdog}
          min={0}
          max={200}
          onSave={(v) => {
            setCqWatchdog(v);
            api.setCqWatchdog(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "max73" && (
        <NumberInputDialog
          title="Max 73 Sends"
          description="Cap RR73/73 retransmissions per contact. 0 = Auto."
          value={max73}
          min={0}
          max={20}
          onSave={(v) => {
            setMax73(v);
            api.setMax73Sends(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "swrThreshold" && (
        <SwrThresholdDialog
          value={swrThreshold}
          onSave={(v) => {
            setSwrThreshold(v);
            api.setSwrHaltThreshold(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "alcLow" && (
        <NumberInputDialog
          title="ALC target low"
          value={alcLow}
          min={10}
          max={200}
          onSave={(v) => {
            setAlcLow(v);
            api.setConfig("alcTargetLow", String(v));
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "alcHigh" && (
        <NumberInputDialog
          title="ALC target high"
          value={alcHigh}
          min={alcLow + 10}
          max={250}
          onSave={(v) => {
            setAlcHigh(v);
            api.setConfig("alcTargetHigh", String(v));
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "huntPriority" && (
        <ListPickerDialog
          title="Hunt priority"
          options={HUNT_PRIORITIES.map((p) => ({ value: p, label: HUNT_PRIORITY_LABEL[p] }))}
          value={huntPriority}
          onSelect={(v) => {
            onHuntPriority(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "huntMinSnr" && (
        <HuntMinSnrDialog
          value={huntMinSnr}
          onSave={(v) => {
            onHuntMinSnr(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "tuneMethod" && (
        <ListPickerDialog
          title="Tune method"
          options={TUNE_METHOD_OPTIONS}
          value={tuneMethod}
          onSelect={(v) => {
            setTuneMethod(v);
            api.setTuneMethod(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "tuneMaxOn" && (
        <NumberInputDialog
          title="Tune max-on time"
          description="Hard cap on a single Tune carrier (2-60s)"
          value={tuneMaxOn}
          min={2}
          max={60}
          unit="s"
          onSave={(v) => {
            setTuneMaxOn(v);
            api.setTuneMaxOnSeconds(v);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
      {openDialog === "tuneLevel" && (
        <NumberInputDialog
          title="Tune level"
          value={tuneLevelPct}
          min={5}
          max={100}
          unit="%"
          onSave={(v) => {
            setTuneLevelPct(v);
            api.setTuneLevel(v / 100);
            setOpenDialog(null);
          }}
          onCancel={() => setOpenDialog(null)}
        />
      )}
    </>
  );
}

/** Hunt's minimum-SNR floor: an editable integer OR blank (disables the
 *  floor entirely) -- `NumberInputDialog` has no "blank" state, so this is
 *  its own tiny dialog, same reasoning as `SwrThresholdDialog`. */
function HuntMinSnrDialog({
  value,
  onSave,
  onCancel,
}: {
  value: number | null;
  onSave: (v: number | null) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(value == null ? "" : String(value));
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal spectrum-width-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>Minimum SNR</h3>
        <div className="muted" style={{ marginBottom: 10 }}>Floor below which Hunt won't answer a CQ (dB) -- blank disables it</div>
        <label className="spectrum-width-field">
          <span className="spectrum-width-hint">e.g. -15</span>
          <input
            type="number"
            inputMode="numeric"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Off"
            autoFocus
          />
        </label>
        <div className="actions">
          <button className="modal-text-btn" onClick={onCancel}>Cancel</button>
          <button
            className="modal-text-btn modal-text-btn-accent"
            onClick={() => {
              const trimmed = draft.trim();
              if (trimmed === "") {
                onSave(null);
                return;
              }
              const n = parseInt(trimmed, 10);
              if (!Number.isNaN(n)) onSave(n);
            }}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

/** SWR threshold needs one decimal (a ratio like "3.0"), unlike every other
 *  row here -- `NumberInputDialog` is integer-only, so this is its own tiny
 *  dialog rather than stretching that shared primitive for one field. */
function SwrThresholdDialog({
  value,
  onSave,
  onCancel,
}: {
  value: number;
  onSave: (v: number) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(value.toFixed(1));
  const parsed = parseFloat(draft);
  const valid = Number.isFinite(parsed) && parsed >= 1.0 && parsed <= 10.0;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal spectrum-width-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>SWR threshold</h3>
        <div className="muted" style={{ marginBottom: 10 }}>SWR ratio that triggers the halt (1.0-10.0)</div>
        <label className="spectrum-width-field">
          <span className="spectrum-width-hint">1.0 - 10.0</span>
          <input
            type="number"
            inputMode="decimal"
            min={1.0}
            max={10.0}
            step={0.1}
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
