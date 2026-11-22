//! Cloudlog / Wavelog / Nextlog integration, ported from Android's
//! `CloudlogEndpoint.java` / `ThirdPartyService.java`'s Cloudlog section /
//! `CloudlogFailureHint.kt`. **Upload only** -- the Android fork also has an
//! optional two-way sync that downloads QSOs from the Wavelog logbook and
//! auto-inserts the ones missing locally (`CloudlogRemoteLog.java`,
//! `downloadMissingFromCloudlog`/`insertRemoteQsos`, issue #965). That's a
//! materially bigger, riskier feature (it inserts new QSO rows from a remote
//! server) that the user explicitly chose NOT to port in this pass, since
//! they don't use these services themselves and can't verify it live. If it's
//! ever wanted later, `CloudlogRemoteLog.java`/`fetchCloudlogRemoteLog` is
//! where to start.
//!
//! Three pieces, matching `qrz.rs`'s shape:
//! 1. **Test connection** (`api/auth/[key]`) + **station picker**
//!    (`api/station_info/[key]`) for the Settings dialog.
//! 2. **Manual/startup catch-up upload**: every QSO not yet `synced_cloudlog`,
//!    one `api/qso` POST per QSO (no batch endpoint). A Station ID / API key
//!    rejection stops the rest of the batch (every row would fail the same
//!    way); any other failure just moves on to the next row.
//! 3. **Auto-upload on log**: the same per-row logic, fired immediately for
//!    one QSO the moment it's logged (`enableCloudlog`).
//!
//! Self-hosted installs vary a lot (no scheme, no `mod_rewrite`, basic auth
//! in the URL) -- `normalize_base`/`candidates` handle that exactly like
//! Android's `CloudlogEndpoint`, including remembering which URL shape
//! answered so later uploads don't pay for a 404 round-trip every time.
//!
//! **Never log or display a Cloudlog URL un-redacted** -- the API key travels
//! as a path segment on `api/auth/[key]` and `api/station_info/[key]`, and a
//! self-hosted install can carry `user:pass@host` basic-auth credentials in
//! the address itself. Every error message in this module is built from the
//! HTTP status and response body alone, never from the request URL (same
//! lesson as the LoTW password-leak fix -- see `lotw.rs`'s top doc comment).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use crate::db::{adif_record, Db, QsoRecord};

pub const CONFIG_ENABLE: &str = "enableCloudlog";
pub const CONFIG_ADDRESS: &str = "cloudlogServerAddress";
pub const CONFIG_API_KEY: &str = "cloudlogApiKey";
pub const CONFIG_STATION_ID: &str = "cloudlogStationID";

const INDEX_PHP: &str = "index.php/";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest failure description kept -- a server error body can be a whole HTML page.
const MAX_FAILURE_LEN: usize = 200;

fn agent() -> ureq::Agent {
    // Redirects disabled -- followed manually by `attempt_with_redirects`,
    // which resends the same method/body each hop and checks same-host/
    // no-downgrade before following. `ureq`'s own default redirect
    // handling would either drop the POST body (301/302/303, converted to
    // GET) or not follow at all (307/308), silently mishandling exactly
    // the Wavelog/Nextlog 308 this module's top doc comment describes.
    ureq::AgentBuilder::new()
        .timeout_connect(HTTP_CONNECT_TIMEOUT)
        .timeout_read(HTTP_READ_TIMEOUT)
        .redirects(0)
        .build()
}

/// A station profile from `api/station_info`: `(station_id, profile_name, callsign, gridsquare)`.
/// Shared with `wrl.rs`, whose logbooks have the same shape (id/name/callsign).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct StationProfile {
    pub station_id: String,
    pub profile_name: String,
    pub callsign: String,
    pub gridsquare: String,
}

impl StationProfile {
    pub fn display_label(&self) -> String {
        let mut out = self.station_id.clone();
        if !self.profile_name.is_empty() {
            out.push_str(" - ");
            out.push_str(&self.profile_name);
        }
        if !self.callsign.is_empty() {
            out.push_str(" (");
            out.push_str(&self.callsign);
            if !self.gridsquare.is_empty() {
                out.push_str(", ");
                out.push_str(&self.gridsquare);
            }
            out.push(')');
        }
        out
    }
}

