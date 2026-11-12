//! PSK Reporter spot fetching for the Map screen's "who's hearing me" /
//! "who I heard" overlay. Ported from Android's `PskReporterClient.kt`,
//! adapted rather than transliterated: the HTTP/XML mechanics match, but
//! this module never computes lat/lon (the frontend already converts a
//! grid locator to coordinates for every other marker on the map) and
//! resolves a spot's band via the existing `bands::band_for_freq_hz`
//! instead of porting a second band-range table.
//!
//! Confirmed against the real PSK Reporter XML fixture
//! (`ft8af/app/src/test/resources/pskreporter/reception-report.xml` on
//! Android's `dev` branch) before writing this: the root element is
//! `<receptionReports>`, containing `<activeReceiver .../>` siblings (not
//! matched here) alongside the real `<receptionReport .../>` rows, which
//! are self-closing (`quick_xml::events::Event::Empty`, never
//! `Start`+`End`). An attribute that's present but empty (e.g.
//! `receiverLocator=""`) counts as missing, same as an absent one.

use std::sync::Mutex;
use std::time::Duration;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Serialize;

use crate::bands;

const BASE_URL: &str = "https://retrieve.pskreporter.info/query";
const APP_CONTACT: &str = "ft8af@example.org";
const USER_AGENT: &str = "ft8af-desktop/1.0";
/// Client-side cooldown between fetches, skippable via `force`. Matches
/// Android's own value: a hair under the 5-minute poll interval the
/// frontend uses, as defense in depth under that loop.
const COOLDOWN_MS: i64 = 290_000;
/// How long a 429/503 response blocks further fetches, including forced
/// ones -- `force` bypasses the cooldown above but never this.
const RATE_LIMIT_COOLDOWN_MS: i64 = 15 * 60_000;
const MAX_SPOTS: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct PskReporterSpot {
    pub callsign: String,
    pub grid: String,
    /// ADIF band name (e.g. "20m"), or "unknown" -- see `bands::band_for_freq_hz`.
    pub band: String,
    pub frequency_hz: i64,
    pub snr: i32,
    pub mode: String,
    pub flow_start_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct PskFetchResult {
    /// `None` if the fetch was skipped (cooldown/rate-limit) or failed --
    /// callers should keep showing whatever spots they already have, not
    /// clear them, matching Android's own "no flicker" behavior.
    pub spots: Option<Vec<PskReporterSpot>>,
    pub error: Option<String>,
}

/// Whether a fetch should actually run right now. A pure predicate (no
/// I/O, no clock dependency) -- mirrors this codebase's own established
/// idiom for testable gating logic (`engine.rs`'s `decode_dispatch_due`/
/// `boundary_tx_ready`) rather than porting Android's injectable-clock
/// seam, since plain `i64` millis need no injection to unit-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchGate {
    Proceed,
    SkippedCooldown,
    SkippedRateLimited,
}

/// Checks the rate-limit back-off *before* the cooldown, unconditionally --
/// a 429/503 lockout is never bypassable, not even by `force`. Getting
/// this order backwards would let `force=true` defeat rate-limiting.
fn fetch_gate(
    now_ms: i64,
    force: bool,
    last_fetch_at_ms: Option<i64>,
    rate_limited_until_ms: Option<i64>,
) -> FetchGate {
    if let Some(until) = rate_limited_until_ms {
        if now_ms < until {
            return FetchGate::SkippedRateLimited;
        }
    }
    if !force {
        if let Some(last) = last_fetch_at_ms {
            if now_ms - last < COOLDOWN_MS {
                return FetchGate::SkippedCooldown;
            }
        }
    }
    FetchGate::Proceed
}

struct Inner {
    last_fetch_at_ms: Option<i64>,
    rate_limited_until_ms: Option<i64>,
    last_error: Option<String>,
}

pub struct PskReporterState {
    inner: Mutex<Inner>,
}

impl Default for PskReporterState {
    fn default() -> Self {
        PskReporterState {
            inner: Mutex::new(Inner {
                last_fetch_at_ms: None,
                rate_limited_until_ms: None,
                last_error: None,
            }),
        }
    }
}

