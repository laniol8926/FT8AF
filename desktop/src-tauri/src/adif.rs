//! ADIF parsing for log import. Ports the reader half of `db.rs`'s hand-rolled
//! ADIF writer: a bare tag-length-value scanner, no external crate.
//!
//! ADIF fields look like `<name:length>value` (optionally `<name:length:type>`,
//! the type suffix is ignored — we only need the value). `<eoh>` ends the
//! header (any tags before it, e.g. `<PROGRAMID:5>FT8AF`, are header metadata,
//! not a QSO, and are discarded); `<eor>` ends one QSO record. Both are
//! case-insensitive and carry no length.

use std::collections::HashMap;

use crate::db::QsoRecord;

/// One parsed ADIF record: field name (upper-cased) -> raw value, unmodified.
#[derive(Debug, Clone, Default)]
pub struct AdifRecord {
    pub fields: HashMap<String, String>,
}

/// Scan ADIF text into records. Tolerant of a missing/odd header, Windows line
/// endings, and stray whitespace between fields — none of that is part of any
/// tag's declared length, so it's skipped automatically by the scan.
pub fn parse_adif(text: &str) -> Vec<AdifRecord> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut records = Vec::new();
    let mut current = HashMap::new();

    while i < len {
        let Some(off) = bytes[i..].iter().position(|&b| b == b'<') else { break };
        i += off;
        let tag_start = i + 1;
        let Some(end_off) = bytes[tag_start..].iter().position(|&b| b == b'>') else { break };
        let tag_end = tag_start + end_off;
        // Tags are ASCII in practice; lossy-decode defensively (byte slices
        // only, never `&text[..]`) so a stray byte here can't panic the parser.
        let tag = String::from_utf8_lossy(&bytes[tag_start..tag_end]).into_owned();
        i = tag_end + 1;

        let lower = tag.to_ascii_lowercase();
        if lower == "eoh" {
            // Discard header-only tags seen before this point (ADIF_VER, PROGRAMID, …).
            current.clear();
            continue;
        }
        if lower == "eor" {
            if !current.is_empty() {
                records.push(AdifRecord { fields: std::mem::take(&mut current) });
            }
            continue;
        }

        let mut parts = tag.split(':');
        let Some(name) = parts.next() else { continue };
        // A field with no `:length` (a bare boolean-style tag) carries no value
        // we can extract positionally; skip it rather than guess.
        let Some(len_str) = parts.next() else { continue };
        let Ok(field_len) = len_str.trim().parse::<usize>() else { continue };
        let end = (i + field_len).min(len);
        // The declared length is a byte count. A real-world export (seen from
        // CQRLOG/QRZ.com with accented names or comments) can disagree with
        // UTF-8's own byte accounting, so `end` can land mid-character. A bare
        // `&text[i..end]` slice panics in that case ("byte index N is not a
        // char boundary") — and because this parser runs inside a Tauri
        // command invoked from webkit2gtk's C callback, that panic can't
        // unwind and aborts the whole running app instead of just failing the
        // import. `from_utf8_lossy` on the raw bytes degrades gracefully
        // (inserting U+FFFD) instead.
        let value = String::from_utf8_lossy(&bytes[i..end]).into_owned();
        i = end;
        current.insert(name.to_ascii_uppercase(), value);
    }

    records
}

