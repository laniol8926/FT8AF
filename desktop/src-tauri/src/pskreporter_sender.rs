//! Uploads decoded FT8/FT4 spots to PSKReporter using the IPFIX binary
//! protocol over UDP (`report.pskreporter.info:4739`), ported from
//! Android's `PskReporterSender.kt` byte-for-byte (template/field IDs,
//! enterprise number, padding, dedup window, send interval/jitter).
//!
//! This is the "who I heard" half of PSK Reporter -- the only half a
//! station's own software can ever produce. The other half ("who heard
//! me") is populated entirely by *other* stations' own PSK-Reporter-
//! enabled software reporting hearing this one; nothing here can affect
//! that side. Gated on the `enablePskReporter` config toggle
//! (`LoggingSettings.tsx` already persists this for real).
//!
//! **Lifecycle, simplified from Android's**: Android's `start`/`stop`
//! tie to recording/screen lifecycle (an Android activity can be
//! backgrounded). Desktop's engine is a single long-lived object for the
//! process's lifetime with no equivalent "paused" state, so this spawns
//! one background flush thread at construction and runs it for the
//! engine's lifetime -- no `stop()` needed. A `flush` on an empty queue
//! is a cheap no-op either way, matching Android's own early return.

use std::collections::{HashMap, VecDeque};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::dsp::DecodedMessage;
use crate::mode::Mode;

pub const CONFIG_ENABLE: &str = "enablePskReporter";

const HOST: &str = "report.pskreporter.info";
const PORT: u16 = 4739;
const MAX_DATAGRAM: usize = 1400;
const SEND_INTERVAL_MS: u64 = 5 * 60 * 1000;
const SEND_JITTER_MS: i64 = 30_000;
const DEDUP_WINDOW_MS: i64 = 5 * 60 * 1000;
const ENTERPRISE_NUMBER: u32 = 30351;
const IPFIX_VERSION: u16 = 0x000A;

const TEMPLATE_SET_ID_OPTIONS: u16 = 0x0003;
const TEMPLATE_SET_ID_DATA: u16 = 0x0002;
const RECEIVER_TEMPLATE_ID: u16 = 0x50E2;
const SENDER_TEMPLATE_ID: u16 = 0x50E3;
const RECEIVER_DATA_SET_ID: u16 = RECEIVER_TEMPLATE_ID;
const SENDER_DATA_SET_ID: u16 = SENDER_TEMPLATE_ID;

const FT_RX_CALLSIGN: u16 = 2;
const FT_RX_LOCATOR: u16 = 4;
const FT_DECODING_SW: u16 = 8;
const FT_RX_ANTENNA: u16 = 9;
const FT_RX_RIG: u16 = 13;
const FT_TX_CALLSIGN: u16 = 1;
const FT_FREQUENCY: u16 = 5;
const FT_SNR: u16 = 6;
const FT_MODE: u16 = 10;
const FT_TX_LOCATOR: u16 = 3;
const FT_INFO_SOURCE: u16 = 11;
/// Standard IANA IPFIX element 150 -- no enterprise bit, no enterprise
/// number (4-byte field descriptor, unlike every other field here).
const FT_FLOW_START: u16 = 150;

#[derive(Debug, Clone, PartialEq)]
pub struct SpotRecord {
    pub sender_callsign: String,
    pub frequency_hz: i64,
    pub snr: i32,
    pub mode: String,
    pub sender_locator: Option<String>,
    pub flow_start_seconds: i64,
}

/// Decide whether a decoded message's grid slot is a genuine Maidenhead
/// locator worth reporting, or a value that must be dropped. FT8's RR73
/// sign-off token happens to look like a 4-char grid (R,R valid field
/// letters; 7,3 valid square digits) but is never a real location --
/// reporting it would inject a bogus locator (a phantom cell in the
/// Arctic Ocean) into PSKReporter's global spot database.
pub fn reportable_locator(grid: &str) -> Option<&str> {
    if grid.len() >= 4 && !grid.eq_ignore_ascii_case("RR73") {
        Some(grid)
    } else {
        None
    }
}

