//! FT8 band plan: standard dial frequencies and band-name lookup, plus
//! user-defined "custom" dial frequencies (issue #470) for chasing DXpeditions
//! on off-plan frequencies. Custom bands are persisted as JSON in the config
//! store and merged into the picker alongside the built-in [`FT8_BANDS`].
//!
//! Also the FT4 band plan ([`FT4_BANDS`]) -- a separate, smaller dial list,
//! mirroring Android's `bands.txt` marked (`*`) FT4 entries exactly. Not
//! every FT8 band has an FT4 allocation (160m and 60m don't), which is why
//! [`mode_band_freq`] returns `Option` and callers must leave the dial
//! untouched on `None` rather than guessing -- see `Engine::switch_mode`'s
//! doc comment for the radio-safety reasoning (ported from
//! `MainViewModel.setOperatingMode`).

use crate::mode::Mode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct Band {
    pub name: &'static str,
    /// Standard FT8 dial (USB suppressed-carrier) frequency in Hz.
    pub dial_hz: u64,
    /// Inclusive band edges (Hz) for naming an arbitrary frequency.
    pub lo_hz: u64,
    pub hi_hz: u64,
}

/// A user-defined dial frequency (issue #470). Persisted as a JSON array under
/// the `custom_bands` config key and merged into the band picker so operators
/// can tune to published DXpedition/off-plan dials that aren't in the band plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomBand {
    /// Optional operator label (e.g. "3B9 DX"). Falls back to the band name.
    pub name: String,
    pub dial_hz: u64,
}

/// A band entry as sent to the UI picker: built-in dials and custom dials share
/// one flat list, `custom` flagging the user-defined ones so the UI can mark
/// them (and offer delete).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BandEntry {
    pub name: String,
    pub dial_hz: u64,
    pub custom: bool,
}

/// Config key under which the custom-band JSON list is stored (see `db.rs`).
pub const CUSTOM_BANDS_KEY: &str = "custom_bands";

/// Accepted dial-frequency range for validation, in Hz. The lower bound keeps
/// out obvious typos; the upper bound comfortably covers the microwave dials in
/// the band plan (QO-100 3cm ~10.489 GHz).
pub const MIN_DIAL_HZ: u64 = 1_000;
pub const MAX_DIAL_HZ: u64 = 30_000_000_000;

pub const FT8_BANDS: &[Band] = &[
    Band { name: "160m", dial_hz: 1_840_000, lo_hz: 1_800_000, hi_hz: 2_000_000 },
    Band { name: "80m", dial_hz: 3_573_000, lo_hz: 3_500_000, hi_hz: 4_000_000 },
    Band { name: "60m", dial_hz: 5_357_000, lo_hz: 5_300_000, hi_hz: 5_410_000 },
    Band { name: "40m", dial_hz: 7_074_000, lo_hz: 7_000_000, hi_hz: 7_300_000 },
    Band { name: "30m", dial_hz: 10_136_000, lo_hz: 10_100_000, hi_hz: 10_150_000 },
    Band { name: "20m", dial_hz: 14_074_000, lo_hz: 14_000_000, hi_hz: 14_350_000 },
    Band { name: "17m", dial_hz: 18_100_000, lo_hz: 18_068_000, hi_hz: 18_168_000 },
    Band { name: "15m", dial_hz: 21_074_000, lo_hz: 21_000_000, hi_hz: 21_450_000 },
    Band { name: "12m", dial_hz: 24_915_000, lo_hz: 24_890_000, hi_hz: 24_990_000 },
    Band { name: "10m", dial_hz: 28_074_000, lo_hz: 28_000_000, hi_hz: 29_700_000 },
    Band { name: "6m", dial_hz: 50_313_000, lo_hz: 50_000_000, hi_hz: 54_000_000 },
    Band { name: "2m", dial_hz: 144_174_000, lo_hz: 144_000_000, hi_hz: 148_000_000 },
];

/// ADIF band name for a dial frequency in Hz (e.g. 14_074_000 -> "20m").
pub fn band_for_freq_hz(hz: u64) -> &'static str {
    for b in FT8_BANDS {
        if hz >= b.lo_hz && hz <= b.hi_hz {
            return b.name;
        }
    }
    "unknown"
}

