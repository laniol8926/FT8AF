import { useState } from "react";

/**
 * Modal for editing callsign/grid/antenna/power together, ported from the
 * Android fork's own `EditOperatorDialog` -- opened by tapping the
 * `OperatorCard` rather than drilling into a separate "Station" screen
 * (Android has no such screen either; this dialog **is** Station editing).
 * No format validation on any field, matching Android (same as the old
 * desktop Station screen).
 */
export function EditOperatorDialog({
  initialCallsign,
  initialGrid,
  initialAntenna,
  initialPower,
  onCancel,
  onSave,
}: {
  initialCallsign: string;
  initialGrid: string;
  initialAntenna: string;
  initialPower: string;
  onCancel: () => void;
  onSave: (callsign: string, grid: string, antenna: string, power: string) => void;
}) {
  const [callsign, setCallsign] = useState(initialCallsign);
  const [grid, setGrid] = useState(initialGrid);
  const [antenna, setAntenna] = useState(initialAntenna);
  const [power, setPower] = useState(initialPower);

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Edit Operator Identity</h3>
        <div className="col">
          <div className="field">
            <label>Callsign</label>
            <input value={callsign} onChange={(e) => setCallsign(e.target.value.toUpperCase())} placeholder="K0XYZ" autoFocus />
          </div>
          <div className="field">
            <label>Grid (Maidenhead)</label>
            <input value={grid} onChange={(e) => setGrid(e.target.value.toUpperCase())} placeholder="EN37" />
          </div>
          <div className="field">
            <label>Antenna</label>
            <input value={antenna} onChange={(e) => setAntenna(e.target.value)} placeholder="e.g. EFHW" />
          </div>
          <div className="field">
            <label>Power (watts)</label>
            <input value={power} onChange={(e) => setPower(e.target.value)} placeholder="e.g. 100" style={{ width: 100 }} />
          </div>
        </div>
        <div className="actions">
          <button className="modal-text-btn" onClick={onCancel}>Cancel</button>
          <button
            className="modal-text-btn modal-text-btn-accent"
            onClick={() => onSave(callsign, grid, antenna, power)}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
