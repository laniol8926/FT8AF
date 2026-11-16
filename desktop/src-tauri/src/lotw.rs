//! LoTW (ARRL Logbook of the World): downloading confirmations for QSOs
//! already logged, and a new, desktop-only upload path via the TQSL CLI.
//! Ported from Android's `LotwClient.kt` (download side only -- Android has
//! no desktop-class signing tool to upload with).
//!
//! **Download**: `https://lotw.arrl.org/lotwuser/lotwreport.adi` returns an
//! ADIF-shaped report of the operator's own confirmed QSLs. `parse_report`
//! reuses `adif::parse_adif`'s tag/record scanner for the raw field
//! extraction (the two formats share the same `<tag:len>value` syntax), but
//! is otherwise a dedicated, lightweight reader -- NOT a reuse of
//! `db.rs`'s ADIF-*import* pipeline (`adif_record_to_qso`/twin-matching):
//! the real report has no `TIME_OFF` field at all, which this codebase's
//! `is_lotw_style_record` requires, and confirmation-matching is
//! UPDATE-only (a confirmation with no local match is silently dropped,
//! never inserted as a new QSO row) -- a fundamentally different operation
//! from importing a full log.
//!
//! **Upload**: TQSL (TrustedQSL) is the ARRL's own official signing tool --
//! shelling out to it is the only way to produce a validly signed LoTW
//! submission, since LoTW requires a local private-key signature with no
//! equivalent network API. Confirmed installed/configured on the reference
//! machine this was built against (`tqsl --version`, real certs under
//! `~/.tqsl/keys/`).
//!
//! **Caution**: `tqsl`'s exact non-zero exit code contract (auth/cert
//! failure vs. a partial batch failure) is undocumented in `tqsl --help`
//! and was never verified against a real run while building this module --
//! any non-zero exit is treated here as a hard failure (nothing gets marked
//! sent). Treat the first real upload as a live, supervised test, not
//! something to trust blind from this comment.
//!
//! Never log a built report URL anywhere -- it carries the plaintext
//! password in the query string (matches Android's own explicit comment).

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::adif;
use crate::db::{adif_record, Db};

pub const CONFIG_USERNAME: &str = "lotwUsername";
pub const CONFIG_PASSWORD: &str = "lotwPassword";
pub const CONFIG_LAST_QSL: &str = "lotwLastQsl";

const REPORT_URL: &str = "https://lotw.arrl.org/lotwuser/lotwreport.adi";

/// One confirmed QSL from a LoTW report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LotwQsl {
    pub call: String,
    pub qso_date: String,
    pub time_on: String,
    pub band: String,
    /// `SUBMODE` when present, else `MODE` -- uppercased. FT4 reports as
    /// `MODE=MFSK SUBMODE=FT4`; this is the value a locally-logged FT4 QSO's
    /// own `mode` field would actually hold.
    pub mode: String,
    /// The literal `MODE` field, uppercased, always (never the submode).
    pub raw_mode: String,
}