/// FT4 dial frequencies -- mirrors Android's `bands.txt` marked (`*`) `:FT4`
/// entries exactly. No entry for 160m or 60m: neither has a conventional FT4
/// allocation, matching Android's own list (and its documented radio-safety
/// behavior of leaving the dial untouched rather than guessing one).
pub const FT4_BANDS: &[Band] = &[
    Band { name: "80m", dial_hz: 3_575_000, lo_hz: 3_500_000, hi_hz: 4_000_000 },
    Band { name: "40m", dial_hz: 7_047_500, lo_hz: 7_000_000, hi_hz: 7_300_000 },
    Band { name: "30m", dial_hz: 10_140_000, lo_hz: 10_100_000, hi_hz: 10_150_000 },
    Band { name: "20m", dial_hz: 14_080_000, lo_hz: 14_000_000, hi_hz: 14_350_000 },
    Band { name: "17m", dial_hz: 18_104_000, lo_hz: 18_068_000, hi_hz: 18_168_000 },
    Band { name: "15m", dial_hz: 21_140_000, lo_hz: 21_000_000, hi_hz: 21_450_000 },
    Band { name: "12m", dial_hz: 24_919_000, lo_hz: 24_890_000, hi_hz: 24_990_000 },
    Band { name: "10m", dial_hz: 28_180_000, lo_hz: 28_000_000, hi_hz: 29_700_000 },
    Band { name: "6m", dial_hz: 50_318_000, lo_hz: 50_000_000, hi_hz: 54_000_000 },
    Band { name: "2m", dial_hz: 144_170_000, lo_hz: 144_000_000, hi_hz: 148_000_000 },
];

/// FT2 dial frequencies -- mirrors Android's `bands.txt` `:FT2` entries,
/// restricted to the bands desktop already has edge definitions (and rig/
/// antenna support) for -- the same 12 names as `FT8_BANDS`. Android's FT2
/// list also covers 1.25m/70cm/23cm/13cm/3cm and the QO-100 satellite dials;
/// those are skipped here (no band-edge or transverter/satellite support
/// anywhere else in this codebase, FT8 included). Unlike FT4, FT2 *does*
/// have 160m and 60m entries in Android's table, so both are included.
pub const FT2_BANDS: &[Band] = &[
    Band { name: "160m", dial_hz: 1_843_000, lo_hz: 1_800_000, hi_hz: 2_000_000 },
    Band { name: "80m", dial_hz: 3_578_000, lo_hz: 3_500_000, hi_hz: 4_000_000 },
    Band { name: "60m", dial_hz: 5_360_000, lo_hz: 5_300_000, hi_hz: 5_410_000 },
    Band { name: "40m", dial_hz: 7_062_000, lo_hz: 7_000_000, hi_hz: 7_300_000 },
    Band { name: "30m", dial_hz: 10_144_000, lo_hz: 10_100_000, hi_hz: 10_150_000 },
    Band { name: "20m", dial_hz: 14_084_000, lo_hz: 14_000_000, hi_hz: 14_350_000 },
    Band { name: "17m", dial_hz: 18_108_000, lo_hz: 18_068_000, hi_hz: 18_168_000 },
    Band { name: "15m", dial_hz: 21_144_000, lo_hz: 21_000_000, hi_hz: 21_450_000 },
    Band { name: "12m", dial_hz: 24_923_000, lo_hz: 24_890_000, hi_hz: 24_990_000 },
    Band { name: "10m", dial_hz: 28_184_000, lo_hz: 28_000_000, hi_hz: 29_700_000 },
    Band { name: "6m", dial_hz: 50_316_000, lo_hz: 50_000_000, hi_hz: 54_000_000 },
    Band { name: "2m", dial_hz: 144_177_000, lo_hz: 144_000_000, hi_hz: 148_000_000 },
];

