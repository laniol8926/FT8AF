import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { ClockSyncEvent } from "../ipc";
import { clampCorrectionMs, formatOffsetMs, stepCorrectionMs } from "../timeCorrection";
import { SettingsRow } from "./SettingsPrimitives";

/**
 * Time Sync, ported from the Android fork's own `TimeSyncSettings.kt`: a
 * manual correction stepper for offline operating (`timeCorrection.ts`,
 * ported from `TimeCorrection.kt`), plus NTP Clock Discipline -- now a real
 * toggle + configurable server (`disciplineClockFromNtp`/`ntpServer`,
 * `timesync.rs`'s `ntp_discipline_enabled`/`server_order`), not just a
 * status readout.
 *
 * Two sections stay deliberate, disclosed stubs rather than ported:
 * - **GPS Clock Discipline**: real Android hardware feature (issue #373),
 *   not dropped as Android-only cruft -- the user has GPS hardware for the
 *   Quadra, just not installed/configured yet, so this is future work once
 *   that's connected, not a permanent gap. Shown disabled with real copy
 *   until then.
 * - **Self-Syncing Clock** (auto-correct from decode DT medians): desktop's
 *   DT is deliberately informational-only -- `ClockSyncEvent.rx_offset_ms`
 *   is permanently zero because "unsafe RX auto-calibration was disabled"
 *   (see that field's own doc comment in `ipc.ts`, predating this port).
 *   Re-enabling an automatic clock-feedback loop needs its own careful,
 *   standalone safety review, not a blind port alongside a dozen other
 *   settings screens in one pass.
 */
export function TimeSyncSettings({ clock, onStatus }: { clock: ClockSyncEvent | null; onStatus: (s: string) => void }) {
  const [correctionMs, setCorrectionMs] = useState(0);
  const [ntpDiscipline, setNtpDiscipline] = useState(true);
  const [ntpServer, setNtpServer] = useState("");

  useEffect(() => {
    api.getConfig("clock_offset_ms").then((v) => v && setCorrectionMs(clampCorrectionMs(parseInt(v, 10) || 0)));
    api.getConfig("disciplineClockFromNtp").then((v) => setNtpDiscipline(v !== "0"));
    api.getConfig("ntpServer").then((v) => setNtpServer(v ?? ""));
  }, []);
  // The engine's own clock_offset_ms is the live source of truth once a sync
  // lands (NTP or this screen's own manual apply) -- keep the readout honest
  // rather than only reflecting what this screen itself last set.
  useEffect(() => {
    if (clock?.synced) setCorrectionMs(clampCorrectionMs(clock.offset_ms));
  }, [clock]);

  function apply(ms: number) {
    const clamped = clampCorrectionMs(ms);
    setCorrectionMs(clamped);
    api.setClockOffset(clamped);
  }

  function toggleNtpDiscipline(on: boolean) {
    setNtpDiscipline(on);
    api.setConfig("disciplineClockFromNtp", on ? "1" : "0");
  }

  function applyNtpServer(v: string) {
    setNtpServer(v);
    api.setConfig("ntpServer", v.trim());
  }

  return (
    <>
      <div className="panel">
        <h3>Manual Correction</h3>
        <div className="col">
          <div className="muted" style={{ fontSize: 12 }}>
            Nudge the app clock when you have no internet to sync. FT8 needs UTC within
            about 1 second. Adjust until the DT column on decoded stations centers near 0.
          </div>
          <div className="muted" style={{ fontSize: 11, fontWeight: 600, letterSpacing: "0.06em", textTransform: "uppercase" }}>
            Current correction
          </div>
          <div
            style={{
              fontSize: 32, fontWeight: 700, textAlign: "center", fontVariantNumeric: "tabular-nums",
              color: correctionMs === 0 ? "var(--text)" : "var(--accent)",
            }}
          >
            {formatOffsetMs(correctionMs)}
          </div>
          <div className="row" style={{ gap: 8 }}>
            {[-500, -100, 100, 500].map((delta) => (
              <button
                key={delta}
                style={{ flex: 1 }}
                disabled={ntpDiscipline}
                onClick={() => apply(stepCorrectionMs(correctionMs, delta))}
              >
                {delta > 0 ? "+" : ""}
                {(delta / 1000).toFixed(1)}
              </button>
            ))}
          </div>
          <button disabled={ntpDiscipline || correctionMs === 0} onClick={() => apply(0)} style={{ alignSelf: "center" }}>
            Reset to 0
          </button>
          {ntpDiscipline && (
            <div className="muted" style={{ fontSize: 12, textAlign: "center" }}>
              NTP clock discipline is controlling the clock — turn it off below to adjust manually.
            </div>
          )}
        </div>
      </div>

      <div className="panel">
        <h3>NTP Clock Discipline</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Discipline clock from NTP"
            description="Periodically sync from a network time server to align the FT8 cycle to true UTC. Overrides the manual correction while on. Needs internet access."
            toggle={ntpDiscipline}
            onToggleChange={toggleNtpDiscipline}
          />
        </div>
        <div className="field" style={{ marginTop: 10 }}>
          <label>NTP server</label>
          <input
            value={ntpServer}
            onChange={(e) => setNtpServer(e.target.value)}
            onBlur={(e) => applyNtpServer(e.target.value)}
            placeholder="e.g. pool.ntp.org"
            style={{ width: 220 }}
          />
          <span className="muted" style={{ display: "block", marginTop: 4, fontSize: 12 }}>
            Blank uses the built-in pool (time.cloudflare.com, time.google.com, pool.ntp.org).
          </span>
        </div>
        <div className="field">
          <label>Status</label>
          <div>{clock?.synced ? `Last offset applied: ${formatOffsetMs(clock.offset_ms)}` : "Waiting for the first NTP sync…"}</div>
        </div>
        <button
          onClick={() => {
            api.resyncTime();
            onStatus("Re-syncing clock to UTC…");
          }}
        >
          Sync now
        </button>
      </div>

      <div className="panel">
        <h3>GPS Clock Discipline</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Discipline clock from GPS"
            description="Use satellite time to align the FT8 cycle to true UTC. Overrides the manual correction while on. Needs a connected GPS receiver -- not yet installed/configured on this machine."
            toggle={false}
            onToggleChange={() => {}}
            disabled
          />
        </div>
      </div>

      <div className="panel">
        <h3>Self-Syncing Clock</h3>
        <div className="settings-category-list">
          <SettingsRow
            label="Auto-sync clock from decodes"
            description="Keeps the clock locked to the stations you hear, no internet or GPS needed. Not available on desktop yet -- decoded DT is informational only pending its own safety review before driving the clock automatically."
            toggle={false}
            onToggleChange={() => {}}
            disabled
          />
        </div>
      </div>
    </>
  );
}
