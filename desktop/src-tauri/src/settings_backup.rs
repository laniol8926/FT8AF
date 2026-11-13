//! Settings backup/export/import, ported from the Android fork's own
//! `SettingsBackup.kt`. The backup is the app's whole `config` key/value
//! table serialized verbatim (sensitive keys excluded unless the operator
//! opts in), plus the Hunt watchlist (`follow_callsigns`) as a top-level
//! array, matching Android's real on-disk format exactly (confirmed
//! against the user's own real export, `ft8af-settings-*.json`, read
//! earlier in this port). Desktop has no separate SharedPreferences-style
//! "prefs" files the way Android's fork does -- custom bands/theme/map
//! style all already live in the one `config` table -- so the optional
//! "prefs" section Android's format allows is never written here, and
//! simply ignored on import if present (matches Android's own
//! forward-compat stance: "older builds read only config, so the format
//! version doesn't change").

use serde::Serialize;
use std::collections::BTreeMap;

/// Bumped when the on-disk backup schema changes incompatibly.
pub const FORMAT_VERSION: i64 = 1;

/// Marker written to identify our files and reject unrelated JSON on import.
pub const APP_NAME: &str = "FT8AF";

/// Config keys holding credentials/secrets -- excluded from an export
/// unless the operator explicitly opts in, so a shared or cloud-stored
/// backup never leaks API keys or passwords by default. Matches Android's
/// own `SettingsBackup.SENSITIVE_KEYS` exactly.
pub const SENSITIVE_KEYS: &[&str] = &[
    "cloudlogApiKey",
    "qrzApiKey",
    "qrzXmlUsername",
    "qrzXmlPassword",
    "wrlApiKey",
    "icomUserName",
    "icomPassword",
    "lotwUsername",
    "lotwPassword",
];

pub fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_KEYS.contains(&key)
}

/// Drop sensitive keys from `config` unless `include_sensitive`.
pub fn filter_config(config: &[(String, String)], include_sensitive: bool) -> BTreeMap<String, String> {
    config
        .iter()
        .filter(|(k, _)| include_sensitive || !is_sensitive_key(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[derive(Serialize)]
struct BackupFile {
    #[serde(rename = "appName")]
    app_name: &'static str,
    #[serde(rename = "formatVersion")]
    format_version: i64,
    #[serde(rename = "appVersion")]
    app_version: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "includesSensitive")]
    includes_sensitive: bool,
    config: BTreeMap<String, String>,
    #[serde(rename = "followCallsigns", skip_serializing_if = "Vec::is_empty")]
    follow_callsigns: Vec<String>,
}

/// Serialize `config` (+ the Hunt watchlist) to the backup JSON text, keys
/// sorted (via `BTreeMap`) for a deterministic, diff-friendly export --
/// matches Android's own `buildBackupJson`.
pub fn build_backup_json(
    config: &[(String, String)],
    include_sensitive: bool,
    app_version: &str,
    created_at: &str,
    mut follow_callsigns: Vec<String>,
) -> String {
    follow_callsigns.sort();
    let backup = BackupFile {
        app_name: APP_NAME,
        format_version: FORMAT_VERSION,
        app_version: app_version.to_string(),
        created_at: created_at.to_string(),
        includes_sensitive: include_sensitive,
        config: filter_config(config, include_sensitive),
        follow_callsigns,
    };
    serde_json::to_string_pretty(&backup).unwrap_or_default()
}

/// Metadata + config parsed from a backup file.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ParsedBackup {
    pub format_version: i64,
    pub app_version: String,
    pub created_at: String,
    pub config: BTreeMap<String, String>,
    pub follow_callsigns: Vec<String>,
}