// --- URL plumbing (port of CloudlogEndpoint.java) -----------------------

fn host_of(schemeless: &str) -> String {
    let mut h = schemeless;
    if let Some(at) = h.find('@') {
        h = &h[at + 1..];
    }
    if let Some(slash) = h.find('/') {
        h = &h[..slash];
    }
    if h.starts_with('[') {
        return match h.find(']') {
            Some(close) => h[..=close].to_string(),
            None => h.to_string(),
        };
    }
    match h.find(':') {
        Some(colon) => h[..colon].to_string(),
        None => h.to_string(),
    }
}

fn looks_like_lan_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let h = host.to_lowercase();
    if h.starts_with('[') {
        return true; // IPv6 literal -- almost always a LAN box
    }
    if h.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) && h.split('.').count() == 4 {
        return true; // IPv4 literal
    }
    if h == "localhost" {
        return true;
    }
    if !h.contains('.') {
        return true; // single-label name: "nas", "raspberrypi"
    }
    h.ends_with(".local") || h.ends_with(".lan") || h.ends_with(".home") || h.ends_with(".internal")
}

/// Normalizes a user-entered address into a base URL ending in `/`. See this
/// module's top doc comment -- matches `CloudlogEndpoint.normalizeBase` exactly.
pub fn normalize_base(address: &str) -> Option<String> {
    let a = address.trim();
    if a.is_empty() {
        return None;
    }
    let mut a = if !a.contains("://") {
        let scheme = if looks_like_lan_host(&host_of(a)) { "http://" } else { "https://" };
        format!("{scheme}{a}")
    } else {
        a.to_string()
    };
    while a.ends_with("//") {
        a.pop();
    }
    if !a.ends_with('/') {
        a.push('/');
    }
    Some(a)
}

fn ends_with_index_php_segment(base: &str) -> bool {
    let rest = match base.find("://") {
        Some(scheme) => &base[scheme + 3..],
        None => base,
    };
    let path = match rest.find('/') {
        Some(slash) => &rest[slash..],
        None => "",
    };
    path.to_lowercase().ends_with(&format!("/{INDEX_PHP}"))
}

/// Address (as normalized) -> base URL variant that last answered. In-memory only.
static WORKING_BASE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn working_base_get(base: &str) -> Option<String> {
    WORKING_BASE.lock().unwrap().as_ref().and_then(|m| m.get(base).cloned())
}

/// Records which base variant answered so the next [`candidates`] call for
/// the same base tries it first.
pub fn remember_working(base: &str, url: &str) {
    let mut guard = WORKING_BASE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    let rewritten = format!("{base}{INDEX_PHP}");
    if url.starts_with(&rewritten) {
        map.insert(base.to_string(), rewritten);
    } else if url.starts_with(base) {
        map.insert(base.to_string(), base.to_string());
    }
}

/// Ordered candidate URLs for `path` under a normalized `base`: the rewritten
/// form first, then the `index.php/` fallback (or just the one URL if `base`
/// already carries `index.php`). A variant that previously answered for this
/// base is moved to the front. See this module's top doc comment.
pub fn candidates(base: &str, path: &str) -> Vec<String> {
    let p = path.trim_start_matches('/');
    if ends_with_index_php_segment(base) {
        return vec![format!("{base}{p}")];
    }
    let plain = format!("{base}{p}");
    let rewritten = format!("{base}{INDEX_PHP}{p}");
    match working_base_get(base) {
        Some(known) if known.ends_with(INDEX_PHP) => vec![rewritten, plain],
        _ => vec![plain, rewritten],
    }
}

/// Render an HTTP failure as one short line: the status, plus the server's
/// error body when it carried one, collapsed to a single line and truncated.
/// Pure -- never touches the URL, see this module's top doc comment.
pub fn describe_http_failure(status: u16, body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        format!("HTTP {status}")
    } else if flat.chars().count() > MAX_FAILURE_LEN {
        let clipped: String = flat.chars().take(MAX_FAILURE_LEN).collect();
        format!("HTTP {status}: {clipped}…")
    } else {
        format!("HTTP {status}: {flat}")
    }
}