fn encode_var_string(s: &str) -> Vec<u8> {
    let utf8 = s.as_bytes();
    if utf8.len() < 255 {
        let mut out = Vec::with_capacity(1 + utf8.len());
        out.push(utf8.len() as u8);
        out.extend_from_slice(utf8);
        out
    } else {
        let mut out = Vec::with_capacity(3 + utf8.len());
        out.push(0xFF);
        out.push((utf8.len() >> 8) as u8);
        out.push((utf8.len() & 0xFF) as u8);
        out.extend_from_slice(utf8);
        out
    }
}

fn pad_len(len: usize) -> usize {
    (4 - (len % 4)) % 4
}

fn encode_receiver_template() -> Vec<u8> {
    // Options template set: templateId + fieldCount + scopeFieldCount, then
    // 5 enterprise-specific fields (8 bytes each: type|0x8000, length, enterprise).
    let field_count = 5u16;
    let template_record_len = 2 + 2 + 2 + (field_count as usize * 8);
    let set_len = 4 + template_record_len;
    let padding = pad_len(set_len);
    let mut buf = Vec::with_capacity(set_len + padding);
    buf.extend_from_slice(&TEMPLATE_SET_ID_OPTIONS.to_be_bytes());
    buf.extend_from_slice(&((set_len + padding) as u16).to_be_bytes());
    buf.extend_from_slice(&RECEIVER_TEMPLATE_ID.to_be_bytes());
    buf.extend_from_slice(&field_count.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // scope field count
    for field in [FT_RX_CALLSIGN, FT_RX_LOCATOR, FT_DECODING_SW, FT_RX_ANTENNA, FT_RX_RIG] {
        buf.extend_from_slice(&(field | 0x8000).to_be_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes()); // variable length
        buf.extend_from_slice(&ENTERPRISE_NUMBER.to_be_bytes());
    }
    buf.resize(buf.len() + padding, 0);
    buf
}

fn encode_sender_template() -> Vec<u8> {
    // Data template set: templateId + fieldCount, then 6 enterprise-specific
    // fields (8 bytes each) + flowStartSeconds (standard, 4-byte descriptor).
    let field_count = 7u16;
    let template_record_len = 2 + 2 + (6 * 8) + 4;
    let set_len = 4 + template_record_len;
    let padding = pad_len(set_len);
    let mut buf = Vec::with_capacity(set_len + padding);
    buf.extend_from_slice(&TEMPLATE_SET_ID_DATA.to_be_bytes());
    buf.extend_from_slice(&((set_len + padding) as u16).to_be_bytes());
    buf.extend_from_slice(&SENDER_TEMPLATE_ID.to_be_bytes());
    buf.extend_from_slice(&field_count.to_be_bytes());
    for (field, len) in [
        (FT_TX_CALLSIGN, 0xFFFFu16),
        (FT_FREQUENCY, 4),
        (FT_SNR, 1),
        (FT_MODE, 0xFFFF),
        (FT_TX_LOCATOR, 0xFFFF),
        (FT_INFO_SOURCE, 1),
    ] {
        buf.extend_from_slice(&(field | 0x8000).to_be_bytes());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&ENTERPRISE_NUMBER.to_be_bytes());
    }
    // flowStartSeconds: standard element, no enterprise bit/number.
    buf.extend_from_slice(&FT_FLOW_START.to_be_bytes());
    buf.extend_from_slice(&4u16.to_be_bytes());
    buf.resize(buf.len() + padding, 0);
    buf
}

fn encode_templates() -> Vec<u8> {
    let mut buf = encode_receiver_template();
    buf.extend(encode_sender_template());
    buf
}

fn encode_receiver_data_set(call: &str, grid: &str, software: &str, antenna: &str, rig: &str) -> Vec<u8> {
    let call_b = encode_var_string(call);
    let grid_b = encode_var_string(grid);
    let sw_b = encode_var_string(software);
    let ant_b = encode_var_string(antenna);
    let rig_b = encode_var_string(rig);
    let body_len = call_b.len() + grid_b.len() + sw_b.len() + ant_b.len() + rig_b.len();
    let set_len = 4 + body_len;
    let padding = pad_len(set_len);
    let mut buf = Vec::with_capacity(set_len + padding);
    buf.extend_from_slice(&RECEIVER_DATA_SET_ID.to_be_bytes());
    buf.extend_from_slice(&((set_len + padding) as u16).to_be_bytes());
    buf.extend_from_slice(&call_b);
    buf.extend_from_slice(&grid_b);
    buf.extend_from_slice(&sw_b);
    buf.extend_from_slice(&ant_b);
    buf.extend_from_slice(&rig_b);
    buf.resize(buf.len() + padding, 0);
    buf
}

