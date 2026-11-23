//! HRDLog.net QSO upload.
//!
//! **Live-confirmed 2026-11-23: the upload field name was wrong.** This
//! module originally sent the ADIF record as a form field named `ADIF`,
//! guessed from secondhand forum reports since HRDLog's own docs
//! (hrdlog.net/docs/, /API.aspx, etc.) returned 403s and generic error
//! pages during research. Live-tested with the user's real account: even
//! a fully well-formed, non-blank ADIF record under that field name still
//! got the server's generic `Missing Call QSO_Date Time_On Band Mode`
//! rejection every time -- the real field was never reaching the parser
//! at all. Confirmed against two independent real source files reading
//! `https://robot.hrdlog.net/NewEntry.aspx` directly: IW1QLH's own
//! `HRDLOG-net-library` (IW1QLH runs hrdlog.net itself) and the
//! `wsjtx-tools` project's `hrdlog-upload.py` -- both use `Callsign`,
//! `Code`, `App`, and **`ADIFData`** (not `ADIF`) for the record, and both
//! check the reply for `<insert>1` (newly stored) / `<insert>0`
//! (already present -- a real duplicate, not an error) / `<error>...`
//! (a genuine rejection). [`upload_and_mark_one`] now classifies on that
//! basis instead of only "any non-2xx-shaped text counts as a failure."
//!
//! Field formats confirmed by the same two sources: `QSO_DATE` as
//! `YYYYMMDD` (no dashes), `TIME_ON` as 6-digit `HHMMSS`, `BAND` lowercase
//! (`"20m"`), matching what this module already sends -- those parts were
//! never the problem.
//!
//! **Live-confirmed 2026-11-22: this module needs its own TLS stack.**
//! The user's real Callsign/Code pair got a real "Connection Failed"
//! (`ureq::ErrorKind::ConnectionFailed`) on every attempt, while `curl`
//! and `openssl s_client` against the exact same endpoint from the exact
//! same machine succeeded every time with a normal response. Isolated to
//! this project's rustls-based TLS client specifically (confirmed by
//! swapping in `native-tls`/OpenSSL for one request and getting the same
//! success curl gets) -- robot.hrdlog.net (or something in front of it)
//! resets the TLS handshake for a rustls `ClientHello` even with an
//! explicit ALPN list added to try to close the gap, but accepts
//! OpenSSL's. So [`agent`] here builds its own `native-tls`-backed ureq
//! agent instead of this project's usual rustls default -- see
//! `Cargo.toml`'s `native-tls` dependency comment for why it's the
//! `vendored` (statically linked) build, not a system-library one.

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
    // See this module's top doc comment: robot.hrdlog.net resets a rustls
    // handshake but accepts OpenSSL's, so this module alone uses
    // native-tls instead of the rest of the project's rustls default.
    let connector = std::sync::Arc::new(
        native_tls::TlsConnector::new().expect("building a native-tls connector"),
    );
    ureq::AgentBuilder::new()
        .tls_connector(connector)
        .timeout_connect(HTTP_CONNECT_TIMEOUT)
        .timeout_read(HTTP_READ_TIMEOUT)
        .build()
}

/// Classifies a real `NewEntry.aspx` reply -- confirmed against two
/// independent real source files (see this module's top doc comment):
/// `<insert>1` means the record was newly stored, `<insert>0` means
/// HRDLog already had it (a real duplicate, not a failure). Anything else
/// (including the `<error>...</error>` shape this module's own Test
/// Connection responses show) is treated as a rejection -- there's no
/// confirmed third outcome shape to special-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertOutcome {
    Stored,
    AlreadyPresent,
    Rejected,
}

