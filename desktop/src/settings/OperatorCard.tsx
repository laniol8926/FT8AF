/**
 * Operator identity card, ported from the Android fork's own
 * `OperatorCard.kt` almost verbatim (amber-tinted background, avatar +
 * callsign/grid, a divider, then rig/antenna/power mini-stats) -- pinned at
 * the top of the Settings landing page. Desktop has no QRZ photo fetch, so
 * the avatar is always the initials fallback Android itself uses when a
 * photo isn't available.
 */
export function initialsFor(callsign: string): string {
  const letters = callsign.replace(/[^A-Za-z0-9]/g, "").slice(0, 2).toUpperCase();
  return letters || "?";
}

export function OperatorCard({
  callsign,
  grid,
  rigName,
  antenna,
  power,
  onClick,
}: {
  callsign: string;
  grid: string;
  rigName: string;
  antenna: string;
  power: string;
  onClick: () => void;
}) {
  return (
    <div className="operator-card" onClick={onClick} role="button">
      <div className="operator-card-top">
        <div className="operator-avatar">{initialsFor(callsign)}</div>
        <div className="operator-identity">
          <div className="operator-callsign">{callsign.toUpperCase() || "NO CALL"}</div>
          <div className="operator-grid">{grid.toUpperCase() || "No grid set"}</div>
          <div className="operator-tap-hint">Tap to edit</div>
        </div>
      </div>
      <hr className="operator-divider" />
      <div className="operator-stats">
        <div className="operator-stat">
          <div className="operator-stat-label">Rig</div>
          <div className="operator-stat-value">{rigName}</div>
        </div>
        <div className="operator-stat">
          <div className="operator-stat-label">Antenna</div>
          <div className="operator-stat-value">{antenna}</div>
        </div>
        <div className="operator-stat">
          <div className="operator-stat-label">Power</div>
          <div className="operator-stat-value">{power}</div>
        </div>
      </div>
    </div>
  );
}
