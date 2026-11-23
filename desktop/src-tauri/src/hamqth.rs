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
/// password are currently configured. Only `check_connection` uses this
/// now (see [`upload_pending`]'s own doc comment for why uploading is
/// disabled) -- it calls `invalidate_token` itself before every check, so
/// a credential change always gets a fresh login rather than a stale
/// cached token.
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

#[derive(Debug, Default, Serialize)]
pub struct HamQthUploadResult {
    pub uploaded: usize,
    pub already_present: usize,
    pub failed: usize,
    pub error: Option<String>,
}

/// **Disabled, deliberately, 2026-11-23 -- see `engine.rs`'s `Engine::new`
/// doc comment for the full real-risk explanation.** The user read
/// hamqth.com's own "Upload log" page, which says an ADIF upload "replaces
/// all QSOs of this callsign on HamQTH, so always upload your complete
/// log" -- that page describes hamqth.com's bulk FILE-upload feature, not
/// this module's `/api/v2/log/qso` JSON endpoint, so it isn't confirmed
/// this endpoint shares that whole-log-replace behavior (a real `409 "The
/// QSO is in the log already"` reply, confirmed live, looks like per-QSO
/// tracking instead). But the risk if it IS the same underlying
/// mechanism -- every single incrementally-uploaded QSO wiping out
/// everything else -- was judged too severe to keep running
/// automatically, so the user chose to upload manually via hamqth.com's
/// own whole-log ADIF feature instead. This returns immediately without
/// making any request. [`check_connection`] is untouched -- it only logs
/// in, never touches the log, so it stays safe and useful for verifying
/// credentials are still valid.
pub fn upload_pending(_db: &Db, _callsign: &str, _password: &str) -> HamQthUploadResult {
    HamQthUploadResult {
        error: Some(
            "HamQTH auto-upload is disabled -- HamQTH's own site says an ADIF upload replaces your whole log, so upload manually via hamqth.com instead.".to_string(),
        ),
        ..Default::default()
    }
}

/// Disabled -- see [`upload_pending`]'s own doc comment.
pub fn upload_one_logged(_db: &Db, _callsign: &str, _password: &str, _q: &QsoRecord) {}

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
    fn upload_pending_is_disabled_regardless_of_credentials() {
        // Live safety disable, 2026-11-23 -- see upload_pending's own doc
        // comment. Must never attempt a real upload no matter what
        // credentials or pending state is passed.
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "", "");
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());

        let result = upload_pending(&db, "K1ABC", "a-real-looking-password");
        assert_eq!(result.uploaded, 0);
        assert!(result.error.unwrap().to_lowercase().contains("disabled"));
    }

    #[test]
    fn upload_one_logged_is_disabled_and_never_panics() {
        let db = Db::open_in_memory().unwrap();
        upload_one_logged(&db, "K1ABC", "a-real-looking-password", &rec());
    }

    #[test]
    #[ignore]
    fn scratch_live_single_qso_upload() {
        // Not run in CI -- a real, one-off live upload of ONE real QSO to
        // see the ACTUAL raw /log/qso response. Test Connection only
        // ever exercises /login, never a real upload.
        let callsign = std::env::var("HAMQTH_CALLSIGN").unwrap();
        let password = std::env::var("HAMQTH_PASSWORD").unwrap();
        let mut r = rec();
        r.station_callsign = callsign.clone();
        r.call = std::env::var("HAMQTH_DX_CALL").unwrap();
        r.qso_date = std::env::var("HAMQTH_QSO_DATE").unwrap();
        r.time_on = std::env::var("HAMQTH_TIME_ON").unwrap();
        r.band = std::env::var("HAMQTH_BAND").unwrap();
        r.mode = std::env::var("HAMQTH_MODE").unwrap();
        r.freq = std::env::var("HAMQTH_FREQ").unwrap_or_default();
        r.gridsquare = std::env::var("HAMQTH_GRID").unwrap_or_default();
        let body = serde_json::json!({ "qso": qso_json(&r) }).to_string();
        eprintln!("request body: {body}");
        let token = ensure_token(&callsign, &password).unwrap();
        let resp = agent()
            .post(&format!("{BASE_URL}/log/qso"))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_string(&body);
        match resp {
            Ok(r) => eprintln!("status={} body={}", r.status(), r.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(code, r)) => eprintln!("status={} body={}", code, r.into_string().unwrap_or_default()),
            Err(ureq::Error::Transport(t)) => eprintln!("transport error: {}", t.kind()),
        }
    }
}
