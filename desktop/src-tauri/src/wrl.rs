//! World Radio League logbook integration, ported from Android's
//! `WrlApi.java` (issue #800 there). WRL is a fixed cloud service at
//! `https://api.worldradioleague.com/v1/` -- no server address to
//! configure, unlike Cloudlog. The operator generates one account-wide key
//! under Integrations -> Developer API, sent as `Authorization: Bearer`.
//! Contacts are JSON with camelCased ADIF-ish field names, one per
//! `POST /v1/contacts`, and unknown fields are *rejected* rather than
//! ignored, so the payload is built field-by-field here rather than reusing
//! the general ADIF string Cloudlog/QRZ get.
//!
//! Same three pieces as `cloudlog.rs`: test connection + logbook picker
//! (plus "create a logbook" -- a brand-new WRL account starts with none, and
//! WRL refuses every contact until it has one), manual/startup catch-up
//! upload, and auto-upload on log.
//!
//! **Disclosed gap**: WRL's `myActivities`/`theirActivities` (POTA/SOTA/
//! WWFF/IOTA references, ADIF `SIG`/`SIG_INFO`/`MY_SIG`/`MY_SIG_INFO`) have
//! no data source on desktop -- there is no POTA/SIG tracking anywhere on a
//! logged QSO yet (same gap `logged_state.rs`'s own doc comment already
//! flags). [`activities`] is ported in full and unit-tested for when that
//! data exists, but [`ContactFields::from_record`] always passes `None` for
//! both pairs today.
//!
//! A 429 (rate limited) with a short `Retry-After` is waited out and
//! retried in-process -- a catch-up upload of a real backlog runs straight
//! into WRL's 60-writes/minute window.

use std::time::Duration;

use serde::Serialize;

use crate::cloudlog::StationProfile;
use crate::db::{Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableWRL";
pub const CONFIG_API_KEY: &str = "wrlApiKey";
pub const CONFIG_LOGBOOK_ID: &str = "wrlLogbookId";

const BASE_URL: &str = "https://api.worldradioleague.com/v1/";
const PROGRAM_ID: &str = "FT8AF";
pub const DEFAULT_LOGBOOK_NAME: &str = "FT8AF";

const MAX_ACTIVITIES: usize = 7;
const MAX_NOTES_LEN: usize = 2000;
const MAX_RST_LEN: usize = 10;
/// Longest `Retry-After` a sync pass sits out: the per-minute write window
/// (60 writes/min). A longer value means the daily quota is gone and
/// waiting in-process is pointless.
const MAX_RETRY_AFTER_S: u64 = 65;
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
const MAX_FAILURE_LEN: usize = 200;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(20);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(HTTP_CONNECT_TIMEOUT).timeout_read(HTTP_READ_TIMEOUT).build()
}

/// Outcome of one contact upload.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum UploadOutcome {
    Ok,
    /// This contact was refused (a bad field, a server hiccup); the next one may still go.
    Failed,
    /// Every following upload would fail the same way -- bad/revoked key, no
    /// usable logbook, quota exhausted, offline -- so a batch should stop.
    FailedStopBatch,
}

/// The QSO fields WRL accepts, as plain strings/options, whichever source
/// they came from.
#[derive(Debug, Default, Clone)]
struct ContactFields {
    call: String,
    qso_date: String,
    time_on: String,
    freq_mhz: String,
    band: String,
    mode: String,
    rst_sent: String,
    rst_rcvd: String,
    gridsquare: String,
    my_gridsquare: String,
    station_callsign: String,
    notes: String,
    /// POTA/SOTA/WWFF/IOTA the *operator* is activating -- see this
    /// module's top doc comment; always `None` on desktop today.
    my_sig: Option<(String, String)>,
    /// Same, for the *other* station's activation.
    sig: Option<(String, String)>,
}

impl ContactFields {
    /// Fields for the immediate post-QSO upload / catch-up sync. Desktop
    /// stores `freq` already in MHz (unlike Android's `BandFreq`, which is
    /// Hz and needs converting) -- see `db.rs`'s `QsoRecord::freq` doc comment.
    fn from_record(r: &QsoRecord) -> Self {
        ContactFields {
            call: r.call.clone(),
            qso_date: r.qso_date.clone(),
            time_on: r.time_on.clone(),
            freq_mhz: r.freq.clone(),
            band: r.band.clone(),
            mode: r.mode.clone(),
            rst_sent: r.rst_sent.clone(),
            rst_rcvd: r.rst_rcvd.clone(),
            gridsquare: r.gridsquare.clone(),
            my_gridsquare: r.my_gridsquare.clone(),
            station_callsign: r.station_callsign.clone(),
            notes: r.comment.clone(),
            my_sig: None,
            sig: None,
        }
    }
}

