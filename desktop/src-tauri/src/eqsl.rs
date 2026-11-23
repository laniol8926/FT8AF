//! eQSL.cc upload + inbox download. **Lower confidence than this
//! project's QRZ/LoTW/Cloudlog/WRL/HamQTH/Club Log integrations, similar
//! to `hrdlog.rs`** -- eQSL has no current, authoritative API
//! documentation reachable during development (its help pages are mostly
//! login-gated or simply don't exist as static docs), so this is built
//! from the one thing confirmed directly against the live server plus
//! widely-repeated, cross-referenced community knowledge (forum reports
//! from Log4OM/other logging-program users hitting these exact
//! endpoints), not an official spec.
//!
//! **Live-confirmed 2026-11-22** against the user's own real eQSL.cc
//! account: `check_connection`'s header-only probe really does come back
//! `<!-- Reply form eQSL.cc ADIF Real-time Interface --> <HTML>
//! <HEAD></HEAD> <BODY> </BODY> </HTML>` on success (eQSL's own "Reply
//! form" wording, not a typo introduced here) -- HTTP 2xx, no
//! `credential_rejected` phrase, exactly what this module already
//! classified as `ok: true`, so no code change was needed. The same
//! session's "Upload now" reported `3 uploaded, 18 already there` -- the
//! 18 came back from eQSL's own server as `"duplicate"`-bearing
//! responses (this project's own earlier real uploads, consistent with
//! the user having already uploaded their whole log before this feature
//! existed), confirming the `ADIFData`/`EQSL_USER`/`EQSL_PSWD` upload
//! shape AND the duplicate-detection string against the real server, not
//! just a plausible guess. **Inbox download is still unconfirmed** --
//! only upload and the connection probe have real evidence behind them
//! so far; needs its own live test before it can be trusted.
//!
//! Two real, separate operations, not one:
//!
//! 1. **Upload** (`POST /qslcard/ImportADIF.cfm`): confirmed live that the
//!    endpoint requires an `ADIFData` parameter (`Error: Missing ADIFData
//!    parameter` when absent). Credential field names (`EQSL_USER`,
//!    `EQSL_PSWD`) come from a Log4OM forum thread quoting eQSL's own
//!    error text ("No match on eQSL_User/eQSL_Pswd") when a password was
//!    too long -- not directly confirmed by this project against a live
//!    account. One QSO per call (not batched, even though the real
//!    endpoint may well accept several `<eor>`-separated records in one
//!    `ADIFData` body) -- deliberately conservative: this project has no
//!    confirmed per-record result format for a multi-record response, and
//!    guessing one wrong risks marking an upload "synced" when it wasn't,
//!    which is worse than the lost efficiency of one request per QSO.
//! 2. **Inbox download** (confirmed Log4OM-forum-reported real-world
//!    usage, not official docs): `GET /qslcard/DownloadInBox.cfm` with
//!    `UserName`/`Password`/`QTHNickname`/`ConfirmedQSLOnly`/`RcvdSince`
//!    (format `YYYYMMDDHHMM`) generates a file server-side and answers
//!    with an HTML page naming it, rather than returning the ADIF
//!    directly; a second `GET` against the named `.adi` URL fetches the
//!    actual file. That file is then merged into the local log exactly
//!    like [`crate::db::Db::apply_lotw_confirmations`] -- UPDATE-only,
//!    **never inserting a new QSO for a record with no local match**
//!    (restated explicitly by the user for eQSL specifically; see
//!    `db.rs`'s `apply_eqsl_inbox` doc comment for why this one can still
//!    reuse the ADIF-import dedup matchers LoTW's own confirmations
//!    deliberately don't).
//!
//! Because the download's real response format is still unconfirmed,
//! [`check_connection`] -- like `hrdlog.rs`'s -- doesn't try to classify
//! pass/fail on its own steam beyond the one confirmed error phrase; it
//! shows eQSL's own response text for the operator to judge (now with a
//! real confirmed example of what success looks like, see above).

use serde::Serialize;

