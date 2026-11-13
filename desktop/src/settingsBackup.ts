// Pure logic for Settings -> Advanced's backup/export filename, ported from
// the Android fork's own `SettingsBackup.defaultFileName`. The actual JSON
// building/parsing/sensitive-key filtering lives in Rust (`settings_backup.rs`,
// which already has direct access to the config table) rather than being
// duplicated here -- this is the one piece of that logic the frontend needs
// for itself, since the Blob-download trick (matching the existing ADIF
// export) needs a filename before the browser-style save even starts.

/** Suggested export file name, e.g. "ft8af-settings-2026-11-14-v0.1.2.json". */
export function defaultBackupFileName(date: string, appVersion: string): string {
  const safeVersion = (appVersion.trim() || "unknown").replace(/[^A-Za-z0-9._-]/g, "_");
  return `ft8af-settings-${date}-v${safeVersion}.json`;
}