/// The band table for a mode -- `FT8_BANDS`, `FT4_BANDS`, or `FT2_BANDS`.
pub fn bands_for_mode(mode: Mode) -> &'static [Band] {
    match mode {
        Mode::Ft8 => FT8_BANDS,
        Mode::Ft4 => FT4_BANDS,
        Mode::Ft2 => FT2_BANDS,
    }
}

/// The dial frequency for a given ADIF band name (e.g. "40m") in a given
/// mode, or `None` if that band has no entry in that mode -- mirrors
/// Android's `OperationBand.getModeBandFreq`. Used to retune within the same
/// band on a mode switch (`Engine::switch_mode`): the band itself never
/// changes, only the in-band dial, and a missing entry means "leave the
/// frequency exactly as it is" (never jump bands to find one).
pub fn mode_band_freq(wave_length: &str, mode: Mode) -> Option<u64> {
    bands_for_mode(mode)
        .iter()
        .find(|b| b.name == wave_length)
        .map(|b| b.dial_hz)
}

/// Band entries for the picker/sheet in a given mode. FT4 has no custom-dial
/// concept on desktop (customs are plain FT8 dials, like the existing
/// built-in/custom split), so only FT8 merges `custom` in.
pub fn bands_for_mode_entries(mode: Mode, custom: &[CustomBand]) -> Vec<BandEntry> {
    match mode {
        Mode::Ft8 => merged_bands(custom),
        Mode::Ft4 => FT4_BANDS
            .iter()
            .map(|b| BandEntry { name: b.name.to_string(), dial_hz: b.dial_hz, custom: false })
            .collect(),
        Mode::Ft2 => FT2_BANDS
            .iter()
            .map(|b| BandEntry { name: b.name.to_string(), dial_hz: b.dial_hz, custom: false })
            .collect(),
    }
}

/// Frequency in MHz as an ADIF-style string, e.g. 14_074_000 -> "14.074000".
pub fn freq_mhz_string(hz: u64) -> String {
    format!("{:.6}", hz as f64 / 1_000_000.0)
}

pub fn default_band() -> &'static Band {
    // 20m — the most active FT8 band.
    &FT8_BANDS[5]
}

// --- custom (user-defined) dial frequencies (issue #470) --------------------

/// Parse an operator-entered dial frequency (whole Hz) and validate its range,
/// returning a human-readable message on bad input so the UI can surface it.
pub fn parse_dial_hz(input: &str) -> Result<u64, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err("Enter a dial frequency in Hz.".to_string());
    }
    let hz: u64 = t
        .parse()
        .map_err(|_| format!("\"{t}\" is not a whole number of Hz."))?;
    if hz < MIN_DIAL_HZ || hz > MAX_DIAL_HZ {
        return Err(format!(
            "{hz} Hz is out of range ({MIN_DIAL_HZ}–{MAX_DIAL_HZ} Hz)."
        ));
    }
    Ok(hz)
}

/// Deserialize the persisted custom-band list. An empty/malformed string (older
/// build, hand-edited DB) yields an empty list rather than an error, so a bad
/// value can never wipe out the built-in bands.
pub fn parse_custom_bands(json: &str) -> Vec<CustomBand> {
    if json.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(json).unwrap_or_default()
}

/// Serialize the custom-band list for persistence.
pub fn serialize_custom_bands(bands: &[CustomBand]) -> String {
    serde_json::to_string(bands).unwrap_or_else(|_| "[]".to_string())
}

/// Add a custom band, or rename the existing one at the same dial. The label is
/// trimmed; a blank label falls back to the ADIF band name for that frequency
/// (e.g. "20m") so every entry shows something meaningful.
pub fn upsert_custom_band(bands: &mut Vec<CustomBand>, name: &str, dial_hz: u64) {
    let trimmed = name.trim();
    let label = if trimmed.is_empty() {
        band_for_freq_hz(dial_hz).to_string()
    } else {
        trimmed.to_string()
    };
    if let Some(existing) = bands.iter_mut().find(|b| b.dial_hz == dial_hz) {
        existing.name = label;
    } else {
        bands.push(CustomBand { name: label, dial_hz });
    }
}

