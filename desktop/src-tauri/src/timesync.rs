//! Built-in SNTP time sync. FT8 lives or dies by sub-second clock accuracy: if
//! the PC clock is off, every decode shows a large DT and your own transmissions
//! land outside the window. Rather than make the user install Meinberg / Dimension 4
//! / "set time automatically", we query an NTP server ourselves and feed the
//! offset into the engine's `clock_offset_ms` — correcting the app's notion of UTC
//! WITHOUT touching the system clock (so no admin rights, no background service).
//!
//! This is SNTP (RFC 4330): one request/response, no filtering or dispersion
//! tracking — plenty for keeping DT inside ±0.1 s.

use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use crate::db::Db;
use crate::engine::EngineCommand;
use crate::util;

/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_DELTA: i64 = 2_208_988_800;

/// Public NTP pools, tried in order until one answers. Anycast services first
/// (single low-latency hop, no pool DNS spread) with the pool as a fallback.
const SERVERS: &[&str] = &["time.cloudflare.com", "time.google.com", "pool.ntp.org"];

/// Re-sync cadence once we've succeeded. Hourly is far tighter than crystal drift
/// needs, but cheap insurance against a clock that wandered while suspended.
const RESYNC_INTERVAL: Duration = Duration::from_secs(3600);
/// Back-off when every server fails (offline / firewalled UDP 123).
const RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// Query one NTP server and return the local clock's offset from true UTC in
/// milliseconds. Positive means the local clock is *behind* real time, so
/// `true_utc = local + offset` — exactly what the engine adds in `now()`.
pub fn query_offset_ms(server: &str) -> anyhow::Result<i64> {
    let addr = (server, 123u16)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve NTP server {server}"))?;

    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_secs(3)))?;
    sock.set_write_timeout(Some(Duration::from_secs(3)))?;

    // 48-byte client request: LI=0, VN=3, Mode=3 (client); rest zeroed.
    let mut pkt = [0u8; 48];
    pkt[0] = 0x1B;

    let t1 = util::now_unix_ms();
    sock.send_to(&pkt, addr)?;
    let mut buf = [0u8; 48];
    let (n, src) = sock.recv_from(&mut buf)?;
    let t4 = util::now_unix_ms();
    if src.ip() != addr.ip() {
        anyhow::bail!("NTP reply from unexpected source {src}, expected {addr}");
    }
    if n < 48 {
        anyhow::bail!("short NTP reply ({n} bytes)");
    }

    // Server receive timestamp = bytes 32..40, transmit timestamp = 40..48.
    let t2 = ntp_ts_to_unix_ms(&buf[32..40]);
    let t3 = ntp_ts_to_unix_ms(&buf[40..48]);
    if t2 == 0 || t3 == 0 {
        anyhow::bail!("NTP server returned a null timestamp (kiss-of-death?)");
    }

    // RFC 4330 offset: ((t2 - t1) + (t3 - t4)) / 2.
    Ok(((t2 - t1) + (t3 - t4)) / 2)
}

/// Which servers to try, in order: the operator's configured primary first if
/// set (Android's own `ntpServer` config key -- Settings -> Time Sync), then
/// the built-in pool. Pure and unit-tested separately from the real network
/// I/O in `query_offset_ms`/`query_offset_ms_multi`.
fn server_order(primary: Option<&str>) -> Vec<&str> {
    let mut order = Vec::new();
    if let Some(p) = primary.filter(|p| !p.is_empty()) {
        order.push(p);
    }
    order.extend_from_slice(SERVERS);
    order
}

/// Try each server in `server_order`; return the first successful offset.
pub fn query_offset_ms_multi(primary: Option<&str>) -> anyhow::Result<i64> {
    let mut last_err = None;
    for srv in server_order(primary) {
        match query_offset_ms(srv) {
            Ok(off) => return Ok(off),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no NTP servers configured")))
}

/// Convert a 64-bit NTP timestamp (32.32 fixed point, big-endian) to Unix millis.
fn ntp_ts_to_unix_ms(b: &[u8]) -> i64 {
    let secs = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64;
    let frac = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as i64;
    let ms_frac = (frac * 1000) >> 32;
    (secs - NTP_UNIX_DELTA) * 1000 + ms_frac
}

/// Whether the periodic background NTP resync should run right now --
/// Android's own `disciplineClockFromNtp` (default on; `"0"` turns it off so
/// a manual correction, Settings -> Time Sync's own stepper, sticks instead
/// of being overwritten by the next hourly sync). The one-shot "Sync now"
/// button (`spawn_once`) ignores this -- an explicit request always runs.
fn ntp_discipline_enabled(db: &Db) -> bool {
    db.get_config("disciplineClockFromNtp").as_deref() != Some("0")
}

/// Spawn the background sync loop: sync on startup, then every hour (with a
/// short retry back-off while offline or while discipline is turned off).
/// Pushes results into the engine via `SetClockOffset`. Re-reads the
/// configured server and the on/off toggle from `db` fresh every cycle, so
/// changing either in Settings takes effect on the next tick without a
/// restart.
pub fn spawn_loop(tx: Sender<EngineCommand>, db: Arc<Db>) {
    std::thread::Builder::new()
        .name("ft8af-timesync".into())
        .spawn(move || loop {
            if !ntp_discipline_enabled(&db) {
                std::thread::sleep(RETRY_INTERVAL);
                continue;
            }
            match query_offset_ms_multi(db.get_config("ntpServer").as_deref()) {
                Ok(off) => {
                    if tx.send(EngineCommand::SetClockOffset(off)).is_err() {
                        return; // engine gone
                    }
                    std::thread::sleep(RESYNC_INTERVAL);
                }
                Err(_) => std::thread::sleep(RETRY_INTERVAL),
            }
        })
        .expect("spawn timesync thread");
}

/// Fire a one-shot sync (for a manual "Resync" button) without blocking the caller.
pub fn spawn_once(tx: Sender<EngineCommand>, ntp_server: Option<String>) {
    std::thread::Builder::new()
        .name("ft8af-timesync-once".into())
        .spawn(move || {
            if let Ok(off) = query_offset_ms_multi(ntp_server.as_deref()) {
                let _ = tx.send(EngineCommand::SetClockOffset(off));
            }
        })
        .expect("spawn one-shot timesync thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_order_puts_a_configured_primary_first() {
        assert_eq!(server_order(Some("ntp.example.org")), vec!["ntp.example.org", "time.cloudflare.com", "time.google.com", "pool.ntp.org"]);
    }

    #[test]
    fn server_order_falls_back_to_the_built_in_pool_when_unset() {
        assert_eq!(server_order(None), SERVERS.to_vec());
    }

    #[test]
    fn server_order_treats_an_empty_string_as_unset() {
        assert_eq!(server_order(Some("")), SERVERS.to_vec());
    }

    #[test]
    fn ntp_discipline_is_on_by_default() {
        let db = crate::db::Db::open_in_memory().unwrap();
        assert!(ntp_discipline_enabled(&db));
    }

    #[test]
    fn ntp_discipline_turns_off_only_on_an_explicit_zero() {
        let db = crate::db::Db::open_in_memory().unwrap();
        db.set_config("disciplineClockFromNtp", "0").unwrap();
        assert!(!ntp_discipline_enabled(&db));
        db.set_config("disciplineClockFromNtp", "1").unwrap();
        assert!(ntp_discipline_enabled(&db));
    }
}