use crate::db::{adif_record, Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableEqsl";
pub const CONFIG_USERNAME: &str = "eqslUsername";
pub const CONFIG_PASSWORD: &str = "eqslPassword";
/// eQSL's "QTH Nickname" -- the station-location profile to download
/// confirmations for (an eQSL account can have more than one). Download
/// only; confirmed via forum reports that upload does not need it.
pub const CONFIG_NICKNAME: &str = "eqslNickname";
/// Last successful download, `YYYYMMDDHHMM` (eQSL's own `RcvdSince`
/// format) -- advanced to "now" on each successful download so the next
/// incremental call only asks for what's new, matching `lotw.rs`'s
/// `CONFIG_LAST_QSL` pattern.
pub const CONFIG_LAST_DOWNLOAD: &str = "eqslLastDownload";

const UPLOAD_URL: &str = "https://www.eqsl.cc/qslcard/ImportADIF.cfm";
const DOWNLOAD_URL: &str = "https://www.eqsl.cc/qslcard/DownloadInBox.cfm";
/// `RcvdSince` for a full-history download. Confirmed elsewhere in this
/// project (the LoTW "download all" fix, `lotw.rs`) that omitting a
/// since-style parameter is not safe to assume means "unbounded" without
/// real evidence -- an explicit, safely-ancient date is used instead.
const FULL_HISTORY_SINCE: &str = "200001010000";
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_DETAIL_LEN: usize = 300;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(HTTP_CONNECT_TIMEOUT).timeout_read(HTTP_READ_TIMEOUT).build()
}

fn clip(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > MAX_DETAIL_LEN {
        let c: String = flat.chars().take(MAX_DETAIL_LEN).collect();
        format!("{c}…")
    } else {
        flat
    }
}

fn credential_rejected(body: &str) -> bool {
    let b = body.to_lowercase();
    b.contains("no match") || b.contains("not a valid eqsl") || b.contains("incorrect password")
}

struct RawResponse {
    status: u16,
    body: String,
}