/// Map one parsed record to a [`QsoRecord`], or `None` if it has no `CALL`
/// (not a usable QSO). Callsigns and grids are upper-cased and the band is
/// lower-cased to match how the app stores its own QSOs (see `logbook.ts`'s
/// `buildQsoRecord` and `db.rs`'s own export, e.g. `<band:3>20m`), so an
/// imported row sorts/filters the same as one logged in-app.
///
/// QSL/upload flags map by ADIF field name ("Y", case-insensitively, is true;
/// anything else, including absent, is false): `QSL_RCVD`→`confirmed` (paper
/// received), `QSL_SENT`→`qsl_sent` (paper sent), `LOTW_QSL_SENT`/`_RCVD`,
/// `EQSL_QSL_SENT`/`_RCVD`. There's no standard ADIF field for a QRZ.com
/// logbook confirmation; `qrz_qsl` is true from either of two app-specific
/// extension fields: `APP_QRZLOG_STATUS=C` (what a genuine QRZ.com logbook
/// export carries) or `APP_FT8AF_QRZ_CONFIRMED=Y` (what `db::adif_qso_body`
/// writes on export, so re-importing this app's own file round-trips the
/// flag without a QRZ.com export in hand). `STATE`/`COUNTRY` are copied through
/// verbatim (trimmed, not
/// case-changed — `STATE` can be a short code like "CA" or a spelled-out
/// non-US subdivision, and reshaping either risks mismatching what
/// [`Db::import_adif_with_progress`]'s "fill only when blank" merge expects
/// to compare against a user's own typed value later).
pub fn adif_record_to_qso(fields: &HashMap<String, String>) -> Option<QsoRecord> {
    let call = fields.get("CALL")?.trim();
    if call.is_empty() {
        return None;
    }
    let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let flag = |k: &str| fields.get(k).is_some_and(|v| v.trim().eq_ignore_ascii_case("Y"));

    Some(QsoRecord {
        id: None,
        call: call.to_ascii_uppercase(),
        gridsquare: get("GRIDSQUARE").to_ascii_uppercase(),
        mode: {
            // SUBMODE, when present, is always the more specific of the two
            // by ADIF's own definition (MODE is just the family it belongs
            // to) -- confirmed from a real CQRLOG export, which tags FT4/JS8
            // contacts <MODE:4>MFSK<SUBMODE:3>FT4/JS8. Preferring it here
            // means a fresh import already stores the specific mode instead
            // of relying on `merge_import_flags`'s upgrade-on-duplicate path
            // to fix it up later.
            let m = get("SUBMODE");
            let m = if m.is_empty() { get("MODE") } else { m };
            if m.is_empty() { "FT8".to_string() } else { m.to_ascii_uppercase() }
        },
        rst_sent: get("RST_SENT"),
        rst_rcvd: get("RST_RCVD"),
        qso_date: get("QSO_DATE"),
        time_on: get("TIME_ON"),
        qso_date_off: get("QSO_DATE_OFF"),
        time_off: get("TIME_OFF"),
        band: get("BAND").to_ascii_lowercase(),
        freq: get("FREQ"),
        station_callsign: get("STATION_CALLSIGN").to_ascii_uppercase(),
        my_gridsquare: get("MY_GRIDSQUARE").to_ascii_uppercase(),
        comment: get("COMMENT"),
        confirmed: flag("QSL_RCVD"),
        qsl_sent: flag("QSL_SENT"),
        lotw_sent: flag("LOTW_QSL_SENT"),
        lotw_rcvd: flag("LOTW_QSL_RCVD"),
        eqsl_sent: flag("EQSL_QSL_SENT"),
        eqsl_rcvd: flag("EQSL_QSL_RCVD"),
        qrz_qsl: flag("APP_FT8AF_QRZ_CONFIRMED")
            || fields.get("APP_QRZLOG_STATUS").is_some_and(|v| v.trim().eq_ignore_ascii_case("C")),
        state: get("STATE"),
        country: get("COUNTRY"),
        synced_cloudlog: false,
        synced_qrz: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_two_records() {
        let text = "FT8AF ADIF Export<eoh>\n\
            <call:5>K1ABC <gridsquare:4>FN42 <mode:3>FT8 <qso_date:8>20260604 \
            <time_on:6>123045 <band:3>20m <QSL_RCVD:1>Y <eor>\n\
            <call:4>W1AW <mode:3>FT8 <qso_date:8>20260604 <time_on:6>130000 <eor>\n";
        let recs = parse_adif(text);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "K1ABC");
        assert_eq!(recs[0].fields.get("GRIDSQUARE").unwrap(), "FN42");
        assert_eq!(recs[1].fields.get("CALL").unwrap(), "W1AW");
        assert!(!recs[1].fields.contains_key("GRIDSQUARE"));
    }

    #[test]
    fn header_tags_before_eoh_are_not_a_phantom_record() {
        let text = "<ADIF_VER:5>3.1.4<PROGRAMID:5>FT8AF<EOH>\n<call:4>W1AW <qso_date:8>20260604 <time_on:6>130000 <eor>";
        let recs = parse_adif(text);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "W1AW");
        assert!(!recs[0].fields.contains_key("ADIF_VER"));
        assert!(!recs[0].fields.contains_key("PROGRAMID"));
    }

    #[test]
    fn tag_names_and_eor_eoh_are_case_insensitive() {
        let text = "<Call:5>K1ABC <MODE:3>FT8 <Qso_Date:8>20260604 <TIME_ON:6>120000 <EOR>";
        let recs = parse_adif(text);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "K1ABC");
        assert_eq!(recs[0].fields.get("MODE").unwrap(), "FT8");
    }

    #[test]
    fn a_record_with_no_call_is_dropped() {
        let fields: HashMap<String, String> =
            [("QSO_DATE".to_string(), "20260604".to_string())].into();
        assert!(adif_record_to_qso(&fields).is_none());
    }

    #[test]
    fn maps_fields_normalizing_case() {
        let text = "<call:5>k1abc <gridsquare:4>fn42 <band:3>20M <mode:3>ft8 \
            <qso_date:8>20260604 <time_on:6>123045 <QSL_RCVD:1>y <eor>";
        let rec = &parse_adif(text)[0];
        let qso = adif_record_to_qso(&rec.fields).unwrap();
        assert_eq!(qso.call, "K1ABC");
        assert_eq!(qso.gridsquare, "FN42");
        assert_eq!(qso.band, "20m");
        assert_eq!(qso.mode, "FT8");
        assert!(qso.confirmed);
    }

    #[test]
    fn submode_wins_over_the_generic_mode_family() {
        // Real CQRLOG export shape: <MODE:4>MFSK<SUBMODE:3>FT4.
        let text = "<call:5>K1ABC <mode:4>MFSK <submode:3>FT4 \
            <qso_date:8>20260604 <time_on:6>123045 <eor>";
        let qso = adif_record_to_qso(&parse_adif(text)[0].fields).unwrap();
        assert_eq!(qso.mode, "FT4");
    }

    #[test]
    fn mode_is_used_when_there_is_no_submode() {
        let text = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 <eor>";
        let qso = adif_record_to_qso(&parse_adif(text)[0].fields).unwrap();
        assert_eq!(qso.mode, "FT8");
    }

    #[test]
    fn missing_mode_defaults_to_ft8_and_missing_qsl_is_unconfirmed() {
        let text = "<call:4>W1AW <qso_date:8>20260604 <time_on:6>120000 <eor>";
        let rec = &parse_adif(text)[0];
        let qso = adif_record_to_qso(&rec.fields).unwrap();
        assert_eq!(qso.mode, "FT8");
        assert!(!qso.confirmed);
    }

    #[test]
    fn a_declared_length_that_splits_a_multibyte_char_does_not_panic() {
        // "café" is 4 Unicode scalars but 5 UTF-8 bytes (é = 0xC3 0xA9). Some
        // real exports (CQRLOG/QRZ.com comments and operator names with
        // accents) declare a tag's length in a way that doesn't line up with
        // UTF-8 byte boundaries once the file is decoded as text. A `&str`
        // slice at that byte offset panics ("byte index N is not a char
        // boundary") -- which crashed the whole app on a real import, since
        // this parser runs inside a Tauri command invoked from webkit2gtk's C
        // callback and a panic there can't unwind. It must degrade (lossily)
        // instead of panicking.
        let text = "<call:4>W1AW <comment:4>café<eor>";
        let recs = parse_adif(text);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "W1AW");
        assert!(recs[0].fields.contains_key("COMMENT"));
    }

    #[test]
    fn truncated_file_does_not_panic() {
        // Declared length overruns what's actually there.
        let text = "<call:50>K1ABC";
        let recs = parse_adif(text);
        // No <eor>, so nothing is flushed — the scan must simply stop, not panic.
        assert!(recs.is_empty());
    }

    #[test]
    fn bare_tag_with_no_length_is_skipped_not_crashed() {
        let text = "<eoh><some_boolean_flag><call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>120000 <eor>";
        let recs = parse_adif(text);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "K1ABC");
        assert!(!recs[0].fields.contains_key("SOME_BOOLEAN_FLAG"));
    }
}