/// Parse and validate backup JSON. Returns a user-facing error message
/// (shown to the operator) when the text isn't a recognizable FT8AF
/// backup, or when its format version is newer than this build
/// understands -- matches Android's own `parseBackupJson` exactly,
/// including its error copy.
pub fn parse_backup_json(text: &str) -> Result<ParsedBackup, String> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "Not a valid backup file (invalid JSON).".to_string())?;
    // The appName marker is what actually distinguishes our backups from
    // any other JSON that happens to carry formatVersion+config keys --
    // reject anything not stamped by us before looking at versions.
    if root.get("appName").and_then(|v| v.as_str()) != Some(APP_NAME) {
        return Err("Not a valid FT8AF settings backup.".to_string());
    }
    let format_version = root.get("formatVersion").and_then(|v| v.as_i64()).unwrap_or(-1);
    if format_version <= 0 {
        return Err("Not a valid FT8AF settings backup.".to_string());
    }
    if format_version > FORMAT_VERSION {
        return Err(format!(
            "This backup was created by a newer version of FT8AF (format {format_version}). \
             Please update the app to import it."
        ));
    }
    let config_obj = root
        .get("config")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "Backup contains no settings to restore.".to_string())?;
    let mut config = BTreeMap::new();
    for (k, v) in config_obj {
        config.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
    }
    if config.is_empty() {
        return Err("Backup contains no settings to restore.".to_string());
    }
    // Trim + drop blanks only -- no case change, matching Android's own
    // `parseBackupJson` exactly (`trim().takeIf { isNotEmpty() }`).
    // Normalization to uppercase happens later, at `add_follow_callsign`
    // apply time, same as Android's own insert path.
    let follow_callsigns = root
        .get("followCallsigns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(ParsedBackup {
        format_version,
        app_version: root.get("appVersion").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        created_at: root.get("createdAt").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        config,
        follow_callsigns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Vec<(String, String)> {
        vec![
            ("my_call".to_string(), "AI5II".to_string()),
            ("my_grid".to_string(), "EM71".to_string()),
            ("qrzApiKey".to_string(), "secret123".to_string()),
        ]
    }

    #[test]
    fn filter_config_excludes_sensitive_keys_by_default() {
        let filtered = filter_config(&sample_config(), false);
        assert!(!filtered.contains_key("qrzApiKey"));
        assert_eq!(filtered.get("my_call"), Some(&"AI5II".to_string()));
    }

    #[test]
    fn filter_config_includes_sensitive_keys_when_opted_in() {
        let filtered = filter_config(&sample_config(), true);
        assert_eq!(filtered.get("qrzApiKey"), Some(&"secret123".to_string()));
    }

    #[test]
    fn build_and_parse_round_trip() {
        let json = build_backup_json(
            &sample_config(),
            false,
            "0.1.2",
            "2026-11-14 10:00",
            vec!["W1AW".to_string(), "K1ABC".to_string()],
        );
        let parsed = parse_backup_json(&json).unwrap();
        assert_eq!(parsed.format_version, FORMAT_VERSION);
        assert_eq!(parsed.app_version, "0.1.2");
        assert_eq!(parsed.created_at, "2026-11-14 10:00");
        assert_eq!(parsed.config.get("my_call"), Some(&"AI5II".to_string()));
        assert!(!parsed.config.contains_key("qrzApiKey"), "excluded by default");
        assert_eq!(parsed.follow_callsigns, vec!["K1ABC".to_string(), "W1AW".to_string()]);
    }

    #[test]
    fn parse_preserves_case_exactly_as_written_no_normalization() {
        // Matches Android's own parseBackupJson: only trim + drop blanks, no
        // case change -- normalization to uppercase happens later, at
        // add_follow_callsign apply time (same as Android's insert path).
        let parsed = parse_backup_json(
            r#"{"appName":"FT8AF","formatVersion":1,"config":{"a":"b"},"followCallsigns":[" w1aw ","K1abc"]}"#,
        )
        .unwrap();
        assert_eq!(parsed.follow_callsigns, vec!["w1aw".to_string(), "K1abc".to_string()]);
    }

    #[test]
    fn build_includes_sensitive_when_requested() {
        let json = build_backup_json(&sample_config(), true, "0.1.2", "now", vec![]);
        let parsed = parse_backup_json(&json).unwrap();
        assert_eq!(parsed.config.get("qrzApiKey"), Some(&"secret123".to_string()));
    }

    #[test]
    fn parse_rejects_invalid_json() {
        assert!(parse_backup_json("not json").is_err());
    }

    #[test]
    fn parse_rejects_json_missing_the_app_name_marker() {
        let err = parse_backup_json(r#"{"formatVersion":1,"config":{"a":"b"}}"#).unwrap_err();
        assert!(err.contains("Not a valid FT8AF"));
    }

    #[test]
    fn parse_rejects_an_empty_config() {
        let err = parse_backup_json(r#"{"appName":"FT8AF","formatVersion":1,"config":{}}"#).unwrap_err();
        assert!(err.contains("no settings to restore"));
    }

    #[test]
    fn parse_rejects_a_newer_format_version() {
        let err = parse_backup_json(
            r#"{"appName":"FT8AF","formatVersion":99,"config":{"a":"b"}}"#,
        )
        .unwrap_err();
        assert!(err.contains("newer version of FT8AF"));
    }

    #[test]
    fn parse_tolerates_a_missing_follow_callsigns_array() {
        let parsed = parse_backup_json(
            r#"{"appName":"FT8AF","formatVersion":1,"config":{"a":"b"}}"#,
        )
        .unwrap();
        assert!(parsed.follow_callsigns.is_empty());
    }

    #[test]
    fn parse_ignores_an_unknown_prefs_section_rather_than_failing() {
        // Desktop never writes "prefs" (see module doc comment), but a real
        // Android export carries one -- importing it on desktop must not
        // error out just because that section exists.
        let parsed = parse_backup_json(
            r#"{"appName":"FT8AF","formatVersion":1,"config":{"a":"b"},"prefs":{"ft8af_theme":{"x":"y"}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.config.get("a"), Some(&"b".to_string()));
    }
}