fn get(url: &str) -> Result<RawResponse, String> {
    match agent().get(url).call() {
        Ok(r) => Ok(RawResponse { status: r.status(), body: r.into_string().unwrap_or_default() }),
        Err(ureq::Error::Status(status, response)) => {
            Ok(RawResponse { status, body: response.into_string().unwrap_or_default() })
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<RawResponse, String> {
    match agent().post(url).send_form(fields) {
        Ok(r) => Ok(RawResponse { status: r.status(), body: r.into_string().unwrap_or_default() }),
        Err(ureq::Error::Status(status, response)) => {
            Ok(RawResponse { status, body: response.into_string().unwrap_or_default() })
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

/// Probes the upload credentials with a header-only `ADIFData` (no QSO
/// records, so nothing is actually uploaded) and reports whether the
/// response looks like the one confirmed credential-rejection phrase --
/// see this module's top doc comment for the real confidence level here.
pub fn check_connection(username: &str, password: &str) -> ConnectionCheck {
    if username.trim().is_empty() || password.is_empty() {
        return ConnectionCheck { ok: false, detail: Some("Enter your eQSL.cc username and password first.".to_string()) };
    }
    match post_form(UPLOAD_URL, &[("EQSL_USER", username.trim()), ("EQSL_PSWD", password), ("ADIFData", "<eoh>")]) {
        Ok(r) if credential_rejected(&r.body) => {
            ConnectionCheck { ok: false, detail: Some(clip(&r.body)) }
        }
        Ok(r) if (200..300).contains(&r.status) => ConnectionCheck { ok: true, detail: Some(clip(&r.body)) },
        Ok(r) => ConnectionCheck { ok: false, detail: Some(format!("HTTP {}: {}", r.status, clip(&r.body))) },
        Err(e) => ConnectionCheck { ok: false, detail: Some(e) },
    }
}

// --- Upload ------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct EqslUploadResult {
    pub uploaded: usize,
    pub already_present: usize,
    pub failed: usize,
    pub error: Option<String>,
}

enum UploadOutcome {
    Ok,
    AlreadyThere,
    Failed(String),
}

fn upload_contact(username: &str, password: &str, q: &QsoRecord) -> UploadOutcome {
    let adif = adif_record(q);
    let resp = match post_form(UPLOAD_URL, &[("EQSL_USER", username), ("EQSL_PSWD", password), ("ADIFData", &adif)]) {
        Ok(r) => r,
        Err(e) => return UploadOutcome::Failed(e),
    };
    if !(200..300).contains(&resp.status) {
        return UploadOutcome::Failed(format!("HTTP {}: {}", resp.status, clip(&resp.body)));
    }
    let lower = resp.body.to_lowercase();
    if credential_rejected(&resp.body) || lower.contains("error") {
        return UploadOutcome::Failed(clip(&resp.body));
    }
    if lower.contains("duplicate") {
        return UploadOutcome::AlreadyThere;
    }
    UploadOutcome::Ok
}

fn upload_and_mark_one(db: &Db, username: &str, password: &str, q: &QsoRecord) -> UploadOutcome {
    let outcome = upload_contact(username, password, q);
    if matches!(outcome, UploadOutcome::Ok | UploadOutcome::AlreadyThere) {
        if let Some(id) = q.id {
            let _ = db.mark_synced(id, "eqsl");
        }
    }
    outcome
}

/// Uploads every QSO not yet marked synced to eQSL (`eqsl_sent = 0`), one
/// `ImportADIF.cfm` call per QSO. Never stops early -- see this module's
/// top doc comment on why a credential failure isn't reliably
/// distinguishable from a per-QSO one here, so stopping early risks
/// leaving a genuinely-fixable backlog stuck longer than just trying each
/// row.
pub fn upload_pending(db: &Db, username: &str, password: &str) -> EqslUploadResult {
    if username.trim().is_empty() || password.is_empty() {
        return EqslUploadResult { error: Some("Enter your eQSL.cc username and password first.".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_eqsl_upload();
    let mut result = EqslUploadResult::default();
    for q in &pending {
        match upload_and_mark_one(db, username.trim(), password, q) {
            UploadOutcome::Ok => result.uploaded += 1,
            UploadOutcome::AlreadyThere => result.already_present += 1,
            UploadOutcome::Failed(e) => {
                if result.error.is_none() {
                    result.error = Some(e);
                }
                result.failed += 1;
            }
        }
    }
    result
}

/// Auto-upload hook for a QSO that was just logged -- same per-row logic
/// as [`upload_pending`], for exactly one record. Callers (engine.rs) run
/// this on a spawned thread, never the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, username: &str, password: &str, q: &QsoRecord) {
    if username.trim().is_empty() || password.is_empty() {
        return;
    }
    upload_and_mark_one(db, username.trim(), password, q);
}

// --- Download ------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct EqslDownloadResult {
    pub newly_confirmed: usize,
    pub unmatched: usize,
    pub error: Option<String>,
}

/// Finds a `.adi`/`.ADI` download URL in `DownloadInBox.cfm`'s HTML
/// response: a full `http(s)://...adi` URL if one's embedded directly,
/// else a bare `NAME.adi`-looking token resolved against eQSL's own
/// `downloadedfiles/` path (the prefix a CQRLOG reference screenshot
/// showed, `https://www.eqsl.cc/downloadedfiles/`). Hand-rolled rather
/// than pulling in a regex dependency for one small, simple pattern.
fn extract_adif_url(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    if let Some(start) = lower.find("http://").or_else(|| lower.find("https://")) {
        let rest = &body[start..];
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>')).unwrap_or(rest.len());
        let candidate = &rest[..end];
        if candidate.to_lowercase().ends_with(".adi") {
            return Some(candidate.to_string());
        }
    }
    // Fall back to a bare filename token ending in .adi/.ADI.
    let bytes = body.as_bytes();
    let lower_bytes = lower.as_bytes();
    let suffix = b".adi";
    let mut i = 0;
    while let Some(pos) = lower_bytes[i..].windows(suffix.len()).position(|w| w == suffix) {
        let end = i + pos + suffix.len();
        let mut start = end - suffix.len();
        while start > 0 {
            let c = bytes[start - 1] as char;
            if c.is_alphanumeric() || c == '_' || c == '-' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < end - suffix.len() {
            return Some(format!("https://www.eqsl.cc/downloadedfiles/{}", &body[start..end]));
        }
        i = end;
        if i >= lower_bytes.len() {
            break;
        }
    }
    None
}

fn looks_like_no_new_confirmations(body: &str) -> bool {
    let b = body.to_lowercase();
    // "you have no log entries" is eQSL's own confirmed real wording
    // (live-tested 2026-11-23 with a narrow RcvdSince window) -- the
    // other three phrases were guesses from secondhand reports and have
    // never actually been seen from the real server.
    b.contains("you have no log entries")
        || b.contains("no new")
        || b.contains("0 new")
        || b.contains("nothing new")
}

/// Downloads eQSL's inbox and applies it to the local log. `full_refresh`
/// ignores the stored last-download marker for one call, using
/// [`FULL_HISTORY_SINCE`] rather than omitting `RcvdSince` -- see that
/// constant's doc comment for why that distinction matters (confirmed
/// elsewhere in this project to be a real, not theoretical, failure mode).
pub fn download_confirmations(db: &Db, username: &str, password: &str, nickname: &str, full_refresh: bool) -> EqslDownloadResult {
    if username.trim().is_empty() || password.is_empty() {
        return EqslDownloadResult { error: Some("Enter your eQSL.cc username and password first.".to_string()), ..Default::default() };
    }
    if nickname.trim().is_empty() {
        return EqslDownloadResult { error: Some("Enter your eQSL.cc QTH Nickname first.".to_string()), ..Default::default() };
    }
    let since = if full_refresh {
        FULL_HISTORY_SINCE.to_string()
    } else {
        db.get_config(CONFIG_LAST_DOWNLOAD).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| FULL_HISTORY_SINCE.to_string())
    };
    let url = format!(
        "{DOWNLOAD_URL}?UserName={}&Password={}&QTHNickname={}&ConfirmedQSLOnly=1&RcvdSince={}",
        urlencode(username.trim()),
        urlencode(password),
        urlencode(nickname.trim()),
        urlencode(&since),
    );
    let first = match get(&url) {
        Ok(r) => r,
        Err(e) => return EqslDownloadResult { error: Some(e), ..Default::default() },
    };
    if !(200..300).contains(&first.status) {
        return EqslDownloadResult { error: Some(format!("HTTP {}: {}", first.status, clip(&first.body))), ..Default::default() };
    }
    if credential_rejected(&first.body) {
        return EqslDownloadResult { error: Some(clip(&first.body)), ..Default::default() };
    }
    let Some(adif_url) = extract_adif_url(&first.body) else {
        if looks_like_no_new_confirmations(&first.body) {
            let _ = db.set_config(CONFIG_LAST_DOWNLOAD, &now_yyyymmddhhmm());
            return EqslDownloadResult::default();
        }
        return EqslDownloadResult { error: Some(clip(&first.body)), ..Default::default() };
    };
    let second = match get(&adif_url) {
        Ok(r) => r,
        Err(e) => return EqslDownloadResult { error: Some(e), ..Default::default() },
    };
    if !(200..300).contains(&second.status) {
        return EqslDownloadResult { error: Some(format!("HTTP {} fetching the inbox file", second.status)), ..Default::default() };
    }
    let (newly_confirmed, unmatched) = db.apply_eqsl_inbox(&second.body);
    let _ = db.set_config(CONFIG_LAST_DOWNLOAD, &now_yyyymmddhhmm());
    EqslDownloadResult { newly_confirmed, unmatched, error: None }
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

fn now_yyyymmddhhmm() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_rejected_matches_the_confirmed_forum_wording() {
        assert!(credential_rejected("No match on eQSL_User/eQSL_Pswd"));
        assert!(!credential_rejected("Your log has been imported."));
    }

    #[test]
    fn credential_rejected_does_not_false_positive_on_the_real_live_confirmed_success_body() {
        // Real response body from the user's own eQSL.cc account,
        // 2026-11-22 (see this module's doc comment) -- a genuine success
        // reply must never be misread as a credential rejection.
        let real_success_body =
            "<!-- Reply form eQSL.cc ADIF Real-time Interface --> <HTML> <HEAD></HEAD> <BODY> </BODY> </HTML>";
        assert!(!credential_rejected(real_success_body));
    }

    #[test]
    fn extract_adif_url_finds_a_full_url_in_the_body() {
        let body = r#"Your file is ready: <a href="https://www.eqsl.cc/downloadedfiles/AI5II.adi">download</a>"#;
        assert_eq!(extract_adif_url(body), Some("https://www.eqsl.cc/downloadedfiles/AI5II.adi".to_string()));
    }

    #[test]
    fn extract_adif_url_falls_back_to_a_bare_filename() {
        let body = "Your inbox has been saved to AI5II_12345.ADI for retrieval.";
        assert_eq!(
            extract_adif_url(body),
            Some("https://www.eqsl.cc/downloadedfiles/AI5II_12345.ADI".to_string())
        );
    }

    #[test]
    fn extract_adif_url_returns_none_without_a_recognizable_filename() {
        assert_eq!(extract_adif_url("No new QSLs since your last download."), None);
    }

    #[test]
    fn looks_like_no_new_confirmations_matches_common_wording() {
        assert!(looks_like_no_new_confirmations("No new QSLs since your last download."));
        assert!(looks_like_no_new_confirmations("0 new records found"));
        assert!(!looks_like_no_new_confirmations("Your file is ready"));
    }

    #[test]
    fn looks_like_no_new_confirmations_matches_the_real_server_wording() {
        // Confirmed live 2026-11-23 against the real eQSL.cc server with a
        // narrow RcvdSince window: this is the ACTUAL wording, not a guess
        // like the three phrases above (never confirmed against a real
        // response). Without this, a real "nothing new" result was
        // misclassified as a download error.
        assert!(looks_like_no_new_confirmations("<H3>You have no log entries</H3>"));
    }

    #[test]
    fn check_connection_requires_credentials() {
        assert!(!check_connection("", "").ok);
        assert!(!check_connection("K1ABC", "").ok);
    }

    #[test]
    fn upload_pending_is_a_no_op_without_credentials() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "", "");
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "K1ABC", "fake-password-not-used-because-nothing-pending");
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }

    #[test]
    fn download_confirmations_requires_credentials_and_a_nickname() {
        let db = Db::open_in_memory().unwrap();
        let r = download_confirmations(&db, "", "", "", false);
        assert!(r.error.is_some());
        let r = download_confirmations(&db, "K1ABC", "pw", "", false);
        assert!(r.error.unwrap().contains("Nickname"));
    }

    #[test]
    #[ignore]
    fn scratch_dump_raw_inbox_html() {
        // Temporary diagnostic: dump the real, un-clipped first-step HTML
        // so extract_adif_url/looks_like_no_new_confirmations can be fixed
        // against eQSL's actual response shape.
        let username = std::env::var("EQSL_USERNAME").unwrap();
        let password = std::env::var("EQSL_PASSWORD").unwrap();
        let nickname = std::env::var("EQSL_NICKNAME").unwrap();
        let since = std::env::var("EQSL_SINCE").unwrap_or_else(|_| FULL_HISTORY_SINCE.to_string());
        let url = format!(
            "{DOWNLOAD_URL}?UserName={}&Password={}&QTHNickname={}&ConfirmedQSLOnly=1&RcvdSince={}",
            urlencode(username.trim()),
            urlencode(&password),
            urlencode(nickname.trim()),
            urlencode(&since),
        );
        let r = get(&url).unwrap();
        let out_path = std::env::var("EQSL_DUMP_PATH").unwrap();
        std::fs::write(&out_path, format!("status={}\n{}", r.status, r.body)).unwrap();
        eprintln!("dumped to {out_path}, status={}, len={}", r.status, r.body.len());
    }

    #[test]
    #[ignore]
    fn scratch_live_download_confirmations() {
        // Not run in CI -- a real, one-off live test against the user's
        // own eQSL.cc account and REAL production database (path via
        // env var), run the same way the "Download new" button in
        // Settings would: never inserts a new QSO, only updates matched
        // ones (see apply_eqsl_inbox's own doc comment).
        let db_path = std::env::var("EQSL_DB_PATH").unwrap();
        let username = std::env::var("EQSL_USERNAME").unwrap();
        let password = std::env::var("EQSL_PASSWORD").unwrap();
        let nickname = std::env::var("EQSL_NICKNAME").unwrap();
        let db = Db::open(&db_path).unwrap();
        let full_refresh = std::env::var("EQSL_FULL_REFRESH").as_deref() == Ok("1");
        let result = download_confirmations(&db, &username, &password, &nickname, full_refresh);
        eprintln!(
            "newly_confirmed={} unmatched={} error={:?}",
            result.newly_confirmed, result.unmatched, result.error
        );
    }
}
