//! HamQTH logbook upload, using HamQTH's modern API v2 (confirmed live
//! against `https://www.hamqth.com/developers.php` -- NOT the older XML
//! callbook-lookup API this codebase already has in `qrz.rs`'s general
//! style; HamQTH has no equivalent of that lookup here, this module is
//! upload-only).
//!
//! Two calls:
//! 1. `POST /api/v2/login` with `{"callsign", "password"}` -> a bearer
//!    token, valid for `expiresIn` seconds (observed: 3600). Login,
//!    not a separate "username" -- HamQTH's own account identifier
//!    *is* the operator's callsign.
//! 2. `POST /api/v2/log/qso` with `Authorization: Bearer <token>` and body
//!    `{"qso": {...}}`, ADIF-shaped lowercase field names. Required:
//!    `call`, `qso_date`, `time_on`, and `band` or `freq`. Returns 201 on
//!    success. A 409 means the QSO already exists in the log (treated the
//!    same as success -- HamQTH already has it); 401 means the token
//!    expired or was never valid; other 4xx means the QSO itself was
//!    rejected.
//!
//! The token is cached in memory for the process's lifetime (matching
//! `qrz.rs`'s `QrzXmlClient` session-key pattern) and only re-fetched after
//! a 401, since re-logging in before every single QSO would be wasteful.
//!
//! **Field names beyond the four required ones are a reasoned guess**: the
//! documentation page confirms the shape (`call`, `qso_date`, `time_on`,
//! `band`, `mode`, `freq` as literal lowercase ADIF-style keys) but doesn't
//! enumerate every optional field HamQTH accepts. `rst_sent`/`rst_rcvd`/
//! `gridsquare`/`my_gridsquare`/`comment`/`station_callsign` are included
//! on that same naming convention -- if HamQTH rejects or ignores any of
//! them, the four required fields still upload correctly either way, since
//! unknown JSON keys are typically ignored rather than rejected. Needs
//! live verification against a real HamQTH account.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::db::{Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableHamQTH";
pub const CONFIG_PASSWORD: &str = "hamqthPassword";

const BASE_URL: &str = "https://www.hamqth.com/api/v2";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Re-login a little before HamQTH's own `expiresIn` (observed 3600s) to
/// avoid a request racing an expiry right at the boundary.
const TOKEN_SAFETY_MARGIN: Duration = Duration::from_secs(60);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(HTTP_CONNECT_TIMEOUT).timeout_read(HTTP_READ_TIMEOUT).build()
}

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

fn describe_failure(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(str::to_string));
    match detail {
        Some(d) if !d.is_empty() => format!("HTTP {status}: {d}"),
        _ => {
            let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
            if flat.is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}: {flat}")
            }
        }
    }
}

