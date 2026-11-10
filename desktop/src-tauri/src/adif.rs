//! ADIF parsing for log import. Ports the reader half of `db.rs`'s hand-rolled
//! ADIF writer: a bare tag-length-value scanner, no external crate.
//!
//! ADIF fields look like `<name:length>value` (optionally `<name:length:type>`,
//! the type suffix is ignored — we only need the value). `<eoh>` ends the
//! header (any tags before it, e.g. `<PROGRAMID:5>FT8AF`, are header metadata,
//! not a QSO, and are discarded); `<eor>` ends one QSO record. Both are
//! case-insensitive and carry no length.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};

use crate::db::QsoRecord;

/// One parsed ADIF record: field name (upper-cased) -> raw value, unmodified.
#[derive(Debug, Clone, Default)]
pub struct AdifRecord {
    pub fields: HashMap<String, String>,
}

/// Scan ADIF text into records, all at once. A thin wrapper over
/// [`AdifStreamReader`] driven against an in-memory cursor — kept so
/// existing small-fixture tests/callers that already have a whole string in
/// hand don't need to touch a file or a `Read`, but this is not a second
/// implementation: there is exactly one tag/record scanner
/// (`AdifStreamReader::next_record`), so the in-memory and streaming paths
/// can never drift apart.
pub fn parse_adif(text: &str) -> Vec<AdifRecord> {
    let mut reader = AdifStreamReader::new(Cursor::new(text.as_bytes()));
    let mut records = Vec::new();
    // A malformed `Cursor<&[u8]>` read can't actually fail (`Read` on a byte
    // slice is infallible), so swallowing an error here just means "stop" --
    // matching the old loop's `break`-on-truncation behavior.
    while let Ok(Some(rec)) = reader.next_record() {
        records.push(rec);
    }
    records
}

/// Streams ADIF records out of a [`Read`] without ever buffering the whole
/// source in memory. A whole-file-then-parse approach (read into one
/// `String`, scan it, collect a `Vec<AdifRecord>`) needs several times the
/// raw file size in peak memory once the frontend's file read and the IPC
/// round-trip are counted too — confirmed on this board: an 82MB real ADIF
/// log plus an already-imported ~45,600-row log locked up a 3.8GB machine
/// under swap pressure. Ported from the same fix already shipped in the
/// Android fork (`com.k1af.ft8af.log.AdifStreamReader`, "Rewrite
/// ImportSharedLogs to stream ADIF records instead of buffering the whole
/// file") — adapted, not transliterated: this parser is a byte-offset,
/// declared-length scanner (not Android's delimiter/regex-split one), so
/// instead of searching for a closing tag across chunk boundaries, each step
/// just `ensure()`s however many more bytes it already knows it needs
/// (either to find the next `<`/`>`, or a field's own declared length) and
/// reads that far ahead. Peak memory is bounded by roughly one record's
/// worth of text, not the whole file: consumed bytes are drained out of the
/// buffer as soon as they're parsed.
///
/// Reuses the exact same tag loop `parse_adif` used to run inline (same
/// `<eoh>`-clears-current, `<eor>`-flushes-current, bad/bare-field-tag-is-
/// skipped-not-the-whole-record semantics) — there is nothing streaming-
/// specific about *what* gets parsed, only *how much* of the source has to
/// be in memory to parse it.
pub struct AdifStreamReader<R: Read> {
    reader: R,
    buf: Vec<u8>,
    eof: bool,
    chunk_bytes: usize,
}

impl<R: Read> AdifStreamReader<R> {
    pub fn new(reader: R) -> Self {
        Self::with_chunk_bytes(reader, 8192)
    }

    /// Visible for tests: a tiny chunk size exercises a tag (or a multi-byte
    /// field value) split across several reads, not just the common case of
    /// everything fitting in one.
    fn with_chunk_bytes(reader: R, chunk_bytes: usize) -> Self {
        Self { reader, buf: Vec::new(), eof: false, chunk_bytes: chunk_bytes.max(1) }
    }

    /// The next parsed record, or `None` at end of stream. Matches
    /// `parse_adif`'s own leniency exactly (ported from the same loop, not
    /// re-derived): a dangling record with no closing `<eor>` is dropped,
    /// same as a truncated field whose declared length overruns what's left
    /// degrades to whatever bytes are actually available rather than
    /// erroring.
    pub fn next_record(&mut self) -> io::Result<Option<AdifRecord>> {
        let mut current: HashMap<String, String> = HashMap::new();
        loop {
            let Some(lt) = self.find(b'<', 0)? else {
                return Ok(None);
            };
            let tag_start = lt + 1;
            let Some(gt) = self.find(b'>', tag_start)? else {
                self.drain_consumed(lt);
                return Ok(None);
            };
            // Tags are ASCII in practice; lossy-decode defensively so a
            // stray byte here can't panic the parser.
            let tag = String::from_utf8_lossy(&self.buf[tag_start..gt]).into_owned();
            let lower = tag.to_ascii_lowercase();
            if lower == "eoh" {
                current.clear();
                self.drain_consumed(gt + 1);
                continue;
            }
            if lower == "eor" {
                self.drain_consumed(gt + 1);
                if current.is_empty() {
                    continue;
                }
                return Ok(Some(AdifRecord { fields: current }));
            }

            let mut parts = tag.split(':');
            let name = parts.next();
            // A field with no `:length` (a bare boolean-style tag) carries no
            // value we can extract positionally; skip just this tag, same as
            // `parse_adif` did, rather than dropping the whole record.
            let field_len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            match (name, field_len) {
                (Some(name), Some(field_len)) => {
                    let value_start = gt + 1;
                    let avail = self.ensure(value_start + field_len)?;
                    let value_end = avail.min(value_start + field_len);
                    // See `parse_adif`'s own comment: the declared length is a
                    // byte count that can disagree with UTF-8's, so this can
                    // land mid-character. Lossy-decode degrades gracefully.
                    let value = String::from_utf8_lossy(&self.buf[value_start..value_end]).into_owned();
                    current.insert(name.to_ascii_uppercase(), value);
                    self.drain_consumed(value_end);
                }
                _ => self.drain_consumed(gt + 1),
            }
        }
    }

