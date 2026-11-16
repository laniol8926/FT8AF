/**
 * Shared Settings UI primitives, ported from the Android fork's own
 * `SettingsRow`/`SettingsScreen.kt` shape -- a landing page of category rows
 * drilling into separate full-screen panels, rather than desktop's old
 * single long scroll. `SettingsRow`/`SettingsSection`/`SettingsDetailScaffold`
 * were Phase 1's; `ListPickerDialog`/`NumberInputDialog`/`SettingsSegmentedRow`
 * were added for Radio & Audio's row+drill-down redesign (each enumerable
 * field -- Rig Model, Baud Rate, Audio Input, etc. -- is its own tappable row
 * opening a picker, matching the real app exactly). `TextListDialog` was
 * added for Decode Filters' blocklist/watchlist editors. `InfoDialog` is
 * still deferred to whichever later phase first needs one.
 */

import { useState } from "react";
import type { ReactNode } from "react";

export function SettingsSection({ title }: { title: string }) {
  return <div className="settings-section-header">{title}</div>;
}

export function SettingsRow({
  label,
  description,
  value,
  onClick,
  disabled,
  toggle,
  onToggleChange,
}: {
  label: string;
  description?: string;
  value?: string;
  onClick?: () => void;
  disabled?: boolean;
  /** Renders a pill switch instead of a chevron/value, matching Android's
   *  own `SettingsRow(toggle:, onToggleChange:)` (e.g. "Auto-update Grid"). */
  toggle?: boolean;
  onToggleChange?: (v: boolean) => void;
}) {
  const clickable = !!onClick && !disabled;
  return (
    <div
      className={"settings-row" + (disabled ? " disabled" : "") + (clickable ? " clickable" : "")}
      onClick={clickable ? onClick : undefined}
      role={clickable ? "button" : undefined}
    >
      <div className="settings-row-text">
        <div className="settings-row-label">{label}</div>
        {description && <div className="settings-row-desc muted">{description}</div>}
      </div>
      {/* A row can carry both a toggle and a value+chevron at once (e.g.
       *  Android's QRZ.com row: an inline enable switch alongside a
       *  "Configured"/"Not configured" value and a chevron into the API
       *  key dialog) -- these aren't mutually exclusive. */}
      {value !== undefined && <div className="settings-row-value muted">{value}</div>}
      {onToggleChange && (
        <label className="settings-toggle" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={!!toggle}
            disabled={disabled}
            onChange={(e) => onToggleChange(e.target.checked)}
          />
          <span className="settings-toggle-track" />
        </label>
      )}
      {clickable && <div className="settings-row-chevron">&rsaquo;</div>}
    </div>
  );
}

/** A row whose trailing content is a full-width inline segmented control
 *  (e.g. "Receive Audio Channel": Mix/Left/Right) rather than a chevron or
 *  toggle -- matches Android's own `SettingsRow` + a 3-way `SegmentedButton`
 *  stacked underneath the description, applying live with no dialog. */
export function SettingsSegmentedRow<T extends string | number>({
  label,
  description,
  options,
  value,
  onChange,
  disabled,
}: {
  label: string;
  description?: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={"settings-row settings-row-segmented" + (disabled ? " disabled" : "")}>
      <div className="settings-row-text">
        <div className="settings-row-label">{label}</div>
        {description && <div className="settings-row-desc muted">{description}</div>}
        <div className="settings-segment-group">
          {options.map((o) => (
            <button
              key={String(o.value)}
              type="button"
              disabled={disabled}
              className={"settings-segment" + (o.value === value ? " active" : "")}
              onClick={() => onChange(o.value)}
            >
              {o.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Modal single-select list, matching Android's own `ListPickerDialog` --
 *  used for every enumerable Radio & Audio field (Rig Model, Connection,
 *  Baud Rate, Audio Input/Output, etc). `searchable` adds a filter box,
 *  worth it only for a long list like the ~300 Hamlib rig models. */
export function ListPickerDialog<T extends string | number>({
  title,
  options,
  value,
  onSelect,
  onCancel,
  searchable,
}: {
  title: string;
  options: { value: T; label: string }[];
  value: T;
  onSelect: (v: T) => void;
  onCancel: () => void;
  searchable?: boolean;
}) {
  const [query, setQuery] = useState("");
  const filtered = searchable && query.trim()
    ? options.filter((o) => o.label.toLowerCase().includes(query.trim().toLowerCase()))
    : options;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal list-picker-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>{title}</h3>
          <button className="modal-close" aria-label="Close" onClick={onCancel}>✕</button>
        </div>
        {searchable && (
          <input
            className="list-picker-search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search…"
            autoFocus
          />
        )}
        <div className="list-picker-options">
          {filtered.length === 0 && <div className="muted" style={{ padding: 12 }}>No matches.</div>}
          {filtered.map((o) => (
            <div
              key={String(o.value)}
              className={"list-picker-option" + (o.value === value ? " active" : "")}
              onClick={() => onSelect(o.value)}
              role="button"
            >
              <span>{o.label}</span>
              {o.value === value && <span className="list-picker-check">✓</span>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Modal bounded-number entry, matching Android's own `NumberInputDialog` --
 *  a centered Cancel/Save dialog (not a live-drag popover, so dragging
 *  doesn't spam the backend with a config rebuild per pixel -- same reasoning
 *  as App.tsx's pre-existing `SpectrumWidthDialog`, which this doesn't
 *  replace since it's used outside Settings too; this is for the Settings
 *  rows that want the same shape). */
export function NumberInputDialog({
  title,
  description,
  value,
  min,
  max,
  step = 1,
  unit,
  onSave,
  onCancel,
}: {
  title: string;
  description?: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onSave: (v: number) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(String(Math.round(value)));
  const parsed = parseInt(draft, 10);
  const valid = Number.isFinite(parsed) && parsed >= min && parsed <= max;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal spectrum-width-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        {description && <div className="muted" style={{ marginBottom: 10 }}>{description}</div>}
        <label className="spectrum-width-field">
          <span className="spectrum-width-hint">{min}–{max}{unit ? ` ${unit}` : ""}</span>
          <input
            type="number"
            inputMode="numeric"
            min={min}
            max={max}
            step={step}
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

/** Modal free-text list editor (comma/space-separated), matching Android's
 *  own `TextListDialog` -- used for the callsign blocklist (exact/prefix/
 *  keyword), the Hunt/worked "from list" editor, and the Needed-DX
 *  watchlist. The caller owns parsing (each list has its own match
 *  semantics); this just edits the raw string. */
export function TextListDialog({
  title,
  description,
  initialValue,
  onSave,
  onCancel,
}: {
  title: string;
  description?: string;
  initialValue: string;
  onSave: (value: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initialValue);
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        {description && <div className="muted" style={{ marginBottom: 10, fontSize: 12 }}>{description}</div>}
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          rows={4}
          autoFocus
          style={{ textTransform: "uppercase" }}
        />
        <div className="actions">
          <button className="modal-text-btn" onClick={onCancel}>Cancel</button>
          <button className="modal-text-btn modal-text-btn-accent" onClick={() => onSave(draft)}>Save</button>
        </div>
      </div>
    </div>
  );
}

export function SettingsDetailScaffold({
  title,
  onBack,
  children,
}: {
  title: string;
  onBack: () => void;
  children: ReactNode;
}) {
  return (
    <div className="settings-detail">
      <div className="settings-detail-header">
        <button type="button" className="settings-back" onClick={onBack} aria-label="Back to Settings" title="Back">
          &larr;
        </button>
        <h3>{title}</h3>
      </div>
      <div className="settings-detail-body">{children}</div>
    </div>
  );
}