fn login(callsign: &str, password: &str) -> Result<(String, Duration), String> {
    let body = serde_json::json!({ "callsign": callsign, "password": password }).to_string();
    let resp = agent().post(&format!("{BASE_URL}/login")).set("Content-Type", "application/json").send_string(&body);
    match resp {
        Ok(r) => {
            let body = r.into_string().map_err(|e| format!("{e}"))?;
            let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| "unexpected login reply".to_string())?;
            let token = v.get("token").and_then(|t| t.as_str()).ok_or("login reply had no token")?;
            let expires = v.get("expiresIn").and_then(|e| e.as_u64()).unwrap_or(3600);
            Ok((token.to_string(), Duration::from_secs(expires)))
        }
        Err(ureq::Error::Status(code, response)) => {
            Err(describe_failure(code, &response.into_string().unwrap_or_default()))
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Process-lifetime token cache, keyed implicitly by whatever callsign/
/// password are currently configured -- a credential change naturally
/// invalidates it on the next 401 (see [`ensure_token`]'s retry-once path),
/// same tradeoff as re-logging in on every password change being
/// unnecessary complexity for a single-operator desktop app.
static TOKEN: Mutex<Option<CachedToken>> = Mutex::new(None);

fn ensure_token(callsign: &str, password: &str) -> Result<String, String> {
    {
        let cached = TOKEN.lock().unwrap();
        if let Some(c) = cached.as_ref() {
            if c.expires_at > Instant::now() {
                return Ok(c.token.clone());
            }
        }
    }
    let (token, ttl) = login(callsign, password)?;
    let expires_at = Instant::now() + ttl.saturating_sub(TOKEN_SAFETY_MARGIN);
    *TOKEN.lock().unwrap() = Some(CachedToken { token: token.clone(), expires_at });
    Ok(token)
}

fn invalidate_token() {
    *TOKEN.lock().unwrap() = None;
}

/// Verifies the configured callsign/password with a fresh login, discarding
/// any cached token first -- matches this codebase's other `test_connection`
/// commands (`qrz.rs`'s XML client).
pub fn check_connection(callsign: &str, password: &str) -> ConnectionCheck {
    if callsign.trim().is_empty() || password.is_empty() {
        return ConnectionCheck { ok: false, detail: Some("Enter your HamQTH callsign and password first.".to_string()) };
    }
    invalidate_token();
    match login(callsign.trim(), password) {
        Ok(_) => ConnectionCheck { ok: true, detail: None },
        Err(e) => ConnectionCheck { ok: false, detail: Some(e) },
    }
}

fn qso_json(r: &QsoRecord) -> serde_json::Value {
    let mut o = serde_json::json!({
        "call": r.call,
        "qso_date": r.qso_date,
        "time_on": r.time_on,
        "band": r.band,
        "mode": r.mode,
    });
    if !r.freq.trim().is_empty() {
        o["freq"] = serde_json::json!(r.freq);
    }
    if !r.rst_sent.trim().is_empty() {
        o["rst_sent"] = serde_json::json!(r.rst_sent);
    }
    if !r.rst_rcvd.trim().is_empty() {
        o["rst_rcvd"] = serde_json::json!(r.rst_rcvd);
    }
    if !r.gridsquare.trim().is_empty() {
        o["gridsquare"] = serde_json::json!(r.gridsquare);
    }
    if !r.my_gridsquare.trim().is_empty() {
        o["my_gridsquare"] = serde_json::json!(r.my_gridsquare);
    }
    if !r.comment.trim().is_empty() {
        o["comment"] = serde_json::json!(r.comment);
    }
    if !r.station_callsign.trim().is_empty() {
        o["station_callsign"] = serde_json::json!(r.station_callsign.trim().to_uppercase());
    }
    o
}

enum UploadOutcome {
    Ok,
    /// HamQTH already has this QSO (409) -- treated like success.
    AlreadyThere,
    Failed(String),
    /// The token was rejected even after a fresh login retry, or the
    /// credentials themselves are wrong -- every further upload in a batch
    /// would fail the same way.
    FailedStopBatch(String),
}

fn upload_contact(callsign: &str, password: &str, r: &QsoRecord) -> UploadOutcome {
    let token = match ensure_token(callsign, password) {
        Ok(t) => t,
        Err(e) => return UploadOutcome::FailedStopBatch(e),
    };
    let body = serde_json::json!({ "qso": qso_json(r) }).to_string();
    let send = |tok: &str| {
        agent()
            .post(&format!("{BASE_URL}/log/qso"))
            .set("Authorization", &format!("Bearer {tok}"))
            .set("Content-Type", "application/json")
            .send_string(&body)
    };
    match send(&token) {
        Ok(_) => UploadOutcome::Ok,
        Err(ureq::Error::Status(409, _)) => UploadOutcome::AlreadyThere,
        Err(ureq::Error::Status(401, _)) => {
            // Token expired/rejected: one retry with a fresh login, since a
            // cached-but-now-stale token shouldn't fail the whole batch.
            invalidate_token();
            let token = match ensure_token(callsign, password) {
                Ok(t) => t,
                Err(e) => return UploadOutcome::FailedStopBatch(e),
            };
            match send(&token) {
                Ok(_) => UploadOutcome::Ok,
                Err(ureq::Error::Status(409, _)) => UploadOutcome::AlreadyThere,
                Err(ureq::Error::Status(code, response)) => {
                    UploadOutcome::FailedStopBatch(describe_failure(code, &response.into_string().unwrap_or_default()))
                }
                Err(ureq::Error::Transport(t)) => UploadOutcome::FailedStopBatch(format!("{}", t.kind())),
            }
        }
        Err(ureq::Error::Status(code, response)) => {
            UploadOutcome::Failed(describe_failure(code, &response.into_string().unwrap_or_default()))
        }
        Err(ureq::Error::Transport(t)) => UploadOutcome::Failed(format!("{}", t.kind())),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct HamQthUploadResult {
    pub uploaded: usize,
    pub already_present: usize,
    pub failed: usize,
    pub error: Option<String>,
}

fn upload_and_mark_one(db: &Db, callsign: &str, password: &str, q: &QsoRecord) -> Result<bool, (String, bool)> {
    match upload_contact(callsign, password, q) {
        UploadOutcome::Ok => {
            if let Some(id) = q.id {
                let _ = db.mark_synced(id, "hamqth");
            }
            Ok(false)
        }
        UploadOutcome::AlreadyThere => {
            if let Some(id) = q.id {
                let _ = db.mark_synced(id, "hamqth");
            }
            Ok(true)
        }
        UploadOutcome::Failed(e) => Err((e, false)),
        UploadOutcome::FailedStopBatch(e) => Err((e, true)),
    }
}

/// Uploads every QSO not yet marked synced to HamQTH (`synced_hamqth = 0`).
/// Stops early once the login/token itself is rejected (every remaining
/// row would fail the same way); a single QSO's own rejection just moves
/// on to the next row.
pub fn upload_pending(db: &Db, callsign: &str, password: &str) -> HamQthUploadResult {
    if callsign.trim().is_empty() || password.is_empty() {
        return HamQthUploadResult { error: Some("Enter your HamQTH callsign and password first.".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_hamqth_upload();
    let mut result = HamQthUploadResult::default();
    for q in &pending {
        match upload_and_mark_one(db, callsign, password, q) {
            Ok(true) => result.already_present += 1,
            Ok(false) => result.uploaded += 1,
            Err((e, stop)) => {
                if result.error.is_none() || stop {
                    result.error = Some(e);
                }
                result.failed += 1;
                if stop {
                    break;
                }
            }
        }
    }
    result
}

/// Auto-upload hook for a QSO that was just logged -- same per-row logic as
/// [`upload_pending`], for exactly one record. Callers (engine.rs) run this
/// on a spawned thread, never the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, callsign: &str, password: &str, q: &QsoRecord) {
    if callsign.trim().is_empty() || password.is_empty() {
        return;
    }
    let _ = upload_and_mark_one(db, callsign, password, q);
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
            my_gridsquare: "EM20".to_string(),
            comment: "QSO by FT8AF".to_string(),
            station_callsign: "n0call".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn qso_json_includes_the_four_required_fields_always() {
        let j = qso_json(&rec());
        assert_eq!(j["call"], "K1ABC");
        assert_eq!(j["qso_date"], "20261122");
        assert_eq!(j["time_on"], "1230");
        assert_eq!(j["band"], "20m");
        assert_eq!(j["mode"], "FT8");
    }

    #[test]
    fn qso_json_omits_blank_optional_fields() {
        let mut r = rec();
        r.gridsquare = "".to_string();
        r.comment = "".to_string();
        let j = qso_json(&r);
        assert!(j.get("gridsquare").is_none());
        assert!(j.get("comment").is_none());
        assert_eq!(j["rst_sent"], "-10");
    }

    #[test]
    fn qso_json_uppercases_the_station_callsign() {
        let j = qso_json(&rec());
        assert_eq!(j["station_callsign"], "N0CALL");
    }

    #[test]
    fn describe_failure_prefers_the_detail_field() {
        assert_eq!(
            describe_failure(401, r#"{"detail":"invalid or expired token"}"#),
            "HTTP 401: invalid or expired token"
        );
        assert_eq!(describe_failure(500, ""), "HTTP 500");
        assert_eq!(describe_failure(500, "<html>oops</html>"), "HTTP 500: <html>oops</html>");
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
}