/// Cloudlog/Wavelog's duplicate-QSO rejection, so a catch-up upload marks the
/// row synced (the logbook already has it) instead of reporting a failure.
pub fn is_duplicate_rejection(failure: &str) -> bool {
    failure.starts_with("HTTP 400") && failure.to_lowercase().contains("duplicate for")
}

// --- CloudlogFailureHint.kt (ported) -------------------------------------

/// Which setting a rejection points at. Only the server's own JSON `reason`
/// is classified; a transport error says nothing about the settings.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FailureKind {
    StationId,
    ApiKey,
    Other,
}

fn parsed_reason(failure: &str) -> Option<String> {
    let raw = failure.trim();
    if raw.is_empty() {
        return None;
    }
    // `"reason":"..."` -- a simple (non-escape-aware) scan is enough here:
    // Cloudlog/Wavelog's own reason strings never contain an escaped quote.
    let key = "\"reason\"";
    let start = raw.find(key)?;
    let after_key = &raw[start + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let after_colon = after_colon.strip_prefix('"')?;
    let end = after_colon.find('"')?;
    let reason = after_colon[..end].trim();
    (!reason.is_empty()).then(|| reason.to_string())
}

fn classify_failure(failure: &str) -> FailureKind {
    let Some(reason) = parsed_reason(failure).map(|r| r.to_lowercase()) else {
        return FailureKind::Other;
    };
    if reason.contains("station") && (reason.contains("id") || reason.contains("profile")) {
        return FailureKind::StationId;
    }
    if reason.contains("api key") || reason.contains("apikey") || reason.contains("key invalid") {
        return FailureKind::ApiKey;
    }
    FailureKind::Other
}

/// True when every further upload in a batch would be refused the same way
/// (wrong Station ID or API key), so a catch-up upload should stop instead
/// of repeating the same rejection once per QSO.
pub fn failure_stops_batch(failure: &str) -> bool {
    classify_failure(failure) != FailureKind::Other
}

// --- HTTP transport -------------------------------------------------------

/// Max redirect hops followed for one candidate URL, matching Android's
/// `sendPostRequest` loop.
const MAX_REDIRECT_HOPS: u32 = 5;

struct RawResponse {
    status: u16,
    body: String,
    location: Option<String>,
}

/// One raw HTTP exchange, redirects disabled -- `Ok` for any response the
/// server actually sent (2xx/3xx/4xx/5xx alike), `Err` only for a transport
/// failure (DNS, connect, timeout). Separating "got an HTTP response" from
/// "the request itself failed" is what lets [`attempt_with_redirects`] see
/// and act on a bare 3xx, which `ureq`'s own default redirect-following
/// would otherwise hide or mishandle -- see this module's top doc comment.
fn send_once(method: &str, url: &str, json: Option<&str>) -> Result<RawResponse, String> {
    let agent = agent();
    let result = match (method, json) {
        ("GET", _) => agent.get(url).call(),
        ("POST", Some(body)) => agent.post(url).set("Content-Type", "application/json").send_string(body),
        _ => unreachable!("cloudlog transport only sends GET or POST-with-body"),
    };
    match result {
        Ok(r) => {
            let status = r.status();
            let location = r.header("Location").map(str::to_string);
            let body = r.into_string().unwrap_or_default();
            Ok(RawResponse { status, body, location })
        }
        Err(ureq::Error::Status(status, response)) => {
            let location = response.header("Location").map(str::to_string);
            let body = response.into_string().unwrap_or_default();
            Ok(RawResponse { status, body, location })
        }
        Err(ureq::Error::Transport(t)) => Err(format!("{}", t.kind())),
    }
}

/// Whether a redirect from `from` to `to` may be followed: same host only,
/// never downgrading https to http. The request can carry the API key (a
/// GET path segment, or inside the POST body), so following a redirect
/// anywhere else would hand it to that host, or send it in the clear.
fn redirect_allowed(from: &url::Url, to: &url::Url) -> bool {
    if !from.host_str().map(str::to_lowercase).eq(&to.host_str().map(str::to_lowercase)) {
        return false;
    }
    let from_https = from.scheme().eq_ignore_ascii_case("https");
    let to_https = to.scheme().eq_ignore_ascii_case("https");
    if from_https && !to_https {
        return false;
    }
    to_https || to.scheme().eq_ignore_ascii_case("http")
}

/// One logical request against a single candidate URL, resolving up to
/// [`MAX_REDIRECT_HOPS`] redirects itself (re-sending the same method and
/// body each hop -- see this module's top doc comment on why `ureq`'s own
/// redirect handling can't be trusted here). `Ok(body)` on 200/201,
/// `Err((status, body))` on any other terminal status (`status` 0 for a
/// transport failure, a redirect limit, or a refused redirect).
fn attempt_with_redirects(method: &str, url: &str, json: Option<&str>) -> Result<String, (u16, String)> {
    let mut current = url.to_string();
    for _ in 0..MAX_REDIRECT_HOPS {
        let resp = send_once(method, &current, json).map_err(|e| (0, e))?;
        match resp.status {
            200 | 201 => return Ok(resp.body),
            301..=303 | 307 | 308 => {
                let Some(location) = resp.location.filter(|l| !l.is_empty()) else {
                    return Err((resp.status, format!("HTTP {} redirect with no Location header", resp.status)));
                };
                let Ok(from_url) = url::Url::parse(&current) else {
                    return Err((resp.status, "redirect source URL could not be parsed".to_string()));
                };
                let Ok(resolved) = from_url.join(&location) else {
                    return Err((resp.status, "redirect target could not be parsed".to_string()));
                };
                if !redirect_allowed(&from_url, &resolved) {
                    return Err((
                        resp.status,
                        format!(
                            "HTTP {} redirect to {}://{} refused: set that address in the Cloudlog settings",
                            resp.status,
                            resolved.scheme(),
                            resolved.host_str().unwrap_or_default()
                        ),
                    ));
                }
                current = resolved.to_string();
            }
            status => return Err((status, resp.body)),
        }
    }
    Err((0, "exceeded redirect limit".to_string()))
}

/// The candidate walk shared by GET and POST: try each URL shape from
/// [`candidates`] in order; a 404 moves on to the next shape, any other
/// status or a transport failure stops; the first success pins its shape.
fn request(method: &str, base: &str, path: &str, json: Option<&str>) -> Result<String, String> {
    let mut last_failure: Option<String> = None;
    for url in candidates(base, path) {
        match attempt_with_redirects(method, &url, json) {
            Ok(body) => {
                remember_working(base, &url);
                return Ok(body);
            }
            Err((404, _)) => {
                last_failure = Some("HTTP 404".to_string());
                continue;
            }
            Err((0, msg)) => {
                last_failure = Some(msg);
                break;
            }
            Err((status, body)) => {
                last_failure = Some(describe_http_failure(status, &body));
                break;
            }
        }
    }
    Err(last_failure.unwrap_or_else(|| "no response".to_string()))
}

fn cloudlog_get(base: &str, path: &str) -> Result<String, String> {
    request("GET", base, path, None)
}

fn cloudlog_post(base: &str, path: &str, json: &str) -> Result<String, String> {
    request("POST", base, path, Some(json))
}

// --- Test connection + station picker -------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: Option<String>,
}

