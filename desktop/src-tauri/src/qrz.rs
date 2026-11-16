//! QRZ.com integration, ported from Android's `QrzWebClient.kt` (dead code
//! here -- see below) / `QrzXmlClient.kt` / `ThirdPartyService.java`'s QRZ
//! section / `MainViewModel.java`'s post-log upload thread. Three real
//! features, confirmed against the fork's actual call sites rather than
//! guessed:
//!
//! 1. **Station lookup during a QSO**: the moment the engine targets a
//!    station to answer (`qso.rs`'s `answer`, mirroring
//!    `FT8TransmitSignal.java`'s one real call site), a background lookup
//!    fetches and caches that station's QRZ-registered state/grid/country
//!    (`QrzXmlClient::prefetch`) so it's ready by the time the QSO
//!    completes and is logged -- feeds `logged_state.rs`'s `STATE` pick and
//!    a `COUNTRY` backfill, both genuinely missing from desktop's logging
//!    path before this. Android's version *also* fetches a profile photo
//!    (`QrzWebClient`'s public-page scrape + `QrzXmlClient`'s `<image>`
//!    tag) for a per-contact avatar chip -- not ported: desktop has no
//!    per-contact avatar UI anywhere to show it (confirmed: grepped the
//!    whole frontend), so that half would be dead plumbing. `QrzWebClient`
//!    itself is correspondingly not ported at all.
//! 2. **Manual upload**: a Settings button that walks every QSO not yet
//!    marked `synced_qrz`, one `ACTION=INSERT` call per QSO (QRZ's Logbook
//!    API has no batch endpoint). Confirmed real Android behavior,
//!    `ThirdPartyService`'s catch-up loop (~line 868): unlike Cloudlog,
//!    QRZ's loop never stops early on a credential failure -- it just
//!    records the first error message and keeps going row by row. A
//!    `RESULT=FAIL` whose `REASON` contains "duplicate" still counts as
//!    synced (QRZ already has it).
//! 3. **Auto-upload on log**: `MainViewModel.java`'s post-insert background
//!    thread, gated on `enableQRZ` -- every QSO is uploaded individually
//!    the moment it's logged, not batched. Same per-row logic as (2), just
//!    triggered immediately for one QSO instead of walked over the pending
//!    set.
//!
//! **Test/verify buttons** (not from a literal Android call site, but a
//! real QRZ API action already used for exactly this by Android's own
//! `CheckQRZConnection` -- `ThirdPartyService.java` ~line 451):
//! `ACTION=STATUS` for the Logbook API key (read-only, inserts nothing),
//! and `QrzXmlClient`'s session auth call itself for the XML credentials
//! (matches Android's `testConnection()`).
//!
//! Desktop's `qrz_qsl`/`synced_qrz` columns and `Db::mark_synced`/
//! `Db::is_synced` already existed for this (confirmed unused by any real
//! call site before this module), so no schema change was needed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Serialize;

use crate::db::{adif_record, Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableQRZ";
pub const CONFIG_API_KEY: &str = "qrzApiKey";
pub const CONFIG_XML_USERNAME: &str = "qrzXmlUsername";
pub const CONFIG_XML_PASSWORD: &str = "qrzXmlPassword";

const LOGBOOK_URL: &str = "https://logbook.qrz.com/api";
const XML_URL: &str = "https://xmldata.qrz.com/xml/current/";
const CACHE_MAX: usize = 200;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build()
}

// --- Logbook API: upload ----------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct QrzUploadResult {
    pub uploaded: usize,
    pub duplicates: usize,
    pub failed: usize,
    /// The *first* failure's message, if any -- informational only. Unlike
    /// Cloudlog's batch, a QRZ failure never stops the remaining rows (see
    /// this module's top doc comment), so later rows may have succeeded
    /// even when this is set.
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
enum InsertOutcome {
    Inserted,
    Duplicate,
}