    /// Read more from the source in ~8KB chunks until `buf` holds at least
    /// `want` bytes or EOF is hit; returns however many bytes are actually
    /// available (`want`, unless EOF cut it short).
    fn ensure(&mut self, want: usize) -> io::Result<usize> {
        while self.buf.len() < want && !self.eof {
            let mut chunk = vec![0u8; self.chunk_bytes];
            let n = self.reader.read(&mut chunk)?;
            if n == 0 {
                self.eof = true;
            } else {
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }
        Ok(self.buf.len().min(want))
    }

    /// The index of `needle` at or after `from`, growing the buffer as
    /// needed; `None` once the source is exhausted with no match.
    fn find(&mut self, needle: u8, from: usize) -> io::Result<Option<usize>> {
        let mut scanned = from;
        loop {
            if let Some(off) = self.buf[scanned..].iter().position(|&b| b == needle) {
                return Ok(Some(scanned + off));
            }
            // Nothing found in what's buffered -- don't re-scan it next time
            // (quadratic on a record that spans many chunks).
            scanned = self.buf.len();
            if self.eof {
                return Ok(None);
            }
            self.ensure(self.buf.len() + 1)?;
        }
    }

    /// Drop fully-parsed bytes from the front of the buffer so it never
    /// holds more than roughly one record's worth of text.
    fn drain_consumed(&mut self, upto: usize) {
        self.buf.drain(0..upto);
    }
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

    // --- AdifStreamReader: chunk-boundary / streaming-specific cases -----
    // (the shared tag/record semantics -- header handling, case sensitivity,
    // malformed fields, UTF-8 lossy decode -- are already covered by the
    // `parse_adif` tests below, which exercise this same reader via the
    // in-memory cursor path; these add only what's specific to reading in
    // small pieces.)

    fn read_all(mut r: AdifStreamReader<Cursor<&[u8]>>) -> Vec<AdifRecord> {
        let mut out = Vec::new();
        while let Some(rec) = r.next_record().unwrap() {
            out.push(rec);
        }
        out
    }

    #[test]
    fn tiny_chunks_still_find_a_tag_split_across_many_reads() {
        let text = "<eoh><call:5>K1ABC<eor><call:5>K2DEF<eor>";
        let reader = AdifStreamReader::with_chunk_bytes(Cursor::new(text.as_bytes()), 1);
        let recs = read_all(reader);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].fields.get("CALL").unwrap(), "K1ABC");
        assert_eq!(recs[1].fields.get("CALL").unwrap(), "K2DEF");
    }

    #[test]
    fn tiny_chunks_still_decode_a_multibyte_value_split_across_reads() {
        let text = "<eoh><call:4>W1AW<comment:5>café<eor>"; // café = 5 UTF-8 bytes
        let reader = AdifStreamReader::with_chunk_bytes(Cursor::new(text.as_bytes()), 1);
        let recs = read_all(reader);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].fields.get("COMMENT").unwrap(), "café");
    }

    #[test]
    fn peak_buffer_stays_near_one_record_not_the_whole_source() {
        // 50,000 small records (~900KB of text, over 100x the default 8KB
        // read-chunk size) -- if next_record() failed to drain consumed
        // bytes, buf would grow to roughly the whole source's size instead
        // of staying near one chunk's worth.
        let mut text = String::from("<eoh>");
        for i in 0..50_000 {
            text.push_str(&format!("<call:5>K{i:05}<eor>"));
        }
        let mut reader = AdifStreamReader::new(Cursor::new(text.as_bytes()));
        let mut count = 0;
        while reader.next_record().unwrap().is_some() {
            count += 1;
            // Generous bound: one read-ahead chunk (8KB) plus one record's
            // worth of slack. The full source is ~900KB, so this only holds
            // if draining is actually working, not just coincidentally small.
            assert!(reader.buf.len() < 20_000, "buffer grew to {} bytes after {count} records", reader.buf.len());
        }
        assert_eq!(count, 50_000);
    }

    #[test]
    fn records_split_across_separate_next_record_calls_match_parse_adif() {
        // Cross-check: streaming one record at a time must agree exactly
        // with parsing the same text all at once.
        let text = "<eoh><call:5>K1ABC<gridsquare:4>FN42<eor><call:4>W1AW<eor>";
        let mut reader = AdifStreamReader::new(Cursor::new(text.as_bytes()));
        let mut streamed = Vec::new();
        while let Some(rec) = reader.next_record().unwrap() {
            streamed.push(rec.fields);
        }
        let whole: Vec<_> = parse_adif(text).into_iter().map(|r| r.fields).collect();
        assert_eq!(streamed, whole);
    }

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
