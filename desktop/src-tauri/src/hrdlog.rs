//! HRDLog.net QSO upload. **Lower confidence than this project's other
//! logging integrations** -- HRDLog.net's own documentation pages
//! (hrdlog.net/docs/, /API.aspx, etc.) could not be reached (403s and
//! generic error pages during research), so this is built against the
//! best available secondhand description (third-party logging-program
//! source and forum posts, cross-checked across two independent
//! searches, not HRDLog's own authoritative docs): `POST` to
//! `https://robot.hrdlog.net/NewEntry.aspx` with form fields `Callsign`
//! (the operator's own callsign), `Code` (the upload code HRDLog emails
//! after registration -- found at hrdlog.net under the account's own
//! settings), `App` (software name), and `ADIF` (one record, `<eor>`
//! included). **Needs live verification against a real HRDLog.net
//! account** -- the endpoint, field names, and response shape here may
//! need correcting once that happens.
//!
//! Because the real response format is unconfirmed, [`check_connection`]
//! deliberately does not try to classify pass/fail itself: it reports
//! whether the server answered at all (HTTP 2xx) and returns the raw
//! response text for the operator to read, rather than risk guessing a
//! success/failure keyword wrong. The same uncertainty means duplicate-QSO
//! detection isn't implemented -- any rejection (a real error, or a
//! duplicate HRDLog already has) is treated as a plain failure and left
//! unsynced for the next attempt, rather than guessing which one it was.

use serde::Serialize;

use crate::db::{Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableHRDLog";
pub const CONFIG_CODE: &str = "hrdlogCode";

const UPLOAD_URL: &str = "https://robot.hrdlog.net/NewEntry.aspx";
const APP_NAME: &str = "FT8AF";
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Longest response body kept for a failure message.
const MAX_DETAIL_LEN: usize = 300;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(HTTP_CONNECT_TIMEOUT).timeout_read(HTTP_READ_TIMEOUT).build()
}