impl PskReporterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `call`, `mode`: the caller always resolves these to concrete
    /// strings -- there's no synchronous "what's the engine's live state"
    /// read the way Android's `GeneralVariables` singleton allows, so
    /// unlike `fetchSpotsForMe`'s `mode: String? = null` default, this
    /// never has an "ask the engine" path.
    pub fn fetch_spots(
        &self,
        call: &str,
        seconds_back: i64,
        force: bool,
        mode: &str,
        by_receiver: bool,
    ) -> PskFetchResult {
        let now_ms = now_ms();
        let gate = {
            let inner = self.inner.lock().unwrap();
            fetch_gate(now_ms, force, inner.last_fetch_at_ms, inner.rate_limited_until_ms)
        };
        match gate {
            FetchGate::SkippedRateLimited => {
                // Deliberately not cleared: the banner should stay visible
                // while backed off.
                let inner = self.inner.lock().unwrap();
                return PskFetchResult { spots: None, error: inner.last_error.clone() };
            }
            FetchGate::SkippedCooldown => {
                // A benign skip clears any stale error so it doesn't linger
                // through normal throttling.
                self.inner.lock().unwrap().last_error = None;
                return PskFetchResult { spots: None, error: None };
            }
            FetchGate::Proceed => {}
        }
        self.inner.lock().unwrap().last_fetch_at_ms = Some(now_ms);

        let params = query_params(call, seconds_back, mode, by_receiver);
        let mut req = ureq::get(BASE_URL);
        for (k, v) in &params {
            req = req.query(k, v);
        }
        let resp = req
            .set("User-Agent", USER_AGENT)
            .timeout(Duration::from_secs(8))
            .call();

        match resp {
            Ok(r) => match r.into_string() {
                Ok(body) => {
                    let spots = parse_spots(&body, by_receiver);
                    self.inner.lock().unwrap().last_error = None;
                    PskFetchResult { spots: Some(spots), error: None }
                }
                Err(e) => self.record_error(format!("{e}")),
            },
            Err(ureq::Error::Status(code, _)) if code == 429 || code == 503 => {
                let mut inner = self.inner.lock().unwrap();
                inner.rate_limited_until_ms = Some(now_ms + RATE_LIMIT_COOLDOWN_MS);
                inner.last_error = Some(format!("rate-limited ({code})"));
                PskFetchResult { spots: None, error: inner.last_error.clone() }
            }
            Err(e) => self.record_error(format!("{e}")),
        }
    }

    fn record_error(&self, message: String) -> PskFetchResult {
        self.inner.lock().unwrap().last_error = Some(message.clone());
        PskFetchResult { spots: None, error: Some(message) }
    }
}