fn encode_sender_record(spot: &SpotRecord) -> Vec<u8> {
    let call_b = encode_var_string(&spot.sender_callsign);
    let mode_b = encode_var_string(&spot.mode);
    let loc_b = encode_var_string(spot.sender_locator.as_deref().unwrap_or(""));
    let mut buf = Vec::with_capacity(call_b.len() + 4 + 1 + mode_b.len() + loc_b.len() + 1 + 4);
    buf.extend_from_slice(&call_b);
    buf.extend_from_slice(&(spot.frequency_hz as i32).to_be_bytes());
    buf.push(spot.snr.clamp(-128, 127) as i8 as u8);
    buf.extend_from_slice(&mode_b);
    buf.extend_from_slice(&loc_b);
    buf.push(1); // informationSource = 1 (automatic)
    buf.extend_from_slice(&(spot.flow_start_seconds as i32).to_be_bytes());
    buf
}

/// Builds one or more IPFIX datagrams for `spots`, each kept under
/// [`MAX_DATAGRAM`] bytes. `sequence_number`/`observation_domain_id` are
/// per-packet IPFIX header fields; `include_templates` controls whether
/// the (larger) template sets are prefixed to the first packet this call
/// produces -- callers re-send templates periodically (see
/// `PskReporterSender::needs_templates`), not on every single packet.
fn build_packets(
    rx_call: &str,
    rx_grid: &str,
    software: &str,
    antenna: &str,
    rig: &str,
    spots: &[SpotRecord],
    include_templates: bool,
    sequence_number: &AtomicU32,
    observation_domain_id: u32,
) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut remaining: Vec<&SpotRecord> = spots.iter().collect();
    let mut first = true;

    while !remaining.is_empty() {
        let template_bytes = if include_templates && first { encode_templates() } else { Vec::new() };
        let receiver_bytes = encode_receiver_data_set(rx_call, rx_grid, software, antenna, rig);
        first = false;

        let overhead = 16 + template_bytes.len() + receiver_bytes.len();
        let budget = MAX_DATAGRAM.saturating_sub(overhead);

        let mut sender_records = Vec::new();
        let mut sender_set_size = 4usize; // set header (setId + length)
        let mut used = 0usize;
        for spot in &remaining {
            let rec = encode_sender_record(spot);
            if sender_set_size + rec.len() > budget && used > 0 {
                break;
            }
            sender_set_size += rec.len();
            sender_records.push(rec);
            used += 1;
        }
        // A single spot that alone exceeds budget (pathological, e.g. a
        // very long callsign/grid) still goes out alone rather than
        // looping forever with nothing consumed.
        if used == 0 && !remaining.is_empty() {
            let rec = encode_sender_record(remaining[0]);
            sender_set_size += rec.len();
            sender_records.push(rec);
            used = 1;
        }
        remaining.drain(0..used);

        let sender_padding = pad_len(sender_set_size);
        let padded_sender_set_size = sender_set_size + sender_padding;
        let total_len = 16 + template_bytes.len() + receiver_bytes.len() + padded_sender_set_size;

        let mut buf = Vec::with_capacity(total_len);
        let export_time = (crate::util::now_unix_ms() / 1000) as u32;
        buf.extend_from_slice(&IPFIX_VERSION.to_be_bytes());
        buf.extend_from_slice(&(total_len as u16).to_be_bytes());
        buf.extend_from_slice(&export_time.to_be_bytes());
        buf.extend_from_slice(&sequence_number.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        buf.extend_from_slice(&observation_domain_id.to_be_bytes());

        buf.extend_from_slice(&template_bytes);
        buf.extend_from_slice(&receiver_bytes);
        buf.extend_from_slice(&SENDER_DATA_SET_ID.to_be_bytes());
        buf.extend_from_slice(&(padded_sender_set_size as u16).to_be_bytes());
        for rec in &sender_records {
            buf.extend_from_slice(rec);
        }
        buf.resize(buf.len() + sender_padding, 0);

        packets.push(buf);
    }
    packets
}