/// Keywords a rejection is likely to contain -- used only to decide
/// whether an upload counts as accepted, never to build a user-facing
/// message (the raw body is already short and shown verbatim instead).
fn looks_like_rejection(body: &str) -> bool {
    let b = body.to_lowercase();
    b.contains("error") || b.contains("invalid") || b.contains("fail") || b.contains("denied") || b.contains("unauthorized")
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

fn adif_record(callsign: &str, r: &QsoRecord) -> String {
    let mut out = String::new();
    let field = |out: &mut String, tag: &str, value: &str| {
        let v = value.trim();
        if !v.is_empty() {
            out.push_str(&format!("<{}:{}>{} ", tag, v.chars().count(), v));
        }
    };
    field(&mut out, "CALL", &r.call);
    field(&mut out, "QSO_DATE", &r.qso_date);
    field(&mut out, "TIME_ON", &r.time_on);
    field(&mut out, "BAND", &r.band);
    field(&mut out, "MODE", &r.mode);
    field(&mut out, "FREQ", &r.freq);
    field(&mut out, "RST_SENT", &r.rst_sent);
    field(&mut out, "RST_RCVD", &r.rst_rcvd);
    field(&mut out, "GRIDSQUARE", &r.gridsquare);
    field(&mut out, "STATION_CALLSIGN", callsign);
    out.push_str("<eor>");
    out
}

struct RawResponse {
    status: u16,
    body: String,
}

fn send(callsign: &str, code: &str, adif: &str) -> Result<RawResponse, String> {
    let result = agent().post(UPLOAD_URL).send_form(&[
        ("Callsign", callsign),
        ("Code", code),
        ("App", APP_NAME),
        ("ADIF", adif),
    ]);
    match result {
        Ok(r) => Ok(RawResponse { status: r.status(), body: r.into_string().unwrap_or_default() }),
        Err(ureq::Error::Status(status, response)) => {
            Ok(RawResponse { status, body: response.into_string().unwrap_or_default() })
        }
        // Prefixed so a TCP/TLS-level failure to reach the server (e.g.
        // ureq's own `ErrorKind::ConnectionFailed`, whose Display text is
        // literally "Connection Failed") is never mistaken for a real
        // HRDLog.net response rejecting the request -- confirmed live
        // 2026-11-22 that the real server accepts this project's exact
        // Callsign/Code pair (it replied with a normal "missing QSO
        // fields" error for a blank test ping, not an auth rejection), so
        // a bare transport error here means the network hiccuped, not
        // that credentials are wrong.
        Err(ureq::Error::Transport(t)) => Err(format!("Network error: {}", t.kind())),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

/// See this module's top doc comment: reports whether HRDLog answered at
/// all, with its raw (short, flattened) response text for the operator to
/// judge -- an empty `ADIF` field is sent so nothing is actually logged.
pub fn check_connection(callsign: &str, code: &str) -> ConnectionCheck {
    if callsign.trim().is_empty() || code.trim().is_empty() {
        return ConnectionCheck { ok: false, detail: Some("Enter your HRDLog.net callsign and Code first.".to_string()) };
    }
    match send(callsign.trim(), code.trim(), "") {
        Ok(r) if (200..300).contains(&r.status) => {
            ConnectionCheck { ok: true, detail: Some(clip(&r.body)) }
        }
        Ok(r) => ConnectionCheck { ok: false, detail: Some(format!("HTTP {}: {}", r.status, clip(&r.body))) },
        Err(e) => ConnectionCheck { ok: false, detail: Some(e) },
    }
}

#[derive(Debug, Default, Serialize)]
pub struct HrdlogUploadResult {
    pub uploaded: usize,
    pub failed: usize,
    pub error: Option<String>,
}

fn upload_and_mark_one(db: &Db, callsign: &str, code: &str, q: &QsoRecord) -> Result<(), String> {
    let adif = adif_record(callsign, q);
    let resp = send(callsign, code, &adif)?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("HTTP {}: {}", resp.status, clip(&resp.body)));
    }
    if looks_like_rejection(&resp.body) {
        return Err(clip(&resp.body));
    }
    if let Some(id) = q.id {
        let _ = db.mark_synced(id, "hrdlog");
    }
    Ok(())
}

/// Uploads every QSO not yet marked synced to HRDLog.net
/// (`synced_hrdlog = 0`), one request per QSO (no batch mechanism is
/// documented). Never stops early -- with no confirmed way to distinguish
/// "every row will fail the same way" (bad Code) from "this one row was
/// rejected", stopping early risks leaving a genuinely-fixable backlog
/// stuck for longer than just trying each row.
pub fn upload_pending(db: &Db, callsign: &str, code: &str) -> HrdlogUploadResult {
    if callsign.trim().is_empty() || code.trim().is_empty() {
        return HrdlogUploadResult { error: Some("Enter your HRDLog.net callsign and Code first.".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_hrdlog_upload();
    let mut result = HrdlogUploadResult::default();
    for q in &pending {
        match upload_and_mark_one(db, callsign.trim(), code.trim(), q) {
            Ok(()) => result.uploaded += 1,
            Err(e) => {
                if result.error.is_none() {
                    result.error = Some(e);
                }
                result.failed += 1;
            }
        }
    }
    result
}

/// Auto-upload hook for a QSO that was just logged -- same per-row logic as
/// [`upload_pending`], for exactly one record. Callers (engine.rs) run this
/// on a spawned thread, never the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, callsign: &str, code: &str, q: &QsoRecord) {
    if callsign.trim().is_empty() || code.trim().is_empty() {
        return;
    }
    let _ = upload_and_mark_one(db, callsign.trim(), code.trim(), q);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> QsoRecord {
        QsoRecord {
            call: "K1ABC".to_string(),
            qso_date: "20261122".to_string(),
            time_on: "1230".to_string(),
            band: "20m".to_string(),
            mode: "FT8".to_string(),
            freq: "14.074".to_string(),
            rst_sent: "-10".to_string(),
            rst_rcvd: "-05".to_string(),
            gridsquare: "FN31".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn adif_record_includes_the_core_fields_and_ends_with_eor() {
        let a = adif_record("N0CALL", &rec());
        assert!(a.contains("<CALL:5>K1ABC"));
        assert!(a.contains("<STATION_CALLSIGN:6>N0CALL"));
        assert!(a.ends_with("<eor>"));
    }

    #[test]
    fn adif_record_omits_blank_fields() {
        let mut r = rec();
        r.gridsquare = "".to_string();
        let a = adif_record("N0CALL", &r);
        assert!(!a.contains("GRIDSQUARE"));
    }

    #[test]
    fn looks_like_rejection_matches_common_error_wording() {
        assert!(looks_like_rejection("Error: invalid code"));
        assert!(looks_like_rejection("ACCESS DENIED"));
        assert!(!looks_like_rejection("OK, 1 QSO added"));
    }

    #[test]
    fn clip_flattens_and_truncates() {
        assert_eq!(clip("  line one\nline two  "), "line one line two");
        let long = "x".repeat(400);
        let out = clip(&long);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < 400);
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
        let result = upload_pending(&db, "K1ABC", "fake-code-not-used-because-nothing-pending");
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }
}