fn parse_kv_response(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.trim().to_uppercase(), v.trim().to_string()))
        .collect()
}

fn insert_adif(api_key: &str, record: &str) -> Result<InsertOutcome, String> {
    let resp = agent()
        .post(LOGBOOK_URL)
        .send_form(&[("KEY", api_key), ("ACTION", "INSERT"), ("ADIF", record)]);
    let body = match resp {
        Ok(r) => r.into_string().map_err(|e| format!("{e}"))?,
        Err(e) => return Err(format!("{e}")),
    };
    let kv = parse_kv_response(&body);
    match kv.get("RESULT").map(String::as_str) {
        Some("OK") | Some("REPLACE") => Ok(InsertOutcome::Inserted),
        Some("FAIL") => {
            let reason = kv.get("REASON").cloned().unwrap_or_default();
            if reason.to_lowercase().contains("duplicate") {
                Ok(InsertOutcome::Duplicate)
            } else if reason.is_empty() {
                Err("QRZ rejected the upload.".to_string())
            } else {
                Err(reason)
            }
        }
        _ => Err("QRZ returned an unexpected response.".to_string()),
    }
}

/// Uploads one QSO and marks it synced on success (insert or duplicate --
/// QRZ already has it either way). Shared by the manual batch and the
/// auto-upload-on-log hook; returns the per-row outcome for the caller to
/// tally, never panics/blocks the engine thread itself (callers that run
/// this from engine.rs do so on a spawned thread).
fn upload_and_mark_one(db: &Db, api_key: &str, q: &QsoRecord) -> Result<InsertOutcome, String> {
    let outcome = insert_adif(api_key, &adif_record(q))?;
    if let Some(id) = q.id {
        let _ = db.mark_synced(id, "qrz");
    }
    Ok(outcome)
}