struct Inner {
    queue: Mutex<VecDeque<SpotRecord>>,
    /// "CALL|BAND_MHZ" -> last-reported epoch ms.
    dedup: Mutex<HashMap<String, i64>>,
    packets_sent: AtomicUsize,
    last_template_sent_ms: AtomicI64,
    sequence_number: AtomicU32,
    observation_domain_id: u32,
    /// The connected rig's display name (`RigConnection::name()`), matching
    /// Android's `GeneralVariables.myRigName` -- live runtime state set by
    /// the engine on connect/disconnect, not a config preference. The
    /// background flush thread has no other way to reach the engine's own
    /// `self.rig`, so this is the shared handoff point.
    rig_name: Mutex<Option<String>>,
    /// Swappable for tests -- real sends go out over a UDP socket; tests
    /// capture the bytes instead.
    send_datagram: Box<dyn Fn(&[u8]) + Send + Sync>,
}

/// Cheaply `Clone`-able (an `Arc` around the shared state) so the
/// background flush thread and the engine's hot `enqueue` path can share
/// the same queue/dedup map.
#[derive(Clone)]
pub struct PskReporterSender {
    inner: Arc<Inner>,
}

impl PskReporterSender {
    pub fn new() -> Self {
        Self::with_sender(Box::new(send_udp))
    }