/// The query parameters for one fetch, as `(name, value)` pairs -- a pure,
/// testable step split out of `fetch_spots` specifically so the
/// sender-vs-receiver param selection and the "mode is always present,
/// never omitted" rule (unlike Android's `mode: String? = null` default,
/// this module's caller always resolves a concrete mode string -- see
/// `fetch_spots`'s doc comment) can be checked without a live network call.
fn query_params(call: &str, seconds_back: i64, mode: &str, by_receiver: bool) -> Vec<(String, String)> {
    let who_param = if by_receiver { "receiverCallsign" } else { "senderCallsign" };
    vec![
        (who_param.to_string(), call.to_string()),
        ("flowStartSeconds".to_string(), format!("-{seconds_back}")),
        ("rronly".to_string(), "1".to_string()),
        ("mode".to_string(), mode.to_string()),
        ("appcontact".to_string(), APP_CONTACT.to_string()),
    ]
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parses a PSK Reporter `<receptionReports>` XML body into spots, dropping
/// (not failing on) any row missing a callsign or a usable locator/frequency,
/// capped to the most recent `MAX_SPOTS` assuming chronological order.
///
/// `by_receiver` picks which side of each report is "the other station" to
/// plot: when `false` (the default "who heard me" direction), every row's
/// `senderCallsign` is the operator themself (that's what was queried for),
/// so the receiver is the other station; when `true` ("who I heard"), the
/// operator is the receiver in every row, so the sender is the other
/// station. Getting this backwards would plot the operator's own callsign
/// as every spot in the "who heard me" direction.
fn parse_spots(body: &str, by_receiver: bool) -> Vec<PskReporterSpot> {
    let mut reader = Reader::from_str(body);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e)) if e.name().as_ref() == b"receptionReport" => {
                let mut sender_call = String::new();
                let mut receiver_call = String::new();
                let mut sender_grid = String::new();
                let mut receiver_grid = String::new();
                let mut freq_hz: Option<i64> = None;
                let mut snr: Option<i32> = None;
                let mut mode = String::new();
                let mut flow_start: Option<i64> = None;
                for attr in e.attributes().flatten() {
                    let value = match attr.unescape_value() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    match attr.key.as_ref() {
                        b"senderCallsign" => sender_call = value.into_owned(),
                        b"receiverCallsign" => receiver_call = value.into_owned(),
                        b"senderLocator" => sender_grid = value.into_owned(),
                        b"receiverLocator" => receiver_grid = value.into_owned(),
                        b"frequency" => freq_hz = value.parse().ok(),
                        b"sNR" => snr = value.parse().ok(),
                        b"mode" => mode = value.into_owned(),
                        b"flowStartSeconds" => flow_start = value.parse().ok(),
                        _ => {}
                    }
                }
                if sender_call.is_empty() || receiver_call.is_empty() {
                    continue;
                }
                let (Some(freq_hz), Some(snr), Some(flow_start)) = (freq_hz, snr, flow_start) else {
                    continue;
                };
                // The caller (the Map screen) plots "the other station" --
                // whichever side isn't the operator -- so both locators must
                // resolve for either direction to be drawable; a row with
                // either side blank is dropped rather than guessing.
                if sender_grid.is_empty() || receiver_grid.is_empty() {
                    continue;
                }
                let (other_call, other_grid) = if by_receiver {
                    (sender_call, sender_grid)
                } else {
                    (receiver_call, receiver_grid)
                };
                out.push(PskReporterSpot {
                    callsign: other_call,
                    grid: other_grid,
                    band: bands::band_for_freq_hz(freq_hz.max(0) as u64).to_string(),
                    frequency_hz: freq_hz,
                    snr,
                    mode,
                    flow_start_seconds: flow_start,
                });
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if out.len() > MAX_SPOTS {
        let drop = out.len() - MAX_SPOTS;
        out.drain(0..drop);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_gate_proceeds_with_no_history() {
        assert_eq!(fetch_gate(1_000_000, false, None, None), FetchGate::Proceed);
    }

    #[test]
    fn fetch_gate_cooldown_blocks_a_too_soon_refetch() {
        assert_eq!(
            fetch_gate(1_000_000, false, Some(1_000_000 - COOLDOWN_MS + 1), None),
            FetchGate::SkippedCooldown
        );
        assert_eq!(
            fetch_gate(1_000_000, false, Some(1_000_000 - COOLDOWN_MS), None),
            FetchGate::Proceed
        );
    }

    #[test]
    fn force_bypasses_cooldown_but_not_rate_limit() {
        let last_fetch = Some(1_000_000 - 1); // well within cooldown
        assert_eq!(fetch_gate(1_000_000, true, last_fetch, None), FetchGate::Proceed);

        let rate_limited_until = Some(1_000_000 + 1);
        assert_eq!(
            fetch_gate(1_000_000, true, last_fetch, rate_limited_until),
            FetchGate::SkippedRateLimited
        );
    }

    #[test]
    fn rate_limit_expires_after_its_own_window() {
        assert_eq!(fetch_gate(1_000_000, false, None, Some(999_999)), FetchGate::Proceed);
        assert_eq!(fetch_gate(1_000_000, false, None, Some(1_000_001)), FetchGate::SkippedRateLimited);
    }

    const FIXTURE: &str = r#"<?xml version="1.0" encoding="ISO-8859-1" ?>
<receptionReports currentSeconds="1747756800">
    <activeReceiver callsign="VE3XY" locator="FN03" frequency="14076000"/>
    <receptionReport receiverCallsign="VE3XY" receiverLocator="FN03ab" senderCallsign="W1AW"
        senderLocator="FN31" frequency="14076234" flowStartSeconds="1747756700" mode="FT8"
        isSender="0" sNR="-08"/>
    <receptionReport receiverCallsign="VE3XY" receiverLocator="" senderCallsign="K1ABC"
        senderLocator="EN37" frequency="14076000" flowStartSeconds="1747756701" mode="FT8"
        isSender="0" sNR="3"/>
    <receptionReport receiverCallsign="" receiverLocator="FN03ab" senderCallsign=""
        senderLocator="FN42" frequency="14076000" flowStartSeconds="1747756702" mode="FT8"
        isSender="0" sNR="0"/>
    <receptionReport receiverCallsign="VE3XY" receiverLocator="FN03ab" senderCallsign="N0CALL"
        senderLocator="EM12" frequency="not-a-number" flowStartSeconds="1747756703" mode="FT8"
        isSender="0" sNR="1"/>
</receptionReports>"#;

    #[test]
    fn parses_the_real_fixture_shape_ignoring_active_receiver() {
        // by_receiver=false ("who heard me"): we are the sender (W1AW) in
        // every row, so the surviving spot's identity is the *receiver*
        // (VE3XY), the station that actually heard us.
        let spots = parse_spots(FIXTURE, false);
        // Only the first report survives: the empty-locator, empty-callsign,
        // and unparseable-frequency rows are all dropped; activeReceiver is
        // never matched as a receptionReport.
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].callsign, "VE3XY");
        assert_eq!(spots[0].grid, "FN03ab");
        assert_eq!(spots[0].band, "20m");
        assert_eq!(spots[0].snr, -8);
        assert_eq!(spots[0].frequency_hz, 14_076_234);
        assert_eq!(spots[0].flow_start_seconds, 1_747_756_700);
    }

    #[test]
    fn by_receiver_true_plots_the_sender_instead() {
        // by_receiver=true ("who I heard"): we are the receiver (VE3XY) in
        // every row, so the surviving spot's identity flips to the sender
        // (W1AW), the station we actually heard.
        let spots = parse_spots(FIXTURE, true);
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].callsign, "W1AW");
        assert_eq!(spots[0].grid, "FN31");
    }

    #[test]
    fn drops_a_row_with_an_empty_locator_attribute() {
        let xml = r#"<receptionReports>
            <receptionReport senderCallsign="K1ABC" senderLocator="EN37"
                receiverCallsign="W1AW" receiverLocator="" frequency="14074000"
                flowStartSeconds="1" mode="FT8" sNR="0"/>
        </receptionReports>"#;
        assert!(parse_spots(xml, false).is_empty());
    }

    #[test]
    fn caps_at_500_keeping_the_most_recent() {
        let mut xml = String::from("<receptionReports>");
        for i in 0..600 {
            xml.push_str(&format!(
                r#"<receptionReport senderCallsign="K{i}" senderLocator="EN37"
                    receiverCallsign="W1AW" receiverLocator="FN31" frequency="14074000"
                    flowStartSeconds="{i}" mode="FT8" sNR="0"/>"#
            ));
        }
        xml.push_str("</receptionReports>");
        // by_receiver=true so the per-row sender (K{i}) is what's plotted,
        // letting this test tell rows apart by callsign.
        let spots = parse_spots(&xml, true);
        assert_eq!(spots.len(), MAX_SPOTS);
        // The first 100 (0..100) were dropped to stay under the cap.
        assert_eq!(spots[0].callsign, "K100");
        assert_eq!(spots.last().unwrap().callsign, "K599");
    }

    #[test]
    fn query_params_selects_sender_vs_receiver_by_direction() {
        let sender_side = query_params("K1ABC", 3600, "FT8", false);
        assert!(sender_side.contains(&("senderCallsign".to_string(), "K1ABC".to_string())));
        assert!(!sender_side.iter().any(|(k, _)| k == "receiverCallsign"));

        let receiver_side = query_params("K1ABC", 3600, "FT8", true);
        assert!(receiver_side.contains(&("receiverCallsign".to_string(), "K1ABC".to_string())));
        assert!(!receiver_side.iter().any(|(k, _)| k == "senderCallsign"));
    }

    #[test]
    fn query_params_always_includes_mode() {
        // Unlike Android's mode: String? = null default, this module's
        // caller always resolves a concrete mode string -- there is no
        // "omitted" path to test for, only that it's always present.
        let params = query_params("K1ABC", 3600, "FT4", false);
        assert!(params.contains(&("mode".to_string(), "FT4".to_string())));
    }
}