/// Remove the custom band at `dial_hz`. Returns true if one was removed.
pub fn remove_custom_band(bands: &mut Vec<CustomBand>, dial_hz: u64) -> bool {
    let before = bands.len();
    bands.retain(|b| b.dial_hz != dial_hz);
    bands.len() != before
}

/// The flat picker list: every built-in dial followed by the custom dials, with
/// custom entries de-duplicated against built-ins by dial frequency so a custom
/// entry that duplicates a stock dial doesn't appear twice.
pub fn merged_bands(custom: &[CustomBand]) -> Vec<BandEntry> {
    let mut out: Vec<BandEntry> = FT8_BANDS
        .iter()
        .map(|b| BandEntry {
            name: b.name.to_string(),
            dial_hz: b.dial_hz,
            custom: false,
        })
        .collect();
    for c in custom {
        if out.iter().any(|b| b.dial_hz == c.dial_hz) {
            continue;
        }
        out.push(BandEntry {
            name: c.name.clone(),
            dial_hz: c.dial_hz,
            custom: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dial_hz_accepts_in_range() {
        assert_eq!(parse_dial_hz("14074000"), Ok(14_074_000));
        assert_eq!(parse_dial_hz("  7074000 "), Ok(7_074_000));
        // Microwave dial from the band plan is still in range.
        assert_eq!(parse_dial_hz("10489540000"), Ok(10_489_540_000));
    }

    #[test]
    fn parse_dial_hz_rejects_bad_input() {
        assert!(parse_dial_hz("").is_err());
        assert!(parse_dial_hz("abc").is_err());
        assert!(parse_dial_hz("14.074").is_err()); // not whole Hz
        assert!(parse_dial_hz("-5").is_err());
        assert!(parse_dial_hz("100").is_err()); // below MIN_DIAL_HZ
        assert!(parse_dial_hz("40000000000").is_err()); // above MAX_DIAL_HZ
    }

    #[test]
    fn custom_bands_round_trip() {
        let bands = vec![
            CustomBand { name: "3B9 DX".into(), dial_hz: 14_090_000 },
            CustomBand { name: "40m DX".into(), dial_hz: 7_090_000 },
        ];
        let json = serialize_custom_bands(&bands);
        assert_eq!(parse_custom_bands(&json), bands);
    }

    #[test]
    fn parse_custom_bands_tolerates_junk() {
        assert!(parse_custom_bands("").is_empty());
        assert!(parse_custom_bands("   ").is_empty());
        assert!(parse_custom_bands("not json").is_empty());
    }

    #[test]
    fn upsert_adds_then_renames_same_dial() {
        let mut bands = Vec::new();
        upsert_custom_band(&mut bands, "DX", 14_090_000);
        assert_eq!(bands.len(), 1);
        // Same dial -> rename in place, not a duplicate.
        upsert_custom_band(&mut bands, "3B9 DX", 14_090_000);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].name, "3B9 DX");
    }

    #[test]
    fn upsert_blank_label_falls_back_to_band_name() {
        let mut bands = Vec::new();
        upsert_custom_band(&mut bands, "   ", 14_090_000);
        assert_eq!(bands[0].name, "20m");
    }

    #[test]
    fn remove_custom_band_reports_hit_and_miss() {
        let mut bands = vec![CustomBand { name: "DX".into(), dial_hz: 14_090_000 }];
        assert!(!remove_custom_band(&mut bands, 7_000_000));
        assert!(remove_custom_band(&mut bands, 14_090_000));
        assert!(bands.is_empty());
    }

    #[test]
    fn merged_bands_appends_custom_and_flags_them() {
        let custom = vec![CustomBand { name: "3B9 DX".into(), dial_hz: 14_090_000 }];
        let merged = merged_bands(&custom);
        assert_eq!(merged.len(), FT8_BANDS.len() + 1);
        let last = merged.last().unwrap();
        assert_eq!(last.dial_hz, 14_090_000);
        assert_eq!(last.name, "3B9 DX");
        assert!(last.custom);
        // Built-ins keep their order and are not flagged custom.
        assert_eq!(merged[0].name, "160m");
        assert!(!merged[0].custom);
    }

    #[test]
    fn merged_bands_dedupes_custom_matching_builtin_dial() {
        // A custom entry on a stock dial (20m = 14.074) must not double the row.
        let custom = vec![CustomBand { name: "dup".into(), dial_hz: 14_074_000 }];
        let merged = merged_bands(&custom);
        assert_eq!(merged.len(), FT8_BANDS.len());
        assert_eq!(merged.iter().filter(|b| b.dial_hz == 14_074_000).count(), 1);
    }

    #[test]
    fn mode_band_freq_matches_known_android_dials() {
        assert_eq!(mode_band_freq("40m", Mode::Ft4), Some(7_047_500));
        assert_eq!(mode_band_freq("20m", Mode::Ft4), Some(14_080_000));
        assert_eq!(mode_band_freq("40m", Mode::Ft8), Some(7_074_000));
        assert_eq!(mode_band_freq("20m", Mode::Ft8), Some(14_074_000));
    }

    #[test]
    fn mode_band_freq_is_none_where_android_has_no_ft4_allocation() {
        // 160m and 60m have no FT4 dial in Android's bands.txt either --
        // callers must leave the frequency untouched on a mode switch, not
        // fall back to some other band's dial.
        assert_eq!(mode_band_freq("160m", Mode::Ft4), None);
        assert_eq!(mode_band_freq("60m", Mode::Ft4), None);
        // Nonsense band name: None in either mode.
        assert_eq!(mode_band_freq("99m", Mode::Ft8), None);
        assert_eq!(mode_band_freq("99m", Mode::Ft4), None);
    }

    #[test]
    fn ft4_bands_has_no_60m_or_160m_entry() {
        assert!(!FT4_BANDS.iter().any(|b| b.name == "160m"));
        assert!(!FT4_BANDS.iter().any(|b| b.name == "60m"));
        assert_eq!(FT4_BANDS.len(), 10);
    }

    #[test]
    fn bands_for_mode_entries_ft4_has_no_customs_merged() {
        let custom = vec![CustomBand { name: "dup".into(), dial_hz: 14_080_000 }];
        let entries = bands_for_mode_entries(Mode::Ft4, &custom);
        // FT4's 20m dial (14.080) coincides with the custom entry above, but
        // FT4 doesn't merge customs at all -- exactly FT4_BANDS.len() rows,
        // none flagged custom.
        assert_eq!(entries.len(), FT4_BANDS.len());
        assert!(entries.iter().all(|b| !b.custom));
    }

    #[test]
    fn ft2_bands_has_160m_and_60m_unlike_ft4() {
        // Android's bands.txt gives FT2 a 160m and 60m dial where FT4 has
        // neither -- confirm desktop's table kept that asymmetry rather than
        // copying FT4's band-name set.
        assert!(FT2_BANDS.iter().any(|b| b.name == "160m"));
        assert!(FT2_BANDS.iter().any(|b| b.name == "60m"));
        assert_eq!(FT2_BANDS.len(), 12);
    }

    #[test]
    fn mode_band_freq_matches_known_android_ft2_dials() {
        assert_eq!(mode_band_freq("40m", Mode::Ft2), Some(7_062_000));
        assert_eq!(mode_band_freq("20m", Mode::Ft2), Some(14_084_000));
        assert_eq!(mode_band_freq("160m", Mode::Ft2), Some(1_843_000));
        assert_eq!(mode_band_freq("60m", Mode::Ft2), Some(5_360_000));
        // Not in desktop's restricted FT2 table (no band-edge/transverter
        // support anywhere else in the codebase) -- None, not a guess.
        assert_eq!(mode_band_freq("70cm", Mode::Ft2), None);
    }

    #[test]
    fn bands_for_mode_entries_ft2_has_no_customs_merged() {
        let custom = vec![CustomBand { name: "dup".into(), dial_hz: 14_084_000 }];
        let entries = bands_for_mode_entries(Mode::Ft2, &custom);
        assert_eq!(entries.len(), FT2_BANDS.len());
        assert!(entries.iter().all(|b| !b.custom));
    }
}