/// Uploads every QSO not yet marked synced to QRZ (`synced_qrz = 0`), one
/// `ACTION=INSERT` call per QSO. Never stops early on a failure -- see this
/// module's top doc comment for why that's a deliberate match of Android's
/// real behavior, not a missing safeguard.
pub fn upload_pending(db: &Db, api_key: &str) -> QrzUploadResult {
    if api_key.trim().is_empty() {
        return QrzUploadResult {
            error: Some("Enter your QRZ Logbook API key first.".to_string()),
            ..Default::default()
        };
    }
    let pending = db.qsos_pending_qrz_upload();
    let mut result = QrzUploadResult::default();
    for q in &pending {
        match upload_and_mark_one(db, api_key, q) {
            Ok(InsertOutcome::Inserted) => result.uploaded += 1,
            Ok(InsertOutcome::Duplicate) => result.duplicates += 1,
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
/// on a spawned thread, never the engine's own run-loop thread, matching
/// Android's own background-thread upload.
pub fn upload_one_logged(db: &Db, api_key: &str, q: &QsoRecord) {
    if api_key.trim().is_empty() {
        return;
    }
    let _ = upload_and_mark_one(db, api_key, q);
}

/// Read-only check that an API key is accepted, using QRZ's own
/// `ACTION=STATUS` (inserts nothing) -- matches Android's
/// `CheckQRZConnection`.
pub fn check_logbook_connection(api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("Enter your QRZ Logbook API key first.".to_string());
    }
    let resp = agent().post(LOGBOOK_URL).send_form(&[("KEY", api_key), ("ACTION", "STATUS")]);
    let body = match resp {
        Ok(r) => r.into_string().map_err(|e| format!("{e}"))?,
        Err(e) => return Err(format!("{e}")),
    };
    let kv = parse_kv_response(&body);
    match kv.get("RESULT").map(String::as_str) {
        Some("OK") => Ok(()),
        Some("FAIL") => Err(kv.get("REASON").cloned().unwrap_or_else(|| "QRZ rejected the API key.".to_string())),
        _ => Err("QRZ returned an unexpected response.".to_string()),
    }
}

// --- XML API: station lookup --------------------------------------------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct QrzLookup {
    pub state: String,
    pub grid: String,
    pub country: String,
}

fn parse_xml_fields(body: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(body);
    let mut out = HashMap::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => current = Some(String::from_utf8_lossy(e.name().as_ref()).to_string()),
            Ok(Event::Text(t)) => {
                if let Some(tag) = &current {
                    if let Ok(text) = t.decode() {
                        let text = text.trim();
                        if !text.is_empty() {
                            out.insert(tag.clone(), text.to_string());
                        }
                    }
                }
            }
            Ok(Event::End(_)) => current = None,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    out
}

fn is_invalid_session(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("invalid session") || e.contains("session timeout")
}

fn authenticate(username: &str, password: &str) -> Result<String, String> {
    let resp = agent().get(XML_URL).query("username", username).query("password", password).call();
    let body = match resp {
        Ok(r) => r.into_string().map_err(|e| format!("{e}"))?,
        Err(e) => return Err(format!("{e}")),
    };
    let fields = parse_xml_fields(&body);
    if let Some(key) = fields.get("Key") {
        return Ok(key.clone());
    }
    Err(fields.get("Error").cloned().unwrap_or_else(|| "QRZ did not return a session key.".to_string()))
}

#[derive(Debug)]
enum FetchError {
    SessionExpired,
    Other(String),
}

/// Pure parse/branch step, shared by `fetch_lookup` and its tests. A
/// lookup response is valid unless it carries an `Error` tag -- any of
/// `state`/`grid`/`country` may simply be absent (QRZ has no entry for
/// that field), which is not itself an error.
fn parse_lookup_response(body: &str) -> Result<QrzLookup, FetchError> {
    let fields = parse_xml_fields(body);
    if let Some(err) = fields.get("Error") {
        return Err(if is_invalid_session(err) {
            FetchError::SessionExpired
        } else {
            FetchError::Other(err.clone())
        });
    }
    Ok(QrzLookup {
        state: fields.get("state").cloned().unwrap_or_default().to_uppercase(),
        grid: fields.get("grid").cloned().unwrap_or_default(),
        country: fields.get("country").cloned().unwrap_or_default(),
    })
}

fn fetch_lookup(session_key: &str, callsign: &str) -> Result<QrzLookup, FetchError> {
    let resp = agent().get(XML_URL).query("s", session_key).query("callsign", callsign).call();
    let body = match resp {
        Ok(r) => r.into_string().map_err(|e| FetchError::Other(format!("{e}")))?,
        Err(e) => return Err(FetchError::Other(format!("{e}"))),
    };
    parse_lookup_response(&body)
}

/// Session-key-cached QRZ XML client with a bounded per-callsign lookup
/// cache (200 entries, oldest evicted first -- Android uses a strict LRU;
/// this is a simpler FIFO approximation, noted since it's a real behavioral
/// difference under cache pressure, not expected to matter at this size).
/// Cheaply `Clone`-able (both fields are `Arc`-wrapped) so a spawned
/// prefetch thread can share the same cache the owning `Engine` reads back
/// from at QSO-completion time.
#[derive(Clone)]
pub struct QrzXmlClient {
    session_key: Arc<Mutex<Option<String>>>,
    cache: Arc<Mutex<VecDeque<(String, QrzLookup)>>>,
}

impl Default for QrzXmlClient {
    fn default() -> Self {
        Self { session_key: Arc::new(Mutex::new(None)), cache: Arc::new(Mutex::new(VecDeque::new())) }
    }
}

impl QrzXmlClient {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_session(&self, username: &str, password: &str) -> Result<String, String> {
        if let Some(k) = self.session_key.lock().unwrap().clone() {
            return Ok(k);
        }
        let key = authenticate(username, password)?;
        *self.session_key.lock().unwrap() = Some(key.clone());
        Ok(key)
    }

    fn cache_get(&self, callsign: &str) -> Option<QrzLookup> {
        let cache = self.cache.lock().unwrap();
        cache.iter().find(|(c, _)| c == callsign).map(|(_, v)| v.clone())
    }

    fn cache_put(&self, callsign: String, lookup: QrzLookup) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|(c, _)| c != &callsign);
        cache.push_back((callsign, lookup));
        while cache.len() > CACHE_MAX {
            cache.pop_front();
        }
    }

    /// Cache-only read -- never touches the network. What engine.rs reads
    /// back at QSO-completion time.
    pub fn cached(&self, callsign: &str) -> Option<QrzLookup> {
        self.cache_get(&callsign.trim().to_uppercase())
    }

    fn lookup_blocking(&self, username: &str, password: &str, callsign: &str) -> Result<QrzLookup, String> {
        let key = self.ensure_session(username, password)?;
        match fetch_lookup(&key, callsign) {
            Ok(lookup) => Ok(lookup),
            Err(FetchError::Other(e)) => Err(e),
            Err(FetchError::SessionExpired) => {
                *self.session_key.lock().unwrap() = None;
                let key2 = self.ensure_session(username, password)?;
                match fetch_lookup(&key2, callsign) {
                    Ok(lookup) => Ok(lookup),
                    Err(FetchError::Other(e)) => Err(e),
                    Err(FetchError::SessionExpired) => Err("QRZ XML session expired again immediately.".to_string()),
                }
            }
        }
    }

    /// Looks `callsign` up in the background and caches the result, so it's
    /// ready by the time the QSO completes. No-op without configured
    /// credentials, for "CQ" (not a real target), or when already cached --
    /// matches `QrzXmlClient.prefetch`'s exact guard conditions. Fire-and-
    /// forget: failures are silent here (nothing is cached on error, so a
    /// later `cached()` read just finds nothing, same as never having
    /// looked up at all).
    pub fn prefetch(&self, db: &Db, callsign: &str) {
        let key = callsign.trim().to_uppercase();
        if key.is_empty() || key == "CQ" || self.cache_get(&key).is_some() {
            return;
        }
        let username = db.get_config(CONFIG_XML_USERNAME).unwrap_or_default();
        let password = db.get_config(CONFIG_XML_PASSWORD).unwrap_or_default();
        if username.is_empty() || password.is_empty() {
            return;
        }
        let client = self.clone();
        std::thread::spawn(move || {
            if let Ok(lookup) = client.lookup_blocking(&username, &password, &key) {
                client.cache_put(key, lookup);
            }
        });
    }

    /// Verifies the configured XML credentials by forcing a fresh
    /// authentication, discarding any cached session first -- matches
    /// Android's `testConnection()`. `"OK"` on success, else a short
    /// message suitable for Settings.
    pub fn test_connection(&self, username: &str, password: &str) -> String {
        if username.trim().is_empty() || password.is_empty() {
            return "Missing username or password".to_string();
        }
        *self.session_key.lock().unwrap() = None;
        match self.ensure_session(username, password) {
            Ok(_) => "OK".to_string(),
            Err(e) => e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kv_response_reads_result_and_reason() {
        let kv = parse_kv_response("RESULT=FAIL&REASON=Duplicate QSO, ignored");
        assert_eq!(kv.get("RESULT").map(String::as_str), Some("FAIL"));
        assert_eq!(kv.get("REASON").map(String::as_str), Some("Duplicate QSO, ignored"));
    }

    const AUTH_OK: &str = "<QRZDatabase><Session><Key>abc123</Key><Count>1</Count></Session></QRZDatabase>";
    const AUTH_ERR: &str = "<QRZDatabase><Session><Error>Username/password incorrect</Error></Session></QRZDatabase>";
    const LOOKUP_OK: &str = "<QRZDatabase><Callsign><call>W1AW</call><state>CT</state><country>United States</country>\
        <grid>FN31pr</grid></Callsign><Session><Key>abc123</Key></Session></QRZDatabase>";
    const LOOKUP_NO_STATE: &str = "<QRZDatabase><Callsign><call>G0ABC</call><country>England</country>\
        <grid>IO91wm</grid></Callsign></QRZDatabase>";
    const LOOKUP_SESSION_EXPIRED: &str = "<QRZDatabase><Session><Error>Session Timeout</Error></Session></QRZDatabase>";
    const LOOKUP_NOT_FOUND: &str = "<QRZDatabase><Session><Error>Not found: X9ZZZ</Error></Session></QRZDatabase>";

    #[test]
    fn parse_xml_fields_reads_the_auth_key() {
        assert_eq!(parse_xml_fields(AUTH_OK).get("Key").map(String::as_str), Some("abc123"));
    }

    #[test]
    fn parse_xml_fields_reads_an_auth_error() {
        assert_eq!(
            parse_xml_fields(AUTH_ERR).get("Error").map(String::as_str),
            Some("Username/password incorrect")
        );
    }

    #[test]
    fn parse_lookup_response_reads_state_grid_country() {
        let lookup = parse_lookup_response(LOOKUP_OK).unwrap();
        assert_eq!(lookup.state, "CT");
        assert_eq!(lookup.grid, "FN31pr");
        assert_eq!(lookup.country, "United States");
    }

    #[test]
    fn parse_lookup_response_tolerates_a_missing_state() {
        let lookup = parse_lookup_response(LOOKUP_NO_STATE).unwrap();
        assert_eq!(lookup.state, "");
        assert_eq!(lookup.country, "England");
    }

    #[test]
    fn parse_lookup_response_detects_session_expiry_distinctly_from_other_errors() {
        assert!(matches!(parse_lookup_response(LOOKUP_SESSION_EXPIRED), Err(FetchError::SessionExpired)));
        assert!(matches!(parse_lookup_response(LOOKUP_NOT_FOUND), Err(FetchError::Other(_))));
    }

    #[test]
    fn prefetch_is_a_no_op_for_cq_and_for_empty_callsigns() {
        let db = Db::open_in_memory().unwrap();
        let client = QrzXmlClient::new();
        client.prefetch(&db, "CQ");
        client.prefetch(&db, "");
        assert!(client.cached("CQ").is_none());
    }

    #[test]
    fn prefetch_is_a_no_op_without_configured_credentials() {
        let db = Db::open_in_memory().unwrap();
        let client = QrzXmlClient::new();
        client.prefetch(&db, "W1AW");
        // No network call should even be attempted -- nothing to assert on
        // the network, but the cache must stay empty synchronously (the
        // early-return guard runs before any thread is spawned).
        assert!(client.cached("W1AW").is_none());
    }

    #[test]
    fn cache_put_then_get_round_trips_uppercased() {
        let client = QrzXmlClient::new();
        client.cache_put("W1AW".to_string(), QrzLookup { state: "CT".into(), grid: "FN31pr".into(), country: "United States".into() });
        assert_eq!(client.cached("w1aw").unwrap().state, "CT");
        assert!(client.cached("K1ABC").is_none());
    }

    #[test]
    fn cache_evicts_oldest_past_the_cap() {
        let client = QrzXmlClient::new();
        for i in 0..(CACHE_MAX + 5) {
            client.cache_put(format!("CALL{i}"), QrzLookup::default());
        }
        assert!(client.cached("CALL0").is_none());
        assert!(client.cached(&format!("CALL{}", CACHE_MAX + 4)).is_some());
        assert_eq!(client.cache.lock().unwrap().len(), CACHE_MAX);
    }

    #[test]
    fn upload_pending_is_a_no_op_with_no_api_key() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "");
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db, "fake-key-not-used-because-nothing-pending");
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }

    #[test]
    fn check_logbook_connection_requires_a_key() {
        assert!(check_logbook_connection("").is_err());
    }
}
