//! Club Log QSO upload (clublog.org, run by G7VJR) -- **not to be confused
//! with `cloudlog.rs`** (Cloudlog/Wavelog/Nextlog, a different, unrelated
//! self-hosted logging platform; the two names are easy to mix up and this
//! file intentionally keeps its own module name distinct).
//!
//! Uses Club Log's modern per-user token API (`clublog.org/edituser.php?
//! tab=api` -> "Tokens"), confirmed live -- NOT the older `realtime.php`/
//! `putlogs.php` email+password+per-application-API-key system most
//! existing third-party documentation and forum posts describe, which is
//! a different, legacy mechanism:
//!
//! - `POST /api/v1/qso` -- one QSO, JSON body with Club Log's own field
//!   names (not raw ADIF tags): `call` (operator's own callsign, always
//!   required), `dxcall`, `datetime` (ISO-8601 UTC), `band` (metres, as a
//!   bare number string, e.g. `"20"` -- NOT `"20m"`), `mode` (ADIF mode),
//!   plus optional `freq` (MHz), `rst_sent`, `rst_rcvd`, `grid`.
//!   Authenticated via `Authorization: Bearer cl_...`.
//! - `POST /api/v1/qso/batch` -- `{"qsos": [...]}`, 1-100 of the same
//!   per-QSO objects, one `results` entry per item (each with its own
//!   `status`/`qsoid`) -- a rejected item doesn't undo the others.
//! - Response `status`: `"created"` or `"updated"` (success -- `updated`
//!   still counts, Club Log already had a matching QSO and merged this
//!   one into it), `"duplicate"` (also treated as success: Club Log
//!   already has this exact QSO, nothing to resend), anything else is a
//!   per-QSO rejection. `401` means the token itself is bad/revoked.
//!
//! **No dedicated "test connection" endpoint is documented.** Test
//! Connection instead POSTs an empty batch (`{"qsos": []}`): Club Log
//! validates the token before the body, so a bad token still answers 401,
//! while a good token answers 422 ("empty list") -- a real response
//! confirming the token works, without writing anything. This is inferred
//! from the documented request-validation order, not itself documented as
//! a connectivity check, so it's worth confirming live.
//!
//! **Live-found real issue, 2026-11-23 (account/token config, not a code
//! bug): a "Test mode" token uploads nothing.** The user reported real
//! QSOs this app had marked synced never actually appeared on their real
//! Club Log account. Live-tested a real single `/qso` upload (something
//! Test Connection's empty-batch probe never exercises) and found the
//! cause: Club Log's own Tokens page has a "Test mode" checkbox ("for
//! developers") -- a token created with it on returns a completely
//! normal-looking `"status":"created"` on every upload, but
//! `"test_mode":true` plus `"message":"Test mode: parameters ok, nothing
//! stored."` reveals nothing was actually written. [`item_outcome`] now
//! checks for `test_mode` before anything else, so this can never again
//! be silently counted as a successful upload -- the operator needs a
//! fresh, non-test-mode token from Club Log's Tokens page.

use serde::Serialize;