/// Interprets the body of `api/auth/[key]`. Pure -- Nextlog and Wavelog both
/// implement it but return slightly different shapes, so match on the
/// meaningful markers.
fn interpret_auth_response(body: &str) -> ConnectionCheck {
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let valid = compact.contains("<status>Valid</status>");
    let rw = compact.contains("<rights>rw</rights>");
    if valid && rw {
        return ConnectionCheck { ok: true, detail: None };
    }
    if valid {
        return ConnectionCheck { ok: false, detail: Some("API key is read-only (needs rw rights)".to_string()) };
    }
    if compact.contains("<status>") {
        return ConnectionCheck { ok: false, detail: Some("API key rejected by server".to_string()) };
    }
    ConnectionCheck { ok: false, detail: Some("unexpected reply (not a Cloudlog API?)".to_string()) }
}

/// Verifies the configured address + API key via `api/auth/[key]`. Reads
/// config directly, matching `qrz.rs`'s pattern for its own test-connection
/// commands.
pub fn check_connection(db: &Db) -> ConnectionCheck {
    let address = db.get_config(CONFIG_ADDRESS).unwrap_or_default();
    let api_key = db.get_config(CONFIG_API_KEY).unwrap_or_default();
    let Some(base) = normalize_base(&address) else {
        return ConnectionCheck { ok: false, detail: Some("no server address configured".to_string()) };
    };
    if api_key.trim().is_empty() {
        return ConnectionCheck { ok: false, detail: Some("no API key configured".to_string()) };
    }
    match cloudlog_get(&base, &format!("api/auth/{}", api_key.trim())) {
        Ok(body) => interpret_auth_response(&body),
        Err(e) => ConnectionCheck { ok: false, detail: Some(e) },
    }
}