    fn with_sender(send_datagram: Box<dyn Fn(&[u8]) + Send + Sync>) -> Self {
        PskReporterSender {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                dedup: Mutex::new(HashMap::new()),
                packets_sent: AtomicUsize::new(0),
                last_template_sent_ms: AtomicI64::new(0),
                sequence_number: AtomicU32::new(0),
                observation_domain_id: rand_u32(),
                rig_name: Mutex::new(None),
                send_datagram,
            }),
        }
    }

    /// Called by the engine whenever it connects/disconnects a rig, so the
    /// next flush's "rigInformation" field reflects live state.
    pub fn set_rig_name(&self, name: Option<String>) {
        *self.inner.rig_name.lock().unwrap() = name;
    }

    pub fn rig_name(&self) -> Option<String> {
        self.inner.rig_name.lock().unwrap().clone()
    }

    /// Spawns the periodic flush thread (5 min +/- 30s jitter, matching
    /// Android) for this engine's lifetime. `station` is called fresh on
    /// every flush so a live Settings change (callsign, grid, antenna,
    /// rig) takes effect without needing a restart.
    pub fn start(&self, station: impl Fn() -> StationInfo + Send + Sync + 'static) {
        let sender = self.clone();
        std::thread::Builder::new()
            .name("ft8af-pskreporter-sender".into())
            .spawn(move || loop {
                let jitter = rand_jitter_ms();
                let sleep_ms = (SEND_INTERVAL_MS as i64 + jitter).max(1_000) as u64;
                std::thread::sleep(Duration::from_millis(sleep_ms));
                let info = station();
                sender.flush(&info);
            })
            .expect("spawn pskreporter-sender thread");
    }

    /// Enqueues every reportable decode in `messages`. Mirrors Android's
    /// `enqueue`/`toSpotRecord` exactly: skipped when disabled/no
    /// callsign configured, self-calls, free-text (`i3==0 && n3==0`),
    /// empty/hashed (`<...>`) callsigns, and anything already reported
    /// for the same callsign+band within [`DEDUP_WINDOW_MS`].
    pub fn enqueue(
        &self,
        messages: &[DecodedMessage],
        enabled: bool,
        my_call: &str,
        dial_hz: u64,
        mode: Mode,
        flow_start_seconds: i64,
        now_ms: i64,
    ) {
        if !enabled || my_call.trim().is_empty() {
            return;
        }
        let mode_str = mode.display_name();
        let band_mhz = dial_hz / 1_000_000;
        let mut added = 0usize;
        for msg in messages {
            let call = msg.call_from.trim();
            if call.is_empty() || call == "<...>" {
                continue;
            }
            let clean_call = call.replace(['<', '>'], "");
            if clean_call.is_empty() {
                continue;
            }
            if crate::callsign::is_my_callsign(&clean_call, my_call) {
                continue;
            }
            if msg.i3 == 0 && msg.n3 == 0 {
                continue;
            }
            let dedup_key = format!("{}|{band_mhz}", clean_call.to_uppercase());
            if !self.mark_if_fresh(&dedup_key, now_ms) {
                continue;
            }
            let freq_hz = dial_hz as i64 + msg.freq_hz.round() as i64;
            self.inner.queue.lock().unwrap().push_back(SpotRecord {
                sender_callsign: clean_call,
                frequency_hz: freq_hz,
                snr: msg.snr,
                mode: mode_str.to_string(),
                sender_locator: reportable_locator(&msg.grid).map(str::to_string),
                flow_start_seconds,
            });
            added += 1;
        }
        let _ = added;
    }

    fn mark_if_fresh(&self, dedup_key: &str, now_ms: i64) -> bool {
        let mut dedup = self.inner.dedup.lock().unwrap();
        if let Some(&last) = dedup.get(dedup_key) {
            if now_ms - last < DEDUP_WINDOW_MS {
                return false;
            }
        }
        dedup.insert(dedup_key.to_string(), now_ms);
        true
    }

    fn prune_dedup(&self, now_ms: i64) {
        self.inner.dedup.lock().unwrap().retain(|_, &mut last| now_ms - last < DEDUP_WINDOW_MS);
    }

    fn needs_templates(&self, now_ms: i64) -> bool {
        if self.inner.packets_sent.load(Ordering::Relaxed) < 3 {
            return true;
        }
        now_ms - self.inner.last_template_sent_ms.load(Ordering::Relaxed) > 3_600_000
    }

    /// Drains the queue and sends it as one or more IPFIX datagrams.
    /// A no-op (matching Android) when there's nothing queued, or no
    /// callsign is configured.
    pub fn flush(&self, station: &StationInfo) {
        let now_ms = crate::util::now_unix_ms();
        self.prune_dedup(now_ms);

        let spots: Vec<SpotRecord> = self.inner.queue.lock().unwrap().drain(..).collect();
        if spots.is_empty() {
            eprintln!("[ft8af] PSKReporter: flush tick, nothing queued to send");
            return;
        }
        if station.my_call.trim().is_empty() {
            eprintln!("[ft8af] PSKReporter: {} spot(s) queued but no callsign configured, dropping", spots.len());
            return;
        }

        let include_templates = self.needs_templates(now_ms);
        let packets = build_packets(
            &station.my_call,
            &station.my_grid,
            &station.software,
            &station.antenna,
            &station.rig,
            &spots,
            include_templates,
            &self.inner.sequence_number,
            self.inner.observation_domain_id,
        );
        for pkt in &packets {
            (self.inner.send_datagram)(pkt);
        }
        eprintln!(
            "[ft8af] PSKReporter: sent {} packet(s) with {} spot(s) as {}",
            packets.len(),
            spots.len(),
            station.my_call,
        );
        self.inner.packets_sent.fetch_add(packets.len(), Ordering::Relaxed);
        if include_templates {
            self.inner.last_template_sent_ms.store(now_ms, Ordering::Relaxed);
        }
    }
}

impl Default for PskReporterSender {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything `flush` needs about this station -- read fresh from config
/// on every flush (not cached at construction) so a live Settings edit
/// takes effect on the next send, matching Android's own
/// read-GeneralVariables-every-flush behavior.
pub struct StationInfo {
    pub my_call: String,
    pub my_grid: String,
    pub software: String,
    pub antenna: String,
    pub rig: String,
}

/// `"FT8AF <version>"`, optionally with the connected rig name --
/// matches `PskReporterSender.buildSoftwareString`.
pub fn build_software_string(version: &str, rig_name: Option<&str>) -> String {
    match rig_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(rig) => format!("FT8AF {version} ({rig})"),
        None => format!("FT8AF {version}"),
    }
}

fn send_udp(data: &[u8]) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = socket.send_to(data, (HOST, PORT));
}

fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    nanos ^ (std::process::id() as u32)
}