use crate::db::{Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableClublog";
pub const CONFIG_TOKEN: &str = "clublogToken";

const BASE_URL: &str = "https://clublog.org/api/v1";
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Max QSOs per `qso/batch` call (Club Log's own documented limit).
const MAX_BATCH_SIZE: usize = 100;
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
const MAX_RETRY_AFTER_S: u64 = 65;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(HTTP_CONNECT_TIMEOUT).timeout_read(HTTP_READ_TIMEOUT).build()
}

/// ADIF band ("20m", "70cm", ...) to Club Log's bare-number-of-metres
/// convention ("20", "0.7"). `None` for a band string that isn't one of
/// the two recognized unit suffixes.
fn band_to_metres(band: &str) -> Option<String> {
    let b = band.trim().to_lowercase();
    if let Some(m) = b.strip_suffix("cm") {
        let cm: f64 = m.parse().ok()?;
        let metres = cm / 100.0;
        return Some(format_trimmed(metres));
    }
    if let Some(m) = b.strip_suffix("mm") {
        let mm: f64 = m.parse().ok()?;
        let metres = mm / 1000.0;
        return Some(format_trimmed(metres));
    }
    let m = b.strip_suffix('m').unwrap_or(&b);
    let metres: f64 = m.parse().ok()?;
    Some(format_trimmed(metres))
}

/// Formats a metres value the way Club Log's own example does: whole
/// numbers with no decimal point, fractional values trimmed of trailing
/// zeros (so `0.70` -> `"0.7"`, `20.0` -> `"20"`).
fn format_trimmed(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `qso_date` (`YYYYMMDD`) + `time_on` (`HHMM` or `HHMMSS`) as Club Log's
/// required ISO-8601 UTC `datetime`. `None` for a malformed date/time.
fn qso_datetime_iso8601(qso_date: &str, time_on: &str) -> Option<String> {
    let d = qso_date.trim();
    if d.len() != 8 || !d.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut t: String = time_on.chars().filter(char::is_ascii_digit).collect();
    if t.len() % 2 == 1 {
        t.insert(0, '0');
    }
    if t.len() < 4 {
        return None;
    }
    t.truncate(6);
    while t.len() < 6 {
        t.push('0');
    }
    Some(format!("{}-{}-{}T{}:{}:{}Z", &d[0..4], &d[4..6], &d[6..8], &t[0..2], &t[2..4], &t[4..6]))
}

fn qso_json(r: &QsoRecord) -> Result<serde_json::Value, String> {
    let call = r.station_callsign.trim();
    if call.is_empty() {
        return Err("no operator callsign known".to_string());
    }
    let datetime = qso_datetime_iso8601(&r.qso_date, &r.time_on).ok_or("missing or malformed qso_date/time_on")?;
    let band = band_to_metres(&r.band).ok_or("missing or unrecognized band")?;
    if r.mode.trim().is_empty() {
        return Err("missing mode".to_string());
    }
    let mut o = serde_json::json!({
        "call": call.to_uppercase(),
        "dxcall": r.call.to_uppercase(),
        "datetime": datetime,
        "band": band,
        "mode": r.mode.to_uppercase(),
    });
    if let Ok(freq) = r.freq.trim().parse::<f64>() {
        o["freq"] = serde_json::json!(freq);
    }
    if !r.rst_sent.trim().is_empty() {
        o["rst_sent"] = serde_json::json!(r.rst_sent.trim());
    }
    if !r.rst_rcvd.trim().is_empty() {
        o["rst_rcvd"] = serde_json::json!(r.rst_rcvd.trim());
    }
    if !r.gridsquare.trim().is_empty() {
        o["grid"] = serde_json::json!(r.gridsquare.trim());
    }
    Ok(o)
}

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

struct Response {
    status: u16,
    body: String,
    retry_after: Option<String>,
}

fn post(token: &str, path: &str, json: &str) -> Result<Response, String> {
    let result = agent()
        .post(&format!("{BASE_URL}{path}"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(json);
    match result {
        Ok(r) => {
            let status = r.status();
            let retry_after = r.header("Retry-After").map(str::to_string);
            let body = r.into_string().unwrap_or_default();
            Ok(Response { status, body, retry_after })
        }
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response.header("Retry-After").map(str::to_string);
            let body = response.into_string().unwrap_or_default();
            Ok(Response { status, body, retry_after })
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

fn describe_failure(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| {
        v.get("message")
            .or_else(|| v.get("error").and_then(|e| e.get("message")))
            .and_then(|m| m.as_str())
            .map(str::to_string)
    });
    match message {
        Some(m) if !m.is_empty() => format!("HTTP {status}: {m}"),
        _ => format!("HTTP {status}"),
    }
}

/// Test Connection -- see this module's top doc comment for why an empty
/// batch call stands in for a dedicated check.
pub fn check_connection(token: &str) -> ConnectionCheck {
    if token.trim().is_empty() {
        return ConnectionCheck { ok: false, detail: Some("Enter your Club Log token first.".to_string()) };
    }
    match post(token.trim(), "/qso/batch", r#"{"qsos":[]}"#) {
        Ok(r) if r.status == 401 => {
            ConnectionCheck { ok: false, detail: Some("Club Log rejected the token.".to_string()) }
        }
        Ok(r) if r.status == 422 => ConnectionCheck { ok: true, detail: None },
        Ok(r) => ConnectionCheck { ok: false, detail: Some(describe_failure(r.status, &r.body)) },
        Err(e) => ConnectionCheck { ok: false, detail: Some(e) },
    }
}

enum ItemOutcome {
    Ok,
    AlreadyThere,
    Rejected(String),
    /// Live-found real issue, not a code bug: a token created with Club
    /// Log's own "Test mode" checkbox (under Tokens, "for developers")
    /// gets a normal-looking `"status":"created"` reply on every upload,
    /// but Club Log never actually stores anything --
    /// `"test_mode":true` plus a message like "Test mode: parameters ok,
    /// nothing stored." is the only tell. Treating this as a plain `Ok`
    /// (as this module used to) means every upload looks successful and
    /// gets marked synced, while the operator's real Club Log account
    /// never receives a single QSO -- confirmed live: the user's real
    /// account showed none of the QSOs this app had already marked
    /// uploaded. Must never be treated as success.
    TestMode,
}

fn item_outcome(entry: &serde_json::Value) -> ItemOutcome {
    if entry.get("test_mode").and_then(|v| v.as_bool()) == Some(true) {
        return ItemOutcome::TestMode;
    }
    match entry.get("status").and_then(|s| s.as_str()) {
        Some("created") | Some("updated") => ItemOutcome::Ok,
        Some("duplicate") => ItemOutcome::AlreadyThere,
        _ => {
            let msg = entry.get("message").and_then(|m| m.as_str()).unwrap_or("rejected");
            ItemOutcome::Rejected(msg.to_string())
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ClublogUploadResult {
    pub uploaded: usize,
    pub already_present: usize,
    pub failed: usize,
    pub error: Option<String>,
}

/// Sends one batch POST, retrying a 429 whose `Retry-After` fits the usual
/// short rate-limit window (Club Log's own daily QSO-write allowance is
/// separate and much larger, so this only ever waits out a short burst
/// limit, same shape as `wrl.rs`'s retry).
fn post_batch_with_retry(token: &str, json: &str) -> Result<Response, String> {
    for attempt in 0.. {
        let resp = post(token, "/qso/batch", json)?;
        if resp.status == 429 && attempt < MAX_RATE_LIMIT_RETRIES {
            if let Some(wait) = resp.retry_after.as_deref().and_then(|h| h.trim().parse::<u64>().ok()) {
                if wait <= MAX_RETRY_AFTER_S {
                    std::thread::sleep(std::time::Duration::from_secs(wait.max(1)));
                    continue;
                }
            }
        }
        return Ok(resp);
    }
    unreachable!()
}

/// Uploads every QSO not yet marked synced to Club Log (`synced_clublog =
/// 0`), in batches of up to 100 via `POST /qso/batch`. Stops early on a 401
/// (bad token -- every remaining row would fail the same way); an
/// individual rejected QSO within an otherwise-successful batch just moves
/// on to the next one.
pub fn upload_pending(db: &Db, token: &str) -> ClublogUploadResult {
    if token.trim().is_empty() {
        return ClublogUploadResult { error: Some("Enter your Club Log token first.".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_clublog_upload();
    let mut result = ClublogUploadResult::default();
    for chunk in pending.chunks(MAX_BATCH_SIZE) {
        let mut items = Vec::with_capacity(chunk.len());
        let mut skip_reasons = Vec::new();
        for q in chunk {
            match qso_json(q) {
                Ok(j) => items.push((q, j)),
                Err(e) => skip_reasons.push((q, e)),
            }
        }
        for (q, e) in skip_reasons {
            let _ = q;
            if result.error.is_none() {
                result.error = Some(e);
            }
            result.failed += 1;
        }
        if items.is_empty() {
            continue;
        }
        let body = serde_json::json!({ "qsos": items.iter().map(|(_, j)| j.clone()).collect::<Vec<_>>() }).to_string();
        let resp = match post_batch_with_retry(token.trim(), &body) {
            Ok(r) => r,
            Err(e) => {
                result.error.get_or_insert(e);
                result.failed += items.len();
                break;
            }
        };
        if resp.status == 401 {
            result.error = Some("Club Log rejected the token.".to_string());
            result.failed += items.len();
            break;
        }
        let results: Vec<serde_json::Value> = serde_json::from_str::<serde_json::Value>(&resp.body)
            .ok()
            .and_then(|v| v.get("results").cloned())
            .and_then(|r| r.as_array().cloned())
            .unwrap_or_default();
        for (i, (q, _)) in items.iter().enumerate() {
            match results.get(i).map(item_outcome) {
                Some(ItemOutcome::Ok) => {
                    result.uploaded += 1;
                    if let Some(id) = q.id {
                        let _ = db.mark_synced(id, "clublog");
                    }
                }
                Some(ItemOutcome::AlreadyThere) => {
                    result.already_present += 1;
                    if let Some(id) = q.id {
                        let _ = db.mark_synced(id, "clublog");
                    }
                }
                Some(ItemOutcome::Rejected(msg)) => {
                    result.error.get_or_insert(msg);
                    result.failed += 1;
                }
                Some(ItemOutcome::TestMode) => {
                    // See ItemOutcome::TestMode's own doc comment -- never
                    // mark synced, so this keeps retrying (and the
                    // operator keeps seeing this message) until they
                    // switch to a non-test-mode token.
                    result.error.get_or_insert_with(|| {
                        "This Club Log token is in Test Mode -- uploads are validated but never actually stored. Create a new token with Test Mode OFF.".to_string()
                    });
                    result.failed += 1;
                }
                None => {
                    result.error.get_or_insert_with(|| "Club Log returned an unexpected batch reply.".to_string());
                    result.failed += 1;
                }
            }
        }
    }
    result
}

/// Auto-upload hook for a QSO that was just logged -- a single-item
/// `POST /qso`, matching the real-time intent `qso/batch` is documented as
/// not being for. Callers (engine.rs) run this on a spawned thread, never
/// the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, token: &str, q: &QsoRecord) {
    if token.trim().is_empty() {
        return;
    }
    let Ok(json) = qso_json(q) else { return };
    let Ok(resp) = post(token.trim(), "/qso", &json.to_string()) else { return };
    if matches!(item_outcome(&serde_json::from_str(&resp.body).unwrap_or_default()), ItemOutcome::Ok | ItemOutcome::AlreadyThere)
    {
        if let Some(id) = q.id {
            let _ = db.mark_synced(id, "clublog");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> QsoRecord {
        QsoRecord {
            call: "k1abc".to_string(),
            qso_date: "20261122".to_string(),
            time_on: "1230".to_string(),
            band: "20m".to_string(),
            mode: "ft8".to_string(),
            freq: "14.074".to_string(),
            rst_sent: "-10".to_string(),
            rst_rcvd: "-05".to_string(),
            gridsquare: "FN31".to_string(),
            station_callsign: "n0call".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn band_to_metres_strips_the_m_suffix() {
        assert_eq!(band_to_metres("20m"), Some("20".to_string()));
        assert_eq!(band_to_metres("160m"), Some("160".to_string()));
        assert_eq!(band_to_metres("1.25m"), Some("1.25".to_string()));
    }

    #[test]
    fn band_to_metres_converts_centimetre_and_millimetre_bands() {
        assert_eq!(band_to_metres("70cm"), Some("0.7".to_string()));
        assert_eq!(band_to_metres("23cm"), Some("0.23".to_string()));
    }

    #[test]
    fn band_to_metres_rejects_garbage() {
        assert_eq!(band_to_metres(""), None);
        assert_eq!(band_to_metres("notaband"), None);
    }

    #[test]
    fn qso_datetime_iso8601_combines_date_and_time() {
        assert_eq!(qso_datetime_iso8601("20261122", "1230"), Some("2026-11-22T12:30:00Z".to_string()));
        assert_eq!(qso_datetime_iso8601("20261122", "123045"), Some("2026-11-22T12:30:45Z".to_string()));
        assert_eq!(qso_datetime_iso8601("20261122", "815"), Some("2026-11-22T08:15:00Z".to_string()));
    }

    #[test]
    fn qso_datetime_iso8601_rejects_malformed_input() {
        assert_eq!(qso_datetime_iso8601("2026112", "1230"), None);
        assert_eq!(qso_datetime_iso8601("20261122", "12"), None);
        assert_eq!(qso_datetime_iso8601("", ""), None);
    }

    #[test]
    fn qso_json_maps_dxcall_and_uppercases_calls_and_mode() {
        let j = qso_json(&rec()).unwrap();
        assert_eq!(j["call"], "N0CALL");
        assert_eq!(j["dxcall"], "K1ABC");
        assert_eq!(j["mode"], "FT8");
        assert_eq!(j["band"], "20");
        assert_eq!(j["grid"], "FN31");
    }

    #[test]
    fn qso_json_requires_an_operator_callsign() {
        let mut r = rec();
        r.station_callsign = "".to_string();
        assert!(qso_json(&r).is_err());
    }

    #[test]
    fn qso_json_requires_a_recognizable_band_and_mode() {
        let mut r = rec();
        r.band = "".to_string();
        assert!(qso_json(&r).is_err());
        let mut r = rec();
        r.mode = "".to_string();
        assert!(qso_json(&r).is_err());
    }

    #[test]
    fn item_outcome_treats_created_updated_and_duplicate_as_success_like() {
        assert!(matches!(item_outcome(&serde_json::json!({"status": "created"})), ItemOutcome::Ok));
        assert!(matches!(item_outcome(&serde_json::json!({"status": "updated"})), ItemOutcome::Ok));
        assert!(matches!(item_outcome(&serde_json::json!({"status": "duplicate"})), ItemOutcome::AlreadyThere));
        assert!(matches!(item_outcome(&serde_json::json!({"status": "rejected", "message": "bad band"})), ItemOutcome::Rejected(_)));
    }

    #[test]
    fn item_outcome_never_treats_a_test_mode_token_reply_as_success() {
        // Real live bug: a Club Log token created with "Test mode" on
        // returns a completely normal-looking "created" status on every
        // upload, but nothing is ever actually stored -- confirmed with
        // the user's real token ({"status":"created","qsoid":null,
        // "test_mode":true,"message":"Test mode: parameters ok, nothing
        // stored."}). This must NEVER match Ok/AlreadyThere regardless of
        // what "status" says.
        let reply = serde_json::json!({
            "status": "created",
            "qsoid": null,
            "test_mode": true,
            "message": "Test mode: parameters ok, nothing stored."
        });
        assert!(matches!(item_outcome(&reply), ItemOutcome::TestMode));
    }

    #[test]
    fn describe_failure_prefers_the_message_field() {
        assert_eq!(describe_failure(422, r#"{"message":"malformed datetime"}"#), "HTTP 422: malformed datetime");
        assert_eq!(describe_failure(500, ""), "HTTP 500");
    }

    #[test]
    fn check_connection_requires_a_token() {
        assert!(!check_connection("").ok);
    }

    #[test]
    fn upload_pending_is_a_no_op_without_a_token() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "");
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "fake-token-not-used-because-nothing-pending");
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }

    #[test]
    #[ignore]
    fn scratch_live_check_connection() {
        // Not run in CI (needs a real token via env var) -- manual
        // diagnostic hook for this module's own live Club Log account,
        // matching hrdlog.rs's own scratch_live_check_connection.
        let token = std::env::var("CLUBLOG_TOKEN").unwrap();
        let check = check_connection(&token);
        eprintln!("ok={} detail={:?}", check.ok, check.detail);
    }

    #[test]
    #[ignore]
    fn scratch_live_single_qso_upload() {
        // Not run in CI -- a real, one-off live upload of ONE real QSO
        // (fields via env vars, no secrets hardcoded) to see the ACTUAL
        // raw /qso response shape. Test Connection only ever validated an
        // empty batch; the real single-QSO response has never actually
        // been observed live.
        let token = std::env::var("CLUBLOG_TOKEN").unwrap();
        let mut r = rec();
        r.station_callsign = std::env::var("CLUBLOG_MY_CALL").unwrap();
        r.call = std::env::var("CLUBLOG_DX_CALL").unwrap();
        r.qso_date = std::env::var("CLUBLOG_QSO_DATE").unwrap();
        r.time_on = std::env::var("CLUBLOG_TIME_ON").unwrap();
        r.band = std::env::var("CLUBLOG_BAND").unwrap();
        r.mode = std::env::var("CLUBLOG_MODE").unwrap();
        r.freq = std::env::var("CLUBLOG_FREQ").unwrap_or_default();
        r.gridsquare = std::env::var("CLUBLOG_GRID").unwrap_or_default();
        let json = qso_json(&r).unwrap();
        eprintln!("request body: {json}");
        let resp = post(&token, "/qso", &json.to_string()).unwrap();
        eprintln!("status={} body={}", resp.status, resp.body);
    }
}