fn trim_to_none(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

fn digits_only(s: &str) -> Option<String> {
    let d: String = s.chars().filter(char::is_ascii_digit).collect();
    (!d.is_empty()).then_some(d)
}

/// ADIF `TIME_ON` as WRL's HHMM/HHMMSS, restoring a dropped leading zero
/// ("815" -> "0815").
fn normalize_time_on(raw: &str) -> Option<String> {
    let mut d = digits_only(raw)?;
    if d.len() == 3 || d.len() == 5 {
        d.insert(0, '0');
    }
    (d.len() == 4 || d.len() == 6).then_some(d)
}

/// Frequency in MHz, or `None` when absent/unparseable/non-positive. A value
/// of 100000 or more is taken as Hz -- no amateur band sits there in MHz,
/// and some imported rows carry Hz.
fn parse_freq_mhz(raw: &str) -> Option<f64> {
    let s = trim_to_none(raw)?;
    let v: f64 = s.parse().ok()?;
    if !(v > 0.0) || v.is_infinite() {
        return None;
    }
    Some(if v >= 100_000.0 { v / 1_000_000.0 } else { v })
}

/// Maidenhead grid syntax (2/4/6/8 chars, WRL's own pattern) -- a grid it
/// would refuse must not sink the whole contact, so an off-pattern value is
/// dropped rather than sent.
fn is_valid_grid(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let field = |ch: char, max: char| ch.is_ascii_alphabetic() && ('A'..=max).contains(&ch.to_ascii_uppercase());
    match c.len() {
        2 => field(c[0], 'R') && field(c[1], 'R'),
        4 => field(c[0], 'R') && field(c[1], 'R') && c[2].is_ascii_digit() && c[3].is_ascii_digit(),
        6 => {
            field(c[0], 'R')
                && field(c[1], 'R')
                && c[2].is_ascii_digit()
                && c[3].is_ascii_digit()
                && field(c[4], 'X')
                && field(c[5], 'X')
        }
        8 => {
            field(c[0], 'R')
                && field(c[1], 'R')
                && c[2].is_ascii_digit()
                && c[3].is_ascii_digit()
                && field(c[4], 'X')
                && field(c[5], 'X')
                && c[6].is_ascii_digit()
                && c[7].is_ascii_digit()
        }
        _ => false,
    }
}

const ACTIVITY_TYPES: &[&str] = &["POTA", "SOTA", "WWFF", "IOTA"];

/// ADIF `SIG`/`SIG_INFO` as a WRL activity array. WRL takes only the array
/// form, one entry per reference, so a two-fer stored as `"US-0001,US-0002"`
/// becomes two entries with the first marked primary. `None` for no refs or
/// a programme WRL does not track.
fn activities(sig: Option<&str>, sig_info: Option<&str>) -> Option<serde_json::Value> {
    let ty = trim_to_none(sig?)?.to_uppercase();
    let refs = trim_to_none(sig_info?)?;
    if !ACTIVITY_TYPES.contains(&ty.as_str()) {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for part in refs.split(|c: char| c == ',' || c.is_whitespace()) {
        let r = part.trim().to_uppercase();
        if r.is_empty() || !seen.insert(r.clone()) {
            continue;
        }
        let mut entry = serde_json::json!({ "type": ty, "ref": r });
        if out.is_empty() {
            entry["isPrimary"] = serde_json::json!(true);
        }
        out.push(entry);
        if out.len() == MAX_ACTIVITIES {
            break;
        }
    }
    (!out.is_empty()).then(|| serde_json::Value::Array(out))
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect()
    } else {
        s.to_string()
    }
}

/// Builds the `POST /v1/contacts` body, or `Err` (with the reason) when a
/// field WRL requires is missing or malformed -- sending it would only earn
/// a 400/422. Optional fields WRL would reject (an off-pattern grid, an
/// over-long report) are dropped or clipped instead, so one bad detail
/// never loses the whole contact.
fn build_contact_json(f: &ContactFields, logbook_id: Option<&str>) -> Result<String, String> {
    let call = trim_to_none(&f.call).ok_or("missing call")?;
    let date = digits_only(&f.qso_date).filter(|d| d.len() == 8).ok_or("missing or malformed qso_date")?;
    let time = normalize_time_on(&f.time_on).ok_or("missing or malformed time_on")?;
    let freq = parse_freq_mhz(&f.freq_mhz).ok_or("missing frequency")?;
    let band = trim_to_none(&f.band).ok_or("missing band")?;
    let mode = trim_to_none(&f.mode).ok_or("missing mode")?;

    let mut o = serde_json::json!({
        "programId": PROGRAM_ID,
        "call": call.to_uppercase(),
        "timestamp": { "qsoDate": date, "timeOn": time },
        "freq": freq,
        "band": band.to_lowercase(),
        // WRL stores one mode field and wants the submode itself ("FT4"),
        // not MFSK+submode.
        "mode": mode.to_uppercase(),
    });
    if let Some(logbook) = logbook_id.and_then(trim_to_none) {
        o["logbookId"] = serde_json::json!(logbook);
    }
    if let Some(rst) = trim_to_none(&f.rst_sent) {
        o["rstSent"] = serde_json::json!(clip(rst, MAX_RST_LEN));
    }
    if let Some(rst) = trim_to_none(&f.rst_rcvd) {
        o["rstRcvd"] = serde_json::json!(clip(rst, MAX_RST_LEN));
    }
    if let Some(g) = trim_to_none(&f.gridsquare).filter(|g| is_valid_grid(g)) {
        o["gridsquare"] = serde_json::json!(g);
    }
    if let Some(g) = trim_to_none(&f.my_gridsquare).filter(|g| is_valid_grid(g)) {
        o["myGridsquare"] = serde_json::json!(g);
    }
    if let Some(station) = trim_to_none(&f.station_callsign) {
        o["stationCallsign"] = serde_json::json!(station.to_uppercase());
    }
    if let Some(notes) = trim_to_none(&f.notes) {
        o["notes"] = serde_json::json!(clip(notes, MAX_NOTES_LEN));
    }
    if let Some((sig, info)) = &f.my_sig {
        if let Some(mine) = activities(Some(sig), Some(info)) {
            o["myActivities"] = mine;
        }
    }
    if let Some((sig, info)) = &f.sig {
        if let Some(theirs) = activities(Some(sig), Some(info)) {
            o["theirActivities"] = theirs;
        }
    }
    Ok(o.to_string())
}

/// One line for debug output and the settings dialog: status, WRL's stable
/// error code, its message and the offending field -- e.g. `HTTP 401
/// INVALID_KEY: The API key is not valid.` Falls back to the flattened body
/// for a non-WRL reply (a proxy error page).
fn describe_failure(status: u16, body: &str) -> String {
    let err = error_object(body);
    let mut out = format!("HTTP {status}");
    if let Some(err) = &err {
        if let Some(code) = err.get("code").and_then(|v| v.as_str()) {
            out.push(' ');
            out.push_str(code);
        }
        if let Some(message) = err.get("message").and_then(|v| v.as_str()) {
            out.push_str(": ");
            out.push_str(message);
        }
        if let Some(field) = err.get("field").and_then(|v| v.as_str()) {
            out.push_str(" [");
            out.push_str(field);
            out.push(']');
        }
    } else {
        let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        if !flat.is_empty() {
            out.push_str(": ");
            out.push_str(&flat);
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_FAILURE_LEN {
        let clipped: String = out.chars().take(MAX_FAILURE_LEN).collect();
        format!("{clipped}…")
    } else {
        out
    }
}

/// Whether a refused upload means every later one in the batch would be
/// refused too, so the sync should stop calling WRL: an auth/tier problem,
/// a destination logbook that cannot take contacts, the API disabled, or a
/// rate limit we will not wait out. A 5xx stops too: an outage fails every
/// row alike, and live WRL answers an unroutable contact (no logbookId, no
/// resolvable default) with 500 `INTERNAL_ERROR` rather than 422
/// `LOGBOOK_REQUIRED`. A validation error is specific to that contact, so
/// it does not stop.
fn is_batch_fatal(status: u16, body: &str) -> bool {
    if status == 401 || status == 403 || status == 429 || status >= 500 {
        return true;
    }
    let err = error_object(body);
    let code = err.as_ref().and_then(|e| e.get("code")).and_then(|v| v.as_str());
    let field = err.as_ref().and_then(|e| e.get("field")).and_then(|v| v.as_str());
    if matches!(code, Some("LOGBOOK_REQUIRED") | Some("API_DISABLED") | Some("RATE_LIMITED")) {
        return true;
    }
    (status == 404 || status == 409) && field == Some("logbookId")
}

/// `Retry-After` in seconds, or `None` when absent or not the delta-seconds form.
fn parse_retry_after_seconds(header: Option<&str>) -> Option<u64> {
    header.and_then(|h| h.trim().parse::<u64>().ok())
}

fn error_object(body: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(body).ok()?.get("error").cloned()
}

fn data_object(body: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(body).ok()?.get("data").cloned()
}

struct Response {
    status: u16,
    body: String,
    retry_after: Option<String>,
}

fn http_call(method: &str, url: &str, api_key: &str, json_body: Option<&str>) -> Result<Response, String> {
    let agent = agent();
    let mut req = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        _ => unreachable!("wrl transport only sends GET or POST"),
    };
    req = req.set("Authorization", &format!("Bearer {api_key}")).set("Accept", "application/json").set(
        "User-Agent",
        PROGRAM_ID,
    );
    let result = match json_body {
        Some(body) => req.set("Content-Type", "application/json").send_string(body),
        None => req.call(),
    };
    match result {
        Ok(r) => {
            let retry_after = r.header("Retry-After").map(str::to_string);
            let body = r.into_string().unwrap_or_default();
            Ok(Response { status: 200, body, retry_after })
        }
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response.header("Retry-After").map(str::to_string);
            let body = response.into_string().unwrap_or_default();
            Ok(Response { status, body, retry_after })
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

/// POSTs one contact. A 429 whose `Retry-After` fits the per-minute window
/// is waited out and retried (a catch-up upload of a big backlog runs
/// straight into 60 writes/min); anything else is reported once.
fn upload_contact(api_key: &str, json: &str) -> (UploadOutcome, Option<String>) {
    for attempt in 0.. {
        let resp = match http_call("POST", &format!("{BASE_URL}contacts"), api_key, Some(json)) {
            Ok(r) => r,
            Err(e) => return (UploadOutcome::FailedStopBatch, Some(e)),
        };
        if resp.status == 200 || resp.status == 201 {
            return (UploadOutcome::Ok, None);
        }
        if resp.status == 429 && attempt < MAX_RATE_LIMIT_RETRIES {
            if let Some(wait) = parse_retry_after_seconds(resp.retry_after.as_deref()) {
                if wait <= MAX_RETRY_AFTER_S {
                    std::thread::sleep(Duration::from_secs(wait.max(1)));
                    continue;
                }
            }
        }
        let why = describe_failure(resp.status, &resp.body);
        let outcome = if is_batch_fatal(resp.status, &resp.body) {
            UploadOutcome::FailedStopBatch
        } else {
            UploadOutcome::Failed
        };
        return (outcome, Some(why));
    }
    unreachable!()
}

/// The account's only logbook, else `None` -- upload-time fallback when no
/// logbook is saved. With several, the choice is the operator's (or a
/// default they set on the website).
fn choose_upload_logbook(books: &[StationProfile]) -> Option<String> {
    (books.len() == 1).then(|| books[0].station_id.clone())
}

fn upload_fields(db: &Db, api_key: &str, f: &ContactFields) -> Result<(), String> {
    let mut logbook = trim_to_none(&db.get_config(CONFIG_LOGBOOK_ID).unwrap_or_default()).map(str::to_string);
    if logbook.is_none() {
        // No logbook saved. WRL will not route a contact to a lone logbook
        // on its own (live: 500 "Could not determine the destination
        // logbook"), and a new account has none at all -- so look before
        // posting.
        match fetch_logbooks_or_none(api_key) {
            Some(books) if books.is_empty() => {
                return Err("no logbook in the World Radio League account: create one in Logging settings".to_string());
            }
            Some(books) => {
                logbook = choose_upload_logbook(&books);
                if let Some(chosen) = &logbook {
                    let _ = db.set_config(CONFIG_LOGBOOK_ID, chosen);
                }
            }
            None => {}
        }
    }
    let json = build_contact_json(f, logbook.as_deref())?;
    let (outcome, why) = upload_contact(api_key, &json);
    match outcome {
        UploadOutcome::Ok => Ok(()),
        _ => Err(why.unwrap_or_else(|| "World Radio League rejected the upload.".to_string())),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct WrlUploadResult {
    pub uploaded: usize,
    pub failed: usize,
    /// The *first* failure's message, if any -- see this module's top doc
    /// comment on `is_batch_fatal`: a fatal rejection stops the batch, so a
    /// later row having `failed` incremented too means every remaining row
    /// was skipped for that same reason, not retried and failed again.
    pub error: Option<String>,
}

/// Uploads every QSO not yet marked synced to World Radio League
/// (`synced_wrl = 0`). Stops early once a batch-fatal rejection is seen
/// (see [`is_batch_fatal`]); the rest stay queued for the next run.
pub fn upload_pending(db: &Db) -> WrlUploadResult {
    let api_key = db.get_config(CONFIG_API_KEY).unwrap_or_default();
    if trim_to_none(&api_key).is_none() {
        return WrlUploadResult { error: Some("Enter your World Radio League API key first.".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_wrl_upload();
    let mut result = WrlUploadResult::default();
    for q in &pending {
        let fields = ContactFields::from_record(q);
        match upload_fields(db, api_key.trim(), &fields) {
            Ok(()) => {
                result.uploaded += 1;
                if let Some(id) = q.id {
                    let _ = db.mark_synced(id, "wrl");
                }
            }
            Err(e) => {
                let stops_batch = is_wrl_fatal_message(&e);
                if result.error.is_none() || stops_batch {
                    result.error = Some(e);
                }
                result.failed += 1;
                if stops_batch {
                    break;
                }
            }
        }
    }
    result
}

/// Whether an error string from [`upload_fields`] represents a batch-fatal
/// condition (vs. a per-contact validation problem). `upload_fields` itself
/// already folds `is_batch_fatal`'s verdict into [`UploadOutcome`], but that
/// distinction doesn't survive past its `Result<(), String>` return --
/// this re-derives it from the message shape alone (no network call), good
/// enough to decide whether a catch-up batch should keep going.
fn is_wrl_fatal_message(message: &str) -> bool {
    message.starts_with("no logbook in the World Radio League account")
        || message.starts_with("HTTP 401")
        || message.starts_with("HTTP 403")
        || message.starts_with("HTTP 429")
        || message.starts_with("HTTP 5")
}

/// Auto-upload hook for a QSO that was just logged -- same per-row logic as
/// [`upload_pending`], for exactly one record. Callers (engine.rs) run this
/// on a spawned thread, never the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, api_key: &str, q: &QsoRecord) {
    if trim_to_none(api_key).is_none() {
        return;
    }
    let fields = ContactFields::from_record(q);
    if upload_fields(db, api_key.trim(), &fields).is_ok() {
        if let Some(id) = q.id {
            let _ = db.mark_synced(id, "wrl");
        }
    }
}

// --- Test connection + logbook picker --------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

fn to_profile(lb: &serde_json::Value) -> Option<StationProfile> {
    let id = lb.get("id").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())?;
    let name = lb.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    let call = lb.get("defaultCallSign").and_then(|v| v.as_str()).unwrap_or_default();
    Some(StationProfile {
        station_id: id.to_string(),
        profile_name: name.to_string(),
        callsign: call.to_string(),
        gridsquare: String::new(),
    })
}

/// Parses a `GET /v1/logbooks` page into station profiles (id, name,
/// default callsign). Locked logbooks are skipped: WRL refuses new contacts
/// into them. `None` for an unreadable body (so a genuinely-empty account
/// is distinguishable from a failed read); only a well-formed
/// `{"data": [...]}` counts as read.
fn parse_logbooks(body: &str) -> Option<Vec<StationProfile>> {
    if body.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = value.get("data")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|lb| {
                let locked = lb.get("isLocked").and_then(|v| v.as_bool()).unwrap_or(false);
                if locked {
                    return None;
                }
                to_profile(lb)
            })
            .collect(),
    )
}

/// The account's logbooks that can accept contacts, or `None` when the list
/// could not be read (no key, bad key, offline) -- so an account that
/// genuinely has no logbooks is distinguishable from a failed request.
pub fn fetch_logbooks_or_none(api_key: &str) -> Option<Vec<StationProfile>> {
    let key = trim_to_none(api_key)?;
    let resp = http_call("GET", &format!("{BASE_URL}logbooks?limit=100"), key, None).ok()?;
    if resp.status == 200 {
        parse_logbooks(&resp.body)
    } else {
        None
    }
}

fn parse_created_logbook(body: &str) -> Option<StationProfile> {
    to_profile(&data_object(body)?)
}

fn build_logbook_create_json(name: &str, default_call_sign: Option<&str>) -> Option<String> {
    let n = trim_to_none(name)?;
    let mut o = serde_json::json!({ "name": clip(n, 100) });
    if let Some(call) = default_call_sign.and_then(trim_to_none).filter(|c| c.len() <= 20) {
        o["defaultCallSign"] = serde_json::json!(call.to_uppercase());
    }
    Some(o.to_string())
}

/// Creates a logbook (`POST /v1/logbooks`). WRL gives a new account no
/// logbook, so without one there is nowhere to put contacts.
pub fn create_logbook(api_key: &str, name: &str, default_call_sign: Option<&str>) -> Result<StationProfile, String> {
    let key = trim_to_none(api_key).ok_or("no API key configured")?;
    let json = build_logbook_create_json(name, default_call_sign).ok_or("missing logbook name")?;
    let resp = http_call("POST", &format!("{BASE_URL}logbooks"), key, Some(&json)).map_err(|e| e)?;
    if resp.status == 200 || resp.status == 201 {
        parse_created_logbook(&resp.body).ok_or_else(|| "unexpected reply (no logbook id)".to_string())
    } else {
        Err(describe_failure(resp.status, &resp.body))
    }
}

/// Interprets a `GET /v1/me` body.
fn interpret_me(body: &str, logbook_id: Option<&str>) -> ConnectionCheck {
    let Some(data) = data_object(body) else {
        return ConnectionCheck { ok: false, detail: Some("unexpected reply (not the World Radio League API?)".to_string()) };
    };
    if logbook_id.and_then(trim_to_none).is_none() {
        let def = data.get("defaultLogbook");
        let resolution = def.and_then(|d| d.get("resolution")).and_then(|v| v.as_str());
        if resolution == Some("none") {
            return ConnectionCheck { ok: false, detail: Some("no unlocked logbook: create one in World Radio League".to_string()) };
        }
        // With no logbookId a contact goes wherever defaultLogbook.logbookId
        // points. Live WRL can report {logbookId:null, resolution:null} for
        // an account whose only logbook was made in the web UI, then answer
        // every contact with 500 "Could not determine the destination
        // logbook" -- so a null destination fails the check whatever the
        // resolution says ("ambiguous" included).
        let has_logbook_id = def
            .and_then(|d| d.get("logbookId"))
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !has_logbook_id {
            return ConnectionCheck { ok: false, detail: Some("no default logbook: choose a logbook".to_string()) };
        }
    }
    ConnectionCheck { ok: true, detail: None }
}

/// Test Connection's verdict on a chosen `logbook_id`: it passes only when
/// that id is among the account's unlocked logbooks. `books` is
/// [`fetch_logbooks_or_none`]'s result, so `None` (list unreadable) fails too.
fn interpret_logbook_choice(books: Option<&[StationProfile]>, logbook_id: &str) -> ConnectionCheck {
    let Some(books) = books else {
        return ConnectionCheck { ok: false, detail: Some("could not read the account's logbooks".to_string()) };
    };
    let chosen = match trim_to_none(logbook_id) {
        Some(c) => c,
        None => return ConnectionCheck { ok: false, detail: Some("chosen logbook not found or locked: choose a logbook".to_string()) },
    };
    if books.iter().any(|b| b.station_id == chosen) {
        ConnectionCheck { ok: true, detail: None }
    } else {
        ConnectionCheck { ok: false, detail: Some("chosen logbook not found or locked: choose a logbook".to_string()) }
    }
}

/// Test Connection: `GET /v1/me` confirms the key without writing anything,
/// and its `defaultLogbook` tells us up front whether a contact with no
/// `logbookId` would be refused. If a logbook was chosen, it must also still
/// exist and accept contacts -- a deleted, foreign or locked id passes
/// `/me` but every contact POST to it would then fail.
pub fn check_connection(api_key: &str, logbook_id: &str) -> ConnectionCheck {
    let Some(key) = trim_to_none(api_key) else {
        return ConnectionCheck { ok: false, detail: Some("no API key configured".to_string()) };
    };
    let resp = match http_call("GET", &format!("{BASE_URL}me"), key, None) {
        Ok(r) => r,
        Err(e) => return ConnectionCheck { ok: false, detail: Some(e) },
    };
    if resp.status != 200 {
        return ConnectionCheck { ok: false, detail: Some(describe_failure(resp.status, &resp.body)) };
    }
    let me = interpret_me(&resp.body, Some(logbook_id));
    let Some(chosen) = trim_to_none(logbook_id) else {
        return me;
    };
    if !me.ok {
        return me;
    }
    interpret_logbook_choice(fetch_logbooks_or_none(key).as_deref(), chosen)
}

/// Picker label for a logbook: its name plus default callsign -- the id
/// means nothing to a person.
pub fn logbook_label(p: &StationProfile) -> String {
    let base = trim_to_none(&p.profile_name).unwrap_or(&p.station_id);
    match trim_to_none(&p.callsign) {
        Some(call) => format!("{base} ({call})"),
        None => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> ContactFields {
        ContactFields {
            call: "K1ABC".to_string(),
            qso_date: "20261122".to_string(),
            time_on: "1230".to_string(),
            freq_mhz: "14.074".to_string(),
            band: "20m".to_string(),
            mode: "FT8".to_string(),
            rst_sent: "-10".to_string(),
            rst_rcvd: "-05".to_string(),
            gridsquare: "FN31pr".to_string(),
            my_gridsquare: "EM20".to_string(),
            station_callsign: "N0CALL".to_string(),
            notes: "QSO by FT8AF".to_string(),
            my_sig: None,
            sig: None,
        }
    }

    #[test]
    fn normalize_time_on_restores_a_dropped_leading_zero() {
        assert_eq!(normalize_time_on("815"), Some("0815".to_string()));
        assert_eq!(normalize_time_on("81505"), Some("081505".to_string()));
        assert_eq!(normalize_time_on("1230"), Some("1230".to_string()));
        // Same leading-zero restore as "815" -> "0815": length 3, not length 4.
        assert_eq!(normalize_time_on("123"), Some("0123".to_string()));
        assert_eq!(normalize_time_on("12"), None);
        assert_eq!(normalize_time_on(""), None);
    }

    #[test]
    fn parse_freq_mhz_treats_a_large_value_as_hz() {
        assert_eq!(parse_freq_mhz("14.074"), Some(14.074));
        assert_eq!(parse_freq_mhz("14074000"), Some(14.074));
        assert_eq!(parse_freq_mhz("0"), None);
        assert_eq!(parse_freq_mhz("-1"), None);
        assert_eq!(parse_freq_mhz(""), None);
        assert_eq!(parse_freq_mhz("not a number"), None);
    }

    #[test]
    fn is_valid_grid_accepts_2_4_6_8_char_forms_only() {
        assert!(is_valid_grid("FN"));
        assert!(is_valid_grid("FN31"));
        assert!(is_valid_grid("FN31pr"));
        assert!(is_valid_grid("FN31pr12"));
        assert!(!is_valid_grid("FN3"));
        assert!(!is_valid_grid(""));
        assert!(!is_valid_grid("ZZ99"));
    }

    #[test]
    fn activities_builds_one_entry_per_ref_first_marked_primary() {
        let arr = activities(Some("POTA"), Some("US-0001,US-0002")).unwrap();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["ref"], "US-0001");
        assert_eq!(arr[0]["isPrimary"], true);
        assert!(arr[1].get("isPrimary").is_none());
    }

    #[test]
    fn activities_dedupes_and_caps_at_max_and_rejects_unknown_programs() {
        let many = (0..10).map(|i| format!("US-{i:04}")).collect::<Vec<_>>().join(",");
        let arr = activities(Some("sota"), Some(&many)).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), MAX_ACTIVITIES);
        assert!(activities(Some("WAS"), Some("TX")).is_none());
        assert!(activities(None, Some("US-0001")).is_none());
        assert!(activities(Some("POTA"), Some("")).is_none());
    }

    #[test]
    fn build_contact_json_rejects_a_missing_required_field() {
        let mut f = fields();
        f.call = "".to_string();
        assert!(build_contact_json(&f, None).is_err());

        let mut f = fields();
        f.qso_date = "2026112".to_string();
        assert!(build_contact_json(&f, None).is_err());

        let mut f = fields();
        f.freq_mhz = "".to_string();
        assert!(build_contact_json(&f, None).is_err());
    }

    #[test]
    fn build_contact_json_drops_an_invalid_grid_instead_of_failing() {
        let mut f = fields();
        f.gridsquare = "NOTAGRID".to_string();
        let json = build_contact_json(&f, None).unwrap();
        assert!(!json.contains("gridsquare"));
        assert!(json.contains("\"call\":\"K1ABC\""));
    }

    #[test]
    fn build_contact_json_includes_logbook_id_when_given() {
        let json = build_contact_json(&fields(), Some("abc-123")).unwrap();
        assert!(json.contains("\"logbookId\":\"abc-123\""));
        let json = build_contact_json(&fields(), None).unwrap();
        assert!(!json.contains("logbookId"));
    }

    #[test]
    fn describe_failure_prefers_the_structured_error_object() {
        let body = r#"{"error":{"code":"INVALID_KEY","message":"The API key is not valid.","field":"apiKey"}}"#;
        assert_eq!(describe_failure(401, body), "HTTP 401 INVALID_KEY: The API key is not valid. [apiKey]");
    }

    #[test]
    fn describe_failure_falls_back_to_the_flattened_body() {
        assert_eq!(describe_failure(502, "<html>Bad gateway</html>"), "HTTP 502: <html>Bad gateway</html>");
        assert_eq!(describe_failure(500, ""), "HTTP 500");
    }

    #[test]
    fn is_batch_fatal_covers_auth_rate_limit_and_server_errors() {
        assert!(is_batch_fatal(401, ""));
        assert!(is_batch_fatal(403, ""));
        assert!(is_batch_fatal(429, ""));
        assert!(is_batch_fatal(500, ""));
        assert!(is_batch_fatal(422, r#"{"error":{"code":"LOGBOOK_REQUIRED"}}"#));
        assert!(is_batch_fatal(404, r#"{"error":{"field":"logbookId"}}"#));
        assert!(!is_batch_fatal(422, r#"{"error":{"code":"INVALID_BAND"}}"#));
    }

    #[test]
    fn parse_retry_after_seconds_accepts_only_the_delta_seconds_form() {
        assert_eq!(parse_retry_after_seconds(Some("5")), Some(5));
        assert_eq!(parse_retry_after_seconds(Some("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(parse_retry_after_seconds(None), None);
    }

    #[test]
    fn parse_logbooks_skips_locked_entries() {
        let body = r#"{"data":[
            {"id":"a","name":"Main","isLocked":false},
            {"id":"b","name":"Old","isLocked":true}
        ]}"#;
        let books = parse_logbooks(body).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].station_id, "a");
    }

    #[test]
    fn parse_logbooks_distinguishes_a_genuinely_empty_account_from_an_unreadable_body() {
        assert_eq!(parse_logbooks(r#"{"data":[]}"#), Some(vec![]));
        assert_eq!(parse_logbooks(""), None);
        assert_eq!(parse_logbooks("not json"), None);
        assert_eq!(parse_logbooks("{}"), None);
    }

    #[test]
    fn choose_upload_logbook_only_auto_picks_a_lone_logbook() {
        let one = vec![StationProfile { station_id: "a".to_string(), ..Default::default() }];
        assert_eq!(choose_upload_logbook(&one), Some("a".to_string()));
        let two = vec![
            StationProfile { station_id: "a".to_string(), ..Default::default() },
            StationProfile { station_id: "b".to_string(), ..Default::default() },
        ];
        assert_eq!(choose_upload_logbook(&two), None);
        assert_eq!(choose_upload_logbook(&[]), None);
    }

    #[test]
    fn logbook_label_prefers_name_then_falls_back_to_id() {
        let named = StationProfile { station_id: "a".to_string(), profile_name: "Home".to_string(), callsign: "K1ABC".to_string(), ..Default::default() };
        assert_eq!(logbook_label(&named), "Home (K1ABC)");
        let bare = StationProfile { station_id: "a".to_string(), ..Default::default() };
        assert_eq!(logbook_label(&bare), "a");
    }

    #[test]
    fn interpret_me_requires_a_resolvable_default_when_no_logbook_is_chosen() {
        assert!(!interpret_me(r#"{"data":{"defaultLogbook":{"resolution":"none"}}}"#, None).ok);
        assert!(!interpret_me(r#"{"data":{"defaultLogbook":{"logbookId":null,"resolution":null}}}"#, None).ok);
        assert!(interpret_me(r#"{"data":{"defaultLogbook":{"logbookId":"a","resolution":"single"}}}"#, None).ok);
        assert!(interpret_me(r#"{"data":{}}"#, Some("abc")).ok);
        assert!(!interpret_me("not json", None).ok);
    }

    #[test]
    fn interpret_logbook_choice_requires_the_chosen_id_in_the_readable_list() {
        let books = vec![StationProfile { station_id: "a".to_string(), ..Default::default() }];
        assert!(interpret_logbook_choice(Some(&books), "a").ok);
        assert!(!interpret_logbook_choice(Some(&books), "b").ok);
        assert!(!interpret_logbook_choice(None, "a").ok);
    }

    #[test]
    fn check_connection_requires_a_key() {
        assert!(!check_connection("", "").ok);
    }

    #[test]
    fn upload_pending_is_a_no_op_without_an_api_key() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db);
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        db.set_config(CONFIG_API_KEY, "fake-key-not-used-because-nothing-pending").unwrap();
        let result = upload_pending(&db);
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }

    #[test]
    fn is_wrl_fatal_message_matches_the_messages_upload_fields_actually_returns() {
        assert!(is_wrl_fatal_message("no logbook in the World Radio League account: create one in Logging settings"));
        assert!(is_wrl_fatal_message("HTTP 401 INVALID_KEY: The API key is not valid."));
        assert!(!is_wrl_fatal_message("missing call"));
    }
}