#[derive(Debug, Default, Serialize)]
pub struct LotwDownloadResult {
    pub in_report: usize,
    pub newly_confirmed: usize,
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct LotwUploadResult {
    pub attempted: usize,
    pub error: Option<String>,
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Builds the LoTW report URL. Never log the result -- see this module's
/// top doc comment.
pub fn build_report_url(username: &str, password: &str, since: Option<&str>) -> String {
    let mut url = format!(
        "{REPORT_URL}?login={}&password={}&qso_query=1&qso_qsl=yes&qso_qsldetail=no",
        urlencode(username),
        urlencode(password),
    );
    if let Some(s) = since.filter(|s| !s.is_empty()) {
        url.push_str("&qso_qslsince=");
        url.push_str(&urlencode(s));
    }
    url
}

/// First 10 characters of a stored "last sync" marker, or `None` for
/// blank/too-short input. Matches Android's `sinceDate` exactly.
pub fn since_date(last_qsl: Option<&str>) -> Option<String> {
    let s = last_qsl?.trim();
    if s.len() < 10 {
        return None;
    }
    Some(s[..10].to_string())
}

/// Parses a `lotwreport.adi` response body into confirmed QSLs. A bad login
/// gets an HTML error page back, not an HTTP error code -- no `<eoh>` at
/// all, detected here and turned into a specific message rather than a
/// silent "zero confirmations."
pub fn parse_report(text: &str) -> Result<Vec<LotwQsl>, String> {
    if !text.to_lowercase().contains("<eoh>") {
        return Err(if text.to_lowercase().contains("password") {
            "LoTW rejected the username/password.".to_string()
        } else {
            "LoTW did not return a report.".to_string()
        });
    }
    let mut out = Vec::new();
    for rec in adif::parse_adif(text) {
        let get = |k: &str| rec.fields.get(k).cloned().unwrap_or_default();
        if !get("QSL_RCVD").eq_ignore_ascii_case("Y") {
            continue;
        }
        let call = get("CALL");
        if call.is_empty() {
            continue;
        }
        let raw_mode = get("MODE").to_uppercase();
        let submode = get("SUBMODE").to_uppercase();
        let mode = if submode.is_empty() { raw_mode.clone() } else { submode };
        out.push(LotwQsl {
            call,
            qso_date: get("QSO_DATE"),
            time_on: get("TIME_ON"),
            band: get("BAND").to_lowercase(),
            mode,
            raw_mode,
        });
    }
    Ok(out)
}

/// Downloads and applies LoTW confirmations, advancing the stored "last
/// sync" marker to today on success so the next incremental call only asks
/// for what's new. `full_refresh` ignores that marker for one call (a
/// "download everything again" affordance) without discarding it.
pub fn download_confirmations(db: &Db, username: &str, password: &str, full_refresh: bool) -> LotwDownloadResult {
    if username.trim().is_empty() || password.is_empty() {
        return LotwDownloadResult {
            error: Some("Enter your LoTW username and password first.".to_string()),
            ..Default::default()
        };
    }
    let since = if full_refresh { None } else { since_date(db.get_config(CONFIG_LAST_QSL).as_deref()) };
    let url = build_report_url(username, password, since.as_deref());
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .build();
    let body = match agent.get(&url).call() {
        Ok(r) => match r.into_string() {
            Ok(b) => b,
            Err(e) => return LotwDownloadResult { error: Some(format!("{e}")), ..Default::default() },
        },
        Err(e) => return LotwDownloadResult { error: Some(format!("{e}")), ..Default::default() },
    };
    let qsls = match parse_report(&body) {
        Ok(q) => q,
        Err(e) => return LotwDownloadResult { error: Some(e), ..Default::default() },
    };
    let in_report = qsls.len();
    let newly_confirmed = db.apply_lotw_confirmations(&qsls);
    let _ = db.set_config(CONFIG_LAST_QSL, &chrono::Utc::now().format("%Y-%m-%d").to_string());
    LotwDownloadResult { in_report, newly_confirmed, error: None }
}

/// Exports every not-yet-uploaded QSO to a temp ADIF file and shells out to
/// `tqsl` to sign + upload them. See this module's top doc comment for the
/// exit-code caveat.
pub fn upload_pending(db: &Db, callsign: &str, upload_dir: &Path) -> LotwUploadResult {
    let pending = db.qsos_pending_lotw_upload();
    if pending.is_empty() {
        return LotwUploadResult::default();
    }
    let ids: Vec<i64> = pending.iter().filter_map(|q| q.id).collect();
    let mut adif_text = String::from("FT8AF LoTW upload<eoh>\n");
    for q in &pending {
        adif_text.push_str(&adif_record(q));
        adif_text.push('\n');
    }
    let path = upload_dir.join("lotw_upload_pending.adi");
    if let Err(e) = std::fs::create_dir_all(upload_dir).and_then(|_| std::fs::write(&path, &adif_text)) {
        return LotwUploadResult { attempted: 0, error: Some(format!("Could not write the upload file: {e}")) };
    }
    match std::process::Command::new("tqsl")
        .args(["-a", "all", "-d", "-u", "-x", "-c", callsign])
        .arg(&path)
        .output()
    {
        Ok(out) if out.status.success() => {
            let _ = db.mark_lotw_sent(&ids);
            LotwUploadResult { attempted: ids.len(), error: None }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let detail = if stderr.is_empty() { format!("tqsl exited with status {}", out.status) } else { stderr };
            LotwUploadResult { attempted: 0, error: Some(detail) }
        }
        Err(e) => LotwUploadResult { attempted: 0, error: Some(format!("Could not run tqsl: {e}")) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_report_url_encodes_credentials_and_omits_since_when_absent() {
        let url = build_report_url("K1ABC", "p@ss w/ord", None);
        assert!(url.starts_with(REPORT_URL));
        assert!(url.contains("login=K1ABC"));
        assert!(url.contains("password=p%40ss%20w%2Ford"));
        assert!(url.contains("qso_query=1&qso_qsl=yes&qso_qsldetail=no"));
        assert!(!url.contains("qso_qslsince"));
    }

    #[test]
    fn build_report_url_includes_since_when_present() {
        let url = build_report_url("K1ABC", "secret", Some("2026-01-01"));
        assert!(url.contains("qso_qslsince=2026-01-01"));
    }

    #[test]
    fn since_date_takes_first_ten_chars_or_none() {
        assert_eq!(since_date(Some("2026-01-01")), Some("2026-01-01".to_string()));
        assert_eq!(since_date(Some("2026-01-01 12:00:00")), Some("2026-01-01".to_string()));
        assert_eq!(since_date(Some("bad")), None);
        assert_eq!(since_date(Some("")), None);
        assert_eq!(since_date(None), None);
    }

    #[test]
    fn parse_report_detects_bad_login_html_page() {
        let html = "<html><body>Invalid login/password combination</body></html>";
        let err = parse_report(html).unwrap_err();
        assert!(err.to_lowercase().contains("password"));
    }

    #[test]
    fn parse_report_detects_an_unexpected_non_report_response() {
        let err = parse_report("<html><body>Service temporarily unavailable</body></html>").unwrap_err();
        assert!(!err.to_lowercase().contains("password"));
    }

    const REPORT: &str = "<PROGRAMID:5>LoTW<EOH>\n\
        <call:5>K1ABC<qso_date:8>20260101<time_on:4>1200<band:3>40m<mode:4>MFSK<submode:3>FT4<qsl_rcvd:1>Y<eor>\n\
        <call:5>W1AW <qso_date:8>20260102<time_on:6>130000<band:3>20m<mode:3>FT8<qsl_rcvd:1>N<eor>\n\
        <call:0><qso_date:8>20260103<time_on:4>1400<band:3>20m<mode:3>FT8<qsl_rcvd:1>Y<eor>\n";

    #[test]
    fn parse_report_extracts_only_confirmed_rows_with_a_callsign() {
        let qsls = parse_report(REPORT).unwrap();
        // Row 2 isn't QSL_RCVD=Y, row 3 has no callsign -- both dropped.
        assert_eq!(qsls.len(), 1);
        assert_eq!(qsls[0].call, "K1ABC");
        assert_eq!(qsls[0].band, "40m");
    }

    #[test]
    fn parse_report_prefers_submode_over_mode_for_the_ft4_quirk() {
        let qsls = parse_report(REPORT).unwrap();
        assert_eq!(qsls[0].mode, "FT4");
        assert_eq!(qsls[0].raw_mode, "MFSK");
    }

    #[test]
    fn parse_report_tolerates_a_typed_field_tag() {
        let typed = "<eoh>\n<call:5>K1ABC<qso_date:8:d>20260101<time_on:4>1200<band:3>40m<mode:3>FT8<qsl_rcvd:1>Y<eor>\n";
        let qsls = parse_report(typed).unwrap();
        assert_eq!(qsls.len(), 1);
        assert_eq!(qsls[0].qso_date, "20260101");
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "K1ABC", std::env::temp_dir().as_path());
        assert_eq!(result.attempted, 0);
        assert!(result.error.is_none());
    }
}
