import { useState } from "react";
import type { BandInfo, OperatingMode } from "./ipc";

// Extracted out of App.tsx so RadioAudioSettings.tsx (under src/settings/)
// can reuse the exact same "Band & Frequency" picker Android's own
// RadioAudioSettings.kt row opens, rather than building a second one --
// App.tsx can't be imported from there without a circular App.tsx <->
// settings/ dependency (App.tsx imports SettingsScreen, which imports
// RadioAudioSettings).

export const MODE_LABEL: Record<OperatingMode, string> = { ft8: "FT8", ft4: "FT4", ft2: "FT2" };

// Cycle-time subtext shown on each mode tile in BandModeSheet, matching
// Android's BandModeSheet.kt (formatSlotSeconds: 15000->"15s", 7500->"7.5s",
// 3750->"3.75s").
const MODE_CYCLE_LABEL: Record<OperatingMode, string> = {
  ft8: "15s", ft4: "7.5s", ft2: "3.75s",
};

// Shared Band & Mode control, used identically by TopBar's title (the
// upper-left picker), the More panel's Band & Mode card, and Settings'
// Radio & Audio "Band & Frequency" row -- matching Android, which reuses
// one bandModeLabel construction everywhere rather than keeping independent
// pickers that could drift apart. `variant` only changes layout/sizing (a
// big borderless title vs. a bordered labeled card); all three open the
// same BandModeSheet. Matches Android's actual BandModeSheet.kt design (a
// bottom sheet: a 3-way mode toggle with cycle times, over a band list with
// a checkmark on the current selection) rather than inline toggle buttons --
// mode selection lives in this one sheet, not as separate buttons next to
// the label.
export function BandModePicker(props: {
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
export function BandModeSheet(props: {
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