fn rand_jitter_ms() -> i64 {
    let r = rand_u32() as i64 % (2 * SEND_JITTER_MS + 1);
    r - SEND_JITTER_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(call_from: &str, grid: &str, i3: u8, n3: u8) -> DecodedMessage {
        DecodedMessage { call_from: call_from.to_string(), grid: grid.to_string(), i3, n3, ..Default::default() }
    }

    #[test]
    fn reportable_locator_rejects_rr73_and_short_grids() {
        assert_eq!(reportable_locator("FN31"), Some("FN31"));
        assert_eq!(reportable_locator("FN31pr"), Some("FN31pr"));
        assert_eq!(reportable_locator("RR73"), None);
        assert_eq!(reportable_locator("rr73"), None);
        assert_eq!(reportable_locator("FN3"), None);
        assert_eq!(reportable_locator(""), None);
    }

    #[test]
    fn encode_var_string_short_and_long() {
        assert_eq!(encode_var_string("AB"), vec![2, b'A', b'B']);
        let long = "x".repeat(300);
        let encoded = encode_var_string(&long);
        assert_eq!(encoded[0], 0xFF);
        assert_eq!(((encoded[1] as usize) << 8) | encoded[2] as usize, 300);
        assert_eq!(encoded.len(), 3 + 300);
    }

    #[test]
    fn encode_receiver_template_is_4_byte_aligned_and_tagged() {
        let bytes = encode_receiver_template();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), TEMPLATE_SET_ID_OPTIONS);
        assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), RECEIVER_TEMPLATE_ID);
    }

    #[test]
    fn encode_sender_template_is_4_byte_aligned_and_tagged() {
        let bytes = encode_sender_template();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), TEMPLATE_SET_ID_DATA);
        assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), SENDER_TEMPLATE_ID);
    }

    #[test]
    fn encode_receiver_data_set_is_4_byte_aligned() {
        let bytes = encode_receiver_data_set("K1ABC", "FN31pr", "FT8AF 1.0", "40m EFHW", "QMX");
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), RECEIVER_DATA_SET_ID);
    }

    #[test]
    fn encode_sender_record_round_trips_callsign_and_frequency() {
        let spot = SpotRecord {
            sender_callsign: "W1AW".into(),
            frequency_hz: 7_074_123,
            snr: -12,
            mode: "FT8".into(),
            sender_locator: Some("FN31".into()),
            flow_start_seconds: 1_700_000_000,
        };
        let rec = encode_sender_record(&spot);
        assert_eq!(rec[0], 4); // "W1AW" length prefix
        assert_eq!(&rec[1..5], b"W1AW");
        let freq = i32::from_be_bytes([rec[5], rec[6], rec[7], rec[8]]);
        assert_eq!(freq, 7_074_123);
        assert_eq!(rec[9] as i8, -12);
    }

    #[test]
    fn build_packets_stays_under_the_datagram_limit_with_many_spots() {
        let spots: Vec<SpotRecord> = (0..200)
            .map(|i| SpotRecord {
                sender_callsign: format!("K1AB{i}"),
                frequency_hz: 7_074_000,
                snr: -10,
                mode: "FT8".into(),
                sender_locator: Some("FN31".into()),
                flow_start_seconds: 1_700_000_000,
            })
            .collect();
        let seq = AtomicU32::new(0);
        let packets = build_packets("K1ABC", "FN31pr", "FT8AF 1.0", "", "", &spots, true, &seq, 42);
        assert!(packets.len() > 1, "200 spots must span more than one datagram");
        for pkt in &packets {
            assert!(pkt.len() <= MAX_DATAGRAM, "packet of {} bytes exceeds MTU budget", pkt.len());
        }
        // Every packet still starts with the IPFIX version.
        for pkt in &packets {
            assert_eq!(u16::from_be_bytes([pkt[0], pkt[1]]), IPFIX_VERSION);
        }
    }

    #[test]
    fn build_packets_only_includes_templates_once_when_requested() {
        let spots = vec![SpotRecord {
            sender_callsign: "W1AW".into(),
            frequency_hz: 7_074_000,
            snr: -5,
            mode: "FT8".into(),
            sender_locator: None,
            flow_start_seconds: 1_700_000_000,
        }];
        let seq = AtomicU32::new(0);
        let with_templates = build_packets("K1ABC", "FN31pr", "sw", "", "", &spots, true, &seq, 1);
        let without = build_packets("K1ABC", "FN31pr", "sw", "", "", &spots, false, &seq, 1);
        assert!(with_templates[0].len() > without[0].len());
    }

    #[test]
    fn enqueue_skips_self_calls_free_text_and_hashed_callsigns() {
        let sender = PskReporterSender::new();
        let messages = vec![
            msg("K1ABC", "FN31", 1, 1),       // self call
            msg("", "FN31", 1, 1),            // empty
            msg("<...>", "FN31", 1, 1),       // hashed placeholder
            msg("W1AW", "FN31", 0, 0),        // free text (i3=0,n3=0)
            msg("W9GR", "FN31", 1, 1),        // the one real spot
        ];
        sender.enqueue(&messages, true, "K1ABC", 7_074_000, Mode::Ft8, 1_700_000_000, 1_700_000_000_000);
        assert_eq!(sender.inner.queue.lock().unwrap().len(), 1);
        assert_eq!(sender.inner.queue.lock().unwrap()[0].sender_callsign, "W9GR");
    }

    #[test]
    fn enqueue_is_a_no_op_when_disabled_or_no_callsign_configured() {
        let sender = PskReporterSender::new();
        let messages = vec![msg("W9GR", "FN31", 1, 1)];
        sender.enqueue(&messages, false, "K1ABC", 7_074_000, Mode::Ft8, 0, 0);
        sender.enqueue(&messages, true, "", 7_074_000, Mode::Ft8, 0, 0);
        assert!(sender.inner.queue.lock().unwrap().is_empty());
    }

    #[test]
    fn enqueue_dedups_the_same_call_and_band_within_the_window() {
        let sender = PskReporterSender::new();
        let messages = vec![msg("W9GR", "FN31", 1, 1)];
        sender.enqueue(&messages, true, "K1ABC", 7_074_000, Mode::Ft8, 0, 1_000);
        sender.enqueue(&messages, true, "K1ABC", 7_074_000, Mode::Ft8, 0, 1_000 + DEDUP_WINDOW_MS - 1);
        assert_eq!(sender.inner.queue.lock().unwrap().len(), 1);
        sender.enqueue(&messages, true, "K1ABC", 7_074_000, Mode::Ft8, 0, 1_000 + DEDUP_WINDOW_MS + 1);
        assert_eq!(sender.inner.queue.lock().unwrap().len(), 2);
    }

    #[test]
    fn flush_sends_nothing_when_the_queue_is_empty() {
        let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let sent_clone = sent.clone();
        let sender = PskReporterSender::with_sender(Box::new(move |d: &[u8]| {
            sent_clone.lock().unwrap().push(d.to_vec());
        }));
        sender.flush(&StationInfo {
            my_call: "K1ABC".into(),
            my_grid: "FN31pr".into(),
            software: "FT8AF 1.0".into(),
            antenna: "".into(),
            rig: "".into(),
        });
        assert!(sent.lock().unwrap().is_empty());
    }

    #[test]
    fn flush_sends_a_real_packet_for_a_queued_spot() {
        let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let sent_clone = sent.clone();
        let sender = PskReporterSender::with_sender(Box::new(move |d: &[u8]| {
            sent_clone.lock().unwrap().push(d.to_vec());
        }));
        let messages = vec![msg("W9GR", "FN31", 1, 1)];
        sender.enqueue(&messages, true, "K1ABC", 7_074_000, Mode::Ft8, 1_700_000_000, 1_700_000_000_000);
        sender.flush(&StationInfo {
            my_call: "K1ABC".into(),
            my_grid: "FN31pr".into(),
            software: "FT8AF 1.0".into(),
            antenna: "".into(),
            rig: "".into(),
        });
        assert_eq!(sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn build_software_string_includes_rig_when_present() {
        assert_eq!(build_software_string("1.0", Some("QRPLabs QMX")), "FT8AF 1.0 (QRPLabs QMX)");
        assert_eq!(build_software_string("1.0", None), "FT8AF 1.0");
        assert_eq!(build_software_string("1.0", Some("  ")), "FT8AF 1.0");
    }
}