fn insert_outcome(body: &str) -> InsertOutcome {
    if body.contains("<insert>1") {
        InsertOutcome::Stored
    } else if body.contains("<insert>0") {
        InsertOutcome::AlreadyPresent
    } else {
        InsertOutcome::Rejected
    }
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

/// Builds a diagnostic message from a `ureq::Transport` without ever
/// touching its `url()` -- unlike `Transport`'s own `Display`, which
/// prepends the request URL (safe here regardless, since this module's
/// URL never carries credentials in a query string, but kept consistent
/// with this project's `lotw.rs`-established rule anyway). Prefixed with
/// "Network error:" so a TCP/TLS-level failure to reach the server (e.g.
/// ureq's own `ErrorKind::ConnectionFailed`, whose bare kind()  Display
/// text is literally "Connection Failed") is never mistaken for a real
/// HRDLog.net response rejecting the request -- confirmed live
/// 2026-11-22 that the real server accepts this project's exact
/// Callsign/Code pair (it replied with a normal "missing QSO fields"
/// error for a blank test ping, not an auth rejection, and "Unknown
/// user" for a genuinely wrong Code), so a bare transport error here
/// means the network hiccuped, not that credentials are wrong. Includes
/// `message()`/`source()` (e.g. the underlying `std::io::Error`/TLS
/// error text) so a *recurring* failure -- as opposed to the one-off
/// blip first assumed -- carries enough detail to actually diagnose,
/// rather than always showing the same uninformative "Connection
/// Failed" regardless of the real underlying cause.
fn describe_transport_error(t: &ureq::Transport) -> String {
    use std::error::Error as _;
    let mut s = format!("Network error: {}", t.kind());
    if let Some(m) = t.message() {
        s.push_str(&format!(" ({m})"));
    }
    if let Some(src) = t.source() {
        s.push_str(&format!(": {src}"));
    }
    s
}

fn send(callsign: &str, code: &str, adif: &str) -> Result<RawResponse, String> {
    let result = agent().post(UPLOAD_URL).send_form(&[
        ("Callsign", callsign),
        ("Code", code),
        ("App", APP_NAME),
        ("ADIFData", adif),
    ]);
    match result {
        Ok(r) => Ok(RawResponse { status: r.status(), body: r.into_string().unwrap_or_default() }),
        Err(ureq::Error::Status(status, response)) => {
            Ok(RawResponse { status, body: response.into_string().unwrap_or_default() })
        }
        Err(ureq::Error::Transport(t)) => Err(describe_transport_error(&t)),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

/// See this module's top doc comment: reports whether HRDLog answered at
/// all, with its raw (short, flattened) response text for the operator to
/// judge -- an empty `ADIFData` field is sent so nothing is actually
/// logged.
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
    pub already_present: usize,
    pub failed: usize,
    pub error: Option<String>,
}

enum UploadOutcome {
    Stored,
    AlreadyPresent,
    Failed(String),
}

fn upload_contact(callsign: &str, code: &str, q: &QsoRecord) -> UploadOutcome {
    let adif = adif_record(callsign, q);
    let resp = match send(callsign, code, &adif) {
        Ok(r) => r,
        Err(e) => return UploadOutcome::Failed(e),
    };
    if !(200..300).contains(&resp.status) {
        return UploadOutcome::Failed(format!("HTTP {}: {}", resp.status, clip(&resp.body)));
    }
    match insert_outcome(&resp.body) {
        InsertOutcome::Stored => UploadOutcome::Stored,
        InsertOutcome::AlreadyPresent => UploadOutcome::AlreadyPresent,
        // Not a confirmed <insert>0/1 -- a genuine <error>, or an
        // unrecognized shape. Either way, treat as a failure (left
        // unsynced for the next attempt) rather than silently assume
        // success for a reply we don't understand.
        InsertOutcome::Rejected => UploadOutcome::Failed(clip(&resp.body)),
    }
}

fn upload_and_mark_one(db: &Db, callsign: &str, code: &str, q: &QsoRecord) -> UploadOutcome {
    let outcome = upload_contact(callsign, code, q);
    if matches!(outcome, UploadOutcome::Stored | UploadOutcome::AlreadyPresent) {
        if let Some(id) = q.id {
            let _ = db.mark_synced(id, "hrdlog");
        }
    }
    outcome
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
            UploadOutcome::Stored => result.uploaded += 1,
            UploadOutcome::AlreadyPresent => result.already_present += 1,
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
    fn insert_outcome_classifies_the_confirmed_reply_shapes() {
        assert_eq!(insert_outcome("<insert>1"), InsertOutcome::Stored);
        assert_eq!(insert_outcome("<insert>0"), InsertOutcome::AlreadyPresent);
        assert_eq!(
            insert_outcome("<error>Missing Call QSO_Date Time_On Band Mode...</error>"),
            InsertOutcome::Rejected
        );
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
    fn describe_transport_error_includes_more_than_the_bare_kind() {
        // A real transport error -- nothing listens on this port, so the
        // OS refuses the connection immediately -- used to confirm
        // describe_transport_error surfaces the real underlying cause
        // (e.g. "Connection refused") instead of only ureq's generic
        // ErrorKind Display text ("Connection Failed"), which was the
        // whole point of this function: a recurring real failure needs
        // enough detail to diagnose, not just a repeat of the same vague
        // label every time.
        let err = ureq::post("http://127.0.0.1:1/").call().unwrap_err();
        let transport = err.into_transport().expect("a connection refusal is a transport error");
        let msg = describe_transport_error(&transport);
        assert!(msg.starts_with("Network error:"), "{msg}");
        assert!(msg.len() > "Network error: Connection Failed".len(), "{msg}");
    }

    #[test]
    #[ignore]
    fn scratch_live_check_connection() {
        // Not run in CI (needs real credentials via env vars) -- kept as a
        // manual diagnostic hook for this module's own live HRDLog.net
        // account. Exercises the real `agent()` (native-tls, per this
        // module's top doc comment), so a future TLS-level regression here
        // would show up as a transport error again, not a silent pass.
        let callsign = std::env::var("HRDLOG_CALLSIGN").unwrap();
        let code = std::env::var("HRDLOG_CODE").unwrap();
        let check = check_connection(&callsign, &code);
        eprintln!("ok={} detail={:?}", check.ok, check.detail);
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