fn parse_station_profiles(json: &str) -> Vec<StationProfile> {
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_object())
        .map(|o| StationProfile {
            station_id: o.get("station_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            profile_name: o.get("station_profile_name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            callsign: o.get("station_callsign").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            gridsquare: o.get("station_gridsquare").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
        .collect()
}

/// Fetches station profiles from a Cloudlog/Wavelog/Nextlog server. Empty on
/// any failure (network, bad JSON, 404) -- never an error to the caller.
pub fn fetch_stations(address: &str, api_key: &str) -> Vec<StationProfile> {
    let Some(base) = normalize_base(address) else {
        return Vec::new();
    };
    if api_key.trim().is_empty() {
        return Vec::new();
    }
    match cloudlog_get(&base, &format!("api/station_info/{}", api_key.trim())) {
        Ok(body) => parse_station_profiles(&body),
        Err(_) => Vec::new(),
    }
}

// --- Upload ----------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct CloudlogUploadResult {
    pub uploaded: usize,
    pub already_present: usize,
    pub failed: usize,
    /// The *first* failure's message, if any. See this module's top doc
    /// comment -- a Station ID / API key rejection stops the batch, so a
    /// later row having `failed` incremented too means every remaining row
    /// was skipped for that same reason, not retried and failed again.
    pub error: Option<String>,
}

fn upload_json(api_key: &str, station_id: &str, adif: &str) -> String {
    serde_json::json!({
        "key": api_key,
        "station_profile_id": station_id,
        "type": "adif",
        "string": adif,
    })
    .to_string()
}

fn upload_and_mark_one(db: &Db, base: &str, api_key: &str, station_id: &str, q: &QsoRecord) -> Result<(), String> {
    let body = upload_json(api_key, station_id, &adif_record(q));
    let result = cloudlog_post(base, "api/qso", &body);
    match &result {
        Ok(_) => {
            if let Some(id) = q.id {
                let _ = db.mark_synced(id, "cloudlog");
            }
            Ok(())
        }
        Err(e) if is_duplicate_rejection(e) => {
            if let Some(id) = q.id {
                let _ = db.mark_synced(id, "cloudlog");
            }
            Ok(())
        }
        Err(e) => Err(e.clone()),
    }
}

/// Uploads every QSO not yet marked synced to Cloudlog/Wavelog/Nextlog
/// (`synced_cloudlog = 0`), one `api/qso` POST per QSO. Stops early once a
/// Station ID / API key rejection is seen -- every remaining row would fail
/// the same way (see this module's top doc comment); the rest stay queued
/// for the next run.
pub fn upload_pending(db: &Db) -> CloudlogUploadResult {
    let address = db.get_config(CONFIG_ADDRESS).unwrap_or_default();
    let api_key = db.get_config(CONFIG_API_KEY).unwrap_or_default();
    let station_id = db.get_config(CONFIG_STATION_ID).unwrap_or_default();
    let Some(base) = normalize_base(&address) else {
        return CloudlogUploadResult { error: Some("no server address configured".to_string()), ..Default::default() };
    };
    if api_key.trim().is_empty() {
        return CloudlogUploadResult { error: Some("no API key configured".to_string()), ..Default::default() };
    }
    let pending = db.qsos_pending_cloudlog_upload();
    let mut result = CloudlogUploadResult::default();
    for q in &pending {
        match upload_and_mark_one(db, &base, api_key.trim(), station_id.trim(), q) {
            Ok(()) => result.uploaded += 1,
            Err(e) => {
                let stops_batch = failure_stops_batch(&e);
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

/// Auto-upload hook for a QSO that was just logged -- same per-row logic as
/// [`upload_pending`], for exactly one record. Callers (engine.rs) run this
/// on a spawned thread, never the engine's own run-loop thread.
pub fn upload_one_logged(db: &Db, address: &str, api_key: &str, station_id: &str, q: &QsoRecord) {
    let Some(base) = normalize_base(address) else { return };
    if api_key.trim().is_empty() {
        return;
    }
    let _ = upload_and_mark_one(db, &base, api_key.trim(), station_id.trim(), q);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_adds_http_for_lan_hosts_and_https_otherwise() {
        assert_eq!(normalize_base("192.168.15.20/cloudlog").unwrap(), "http://192.168.15.20/cloudlog/");
        assert_eq!(normalize_base("nas/wavelog").unwrap(), "http://nas/wavelog/");
        assert_eq!(normalize_base("log.example.com").unwrap(), "https://log.example.com/");
        assert_eq!(normalize_base("http://192.168.186.73/wavelog/").unwrap(), "http://192.168.186.73/wavelog/");
    }

    #[test]
    fn normalize_base_handles_blank_and_whitespace() {
        assert!(normalize_base("").is_none());
        assert!(normalize_base("   ").is_none());
        assert_eq!(normalize_base("  log.example.com  ").unwrap(), "https://log.example.com/");
    }

    #[test]
    fn normalize_base_collapses_a_double_trailing_slash() {
        assert_eq!(normalize_base("https://log.example.com//").unwrap(), "https://log.example.com/");
    }

    #[test]
    fn looks_like_lan_host_covers_ipv4_localhost_and_single_label() {
        assert!(looks_like_lan_host("192.168.1.1"));
        assert!(looks_like_lan_host("localhost"));
        assert!(looks_like_lan_host("raspberrypi"));
        assert!(looks_like_lan_host("nas.local"));
        assert!(!looks_like_lan_host("log.example.com"));
    }

    // Each test below uses its own unique base URL (never reused elsewhere
    // in this file) so they can't collide on `WORKING_BASE`'s shared,
    // process-global cache when the test binary runs them in parallel --
    // no #[serial]/locking needed.

    #[test]
    fn candidates_tries_plain_then_rewritten_form_by_default() {
        let out = candidates("https://cl-test-plain.example.com/", "api/auth/KEY");
        assert_eq!(
            out,
            vec![
                "https://cl-test-plain.example.com/api/auth/KEY",
                "https://cl-test-plain.example.com/index.php/api/auth/KEY",
            ]
        );
    }

    #[test]
    fn candidates_returns_a_single_url_when_base_already_has_index_php() {
        let out = candidates("https://cl-test-idxphp.example.com/index.php/", "api/auth/KEY");
        assert_eq!(out, vec!["https://cl-test-idxphp.example.com/index.php/api/auth/KEY"]);
    }

    #[test]
    fn remember_working_moves_the_rewritten_form_to_the_front_next_time() {
        let base = "https://cl-test-remember.example.com/";
        remember_working(base, &format!("{base}index.php/api/auth/KEY"));
        let out = candidates(base, "api/qso");
        assert_eq!(
            out,
            vec![
                "https://cl-test-remember.example.com/index.php/api/qso",
                "https://cl-test-remember.example.com/api/qso",
            ]
        );
    }

    #[test]
    fn describe_http_failure_flattens_whitespace_and_truncates() {
        assert_eq!(describe_http_failure(401, ""), "HTTP 401");
        assert_eq!(describe_http_failure(401, "  bad\nkey  "), "HTTP 401: bad key");
        let long = "x".repeat(300);
        let out = describe_http_failure(500, &long);
        assert!(out.starts_with("HTTP 500: "));
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < 300);
    }

    #[test]
    fn is_duplicate_rejection_matches_the_real_wavelog_reason() {
        assert!(is_duplicate_rejection("HTTP 400: {\"status\":\"failed\",\"reason\":\"Duplicate for callsign...\"}"));
        assert!(!is_duplicate_rejection("HTTP 401: bad key"));
        assert!(!is_duplicate_rejection(""));
    }

    #[test]
    fn classify_failure_recognizes_station_id_and_api_key_reasons() {
        assert_eq!(
            classify_failure("HTTP 401: {\"reason\":\"station id does not belong to the API key owner.\"}"),
            FailureKind::StationId
        );
        assert_eq!(classify_failure("HTTP 401: {\"reason\":\"API Key invalid\"}"), FailureKind::ApiKey);
        assert_eq!(classify_failure("HTTP 500: proxy error"), FailureKind::Other);
        assert_eq!(classify_failure(""), FailureKind::Other);
    }

    #[test]
    fn failure_stops_batch_matches_classify_failure() {
        assert!(failure_stops_batch("HTTP 401: {\"reason\":\"station id does not belong to the API key owner.\"}"));
        assert!(!failure_stops_batch("HTTP 500: proxy error"));
    }

    #[test]
    fn interpret_auth_response_reads_valid_and_rights() {
        assert!(interpret_auth_response("<status>Valid</status><rights>rw</rights>").ok);
        let ro = interpret_auth_response("<status>Valid</status><rights>ro</rights>");
        assert!(!ro.ok);
        assert!(ro.detail.unwrap().contains("read-only"));
        let bad = interpret_auth_response("<status>Invalid</status>");
        assert!(!bad.ok);
        let garbage = interpret_auth_response("<html>not cloudlog</html>");
        assert!(!garbage.ok);
    }

    #[test]
    fn parse_station_profiles_reads_fields_and_tolerates_garbage() {
        let json = r#"[{"station_id":"3","station_profile_name":"Home","station_callsign":"K1ABC","station_gridsquare":"FN31"}]"#;
        let stations = parse_station_profiles(json);
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].station_id, "3");
        assert_eq!(stations[0].display_label(), "3 - Home (K1ABC, FN31)");
        assert!(parse_station_profiles("").is_empty());
        assert!(parse_station_profiles("not json").is_empty());
        assert!(parse_station_profiles("{}").is_empty());
    }

    #[test]
    fn check_connection_requires_an_address_and_a_key() {
        let db = Db::open_in_memory().unwrap();
        let check = check_connection(&db);
        assert!(!check.ok);
        assert_eq!(check.detail.unwrap(), "no server address configured");

        db.set_config(CONFIG_ADDRESS, "log.example.com").unwrap();
        let check = check_connection(&db);
        assert!(!check.ok);
        assert_eq!(check.detail.unwrap(), "no API key configured");
    }

    #[test]
    fn upload_pending_is_a_no_op_without_address_or_key() {
        let db = Db::open_in_memory().unwrap();
        let result = upload_pending(&db);
        assert_eq!(result.uploaded, 0);
        assert!(result.error.is_some());

        db.set_config(CONFIG_ADDRESS, "log.example.com").unwrap();
        let result = upload_pending(&db);
        assert!(result.error.unwrap().contains("API key"));
    }

    #[test]
    fn upload_pending_is_a_no_op_when_nothing_is_pending() {
        let db = Db::open_in_memory().unwrap();
        db.set_config(CONFIG_ADDRESS, "log.example.com").unwrap();
        db.set_config(CONFIG_API_KEY, "fake-key-not-used-because-nothing-pending").unwrap();
        let result = upload_pending(&db);
        assert_eq!(result.uploaded, 0);
        assert_eq!(result.failed, 0);
        assert!(result.error.is_none());
    }

    #[test]
    fn display_label_degrades_gracefully_with_missing_fields() {
        let bare = StationProfile { station_id: "3".to_string(), ..Default::default() };
        assert_eq!(bare.display_label(), "3");
    }
}
