//! FCC amateur-license bulk download + Census ZCTA cross-reference, seeding
//! `callsign_qth` (schema v7, `db.rs`) with every active US station's grid
//! on first run. The table is otherwise populated live, decode-by-decode
//! (`engine.rs::publish_decodes`) and from ADIF import (`db.rs`) -- this
//! module exists only to give a brand-new install a useful Map screen
//! immediately, before the operator has heard or logged anyone.
//!
//! The FCC's amateur bulk dump (`l_amat.zip`, `EN.dat`/`HD.dat`, confirmed
//! against the real file: pipe-delimited, Call Sign at field 4 in both,
//! License Status at field 5 in HD.dat ("A" = Active), ZIP Code at field 18
//! in EN.dat) carries name/address/ZIP per callsign but **no lat/lon** --
//! amateur licenses have no filed antenna coordinates, unlike commercial/
//! repeater licenses. The fix: cross-reference the ZIP against a bundled
//! Census Bureau ZCTA Gazetteer file (ZIP -> lat/lon centroid), then convert
//! with the already-existing [`crate::util::latlon_to_maidenhead`]. ZCTA-
//! centroid accuracy is well inside a grid4 cell (~1°×2°).
//!
//! **Memory shape matters here**: `EN.dat`/`HD.dat` are ~220MB/~240MB
//! uncompressed (1.7M historical records each, ~822k currently Active) --
//! loading either into a `String` would be a very real problem on the ARM
//! SBC this app also targets, and this machine's own `/tmp` is tmpfs (RAM-
//! backed), so even a naive "download to a temp file" doesn't avoid holding
//! the data in RAM unless the temp file lives on real disk storage. Both
//! `.dat` files are therefore streamed line-by-line straight out of the zip
//! (the `zip` crate's `ZipFile` is a `Read`, wrapped in a `BufReader`), and
//! the downloaded zip itself is written to a caller-supplied `work_dir`
//! (the caller passes the app's real data directory, not `std::env::temp_dir()`)
//! and deleted once parsed. Peak RSS is bounded by the active-callsign
//! `HashSet` (~822k short strings, tens of MB) and the bundled Gazetteer map
//! (~34k entries), not by the file sizes.

use crate::db::Db;
use crate::util::latlon_to_maidenhead;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

const FCC_AMATEUR_URL: &str = "https://data.fcc.gov/download/pub/uls/complete/l_amat.zip";

const ZCTA_GAZETTEER: &str = include_str!("../resources/zcta_gazetteer.txt");

/// ZIP5 -> (lat, lon), parsed once from the bundled Census ZCTA Gazetteer
/// (`GEOID|INTPTLAT|INTPTLONG`, confirmed against the real 2026 national
/// file at census.gov/programs-surveys/geography/technical-documentation/
/// records-layout/gaz-record-layouts.html).
static ZCTA: Lazy<HashMap<String, (f64, f64)>> = Lazy::new(|| {
    let mut map = HashMap::new();
    for line in ZCTA_GAZETTEER.lines().skip(1) {
        let mut f = line.split('|');
        let (Some(geoid), Some(lat), Some(lon)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) {
            map.insert(geoid.to_string(), (lat, lon));
        }
    }
    map
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FccSeedPhase {
    #[default]
    Downloading,
    Processing,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FccSeedProgress {
    pub phase: FccSeedPhase,
    pub processed: i64,
    pub total: i64,
    /// The callsign just processed, for a "Processing Callsign: W1AW"
    /// readout -- empty during the download phase (nothing to name yet) and
    /// on the final completion tick.
    pub current_callsign: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FccSeedSummary {
    /// Rows actually inserted (a callsign already present -- e.g. already
    /// live-heard -- doesn't count, matching `INSERT OR IGNORE`).
    pub seeded: i64,
    /// Active amateur licenses found in HD.dat, for the completion message.
    pub active_licenses: i64,
}

/// Download, parse, and seed `callsign_qth` from the FCC/Census data. Writes
/// the downloaded zip to `work_dir` (the app's real data directory -- see
/// the module doc comment on why this must not be a tmpfs path) and always
/// removes it afterward, success or failure.
pub fn download_and_seed(
    db: &Db,
    work_dir: &Path,
    mut on_progress: impl FnMut(FccSeedProgress),
) -> anyhow::Result<FccSeedSummary> {
    let tmp = work_dir.join(format!(".ft8af-l_amat-{}.zip", std::process::id()));
    download_to_file(&tmp, |processed, total| {
        on_progress(FccSeedProgress {
            phase: FccSeedPhase::Downloading,
            processed,
            total,
            current_callsign: String::new(),
        });
    })?;

    let result = (|| -> anyhow::Result<FccSeedSummary> {
        let file = std::fs::File::open(&tmp)?;
        let mut zip = zip::ZipArchive::new(file)?;

        // Pass 1: which callsigns currently hold an Active license. Owned
        // HashSet so the HD.dat ZipFile borrow ends before EN.dat's begins
        // (ZipArchive hands out one `ZipFile<'_>` at a time).
        let active = {
            let hd = zip.by_name("HD.dat")?;
            active_callsigns_from_hd(BufReader::new(hd))
        };
        let active_licenses = active.len() as i64;

        // Pass 2: EN.dat -> (callsign, grid) for active callsigns whose ZIP
        // resolves in the bundled Gazetteer, batch-inserted under one
        // transaction (db.rs::seed_callsign_qth_batch).
        let en = zip.by_name("EN.dat")?;
        let pairs = en_dat_pairs(BufReader::new(en), &active);
        let seeded = db.seed_callsign_qth_batch(pairs, active_licenses, |processed, total, call| {
            on_progress(FccSeedProgress {
                phase: FccSeedPhase::Processing,
                processed,
                total,
                current_callsign: call.to_string(),
            });
        });

        Ok(FccSeedSummary { seeded, active_licenses })
    })();

    let _ = std::fs::remove_file(&tmp);
    result
}

/// Stream the FCC zip to `dest`, reporting byte progress roughly every 1MB
/// (a tick per 64KB read would be needlessly chatty for a ~200MB file).
fn download_to_file(dest: &Path, mut on_progress: impl FnMut(i64, i64)) -> anyhow::Result<()> {
    let resp = ureq::get(FCC_AMATEUR_URL).timeout(Duration::from_secs(300)).call()?;
    let total: i64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0i64;
    let mut since_last_tick = 0i64;
    const TICK_BYTES: i64 = 1024 * 1024;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as i64;
        since_last_tick += n as i64;
        if since_last_tick >= TICK_BYTES {
            since_last_tick = 0;
            on_progress(downloaded, total);
        }
    }
    on_progress(downloaded, if total > 0 { total } else { downloaded });
    Ok(())
}

/// Callsigns (uppercased) with License Status "A" (Active) in HD.dat.
/// Fields, confirmed against the real file: 0=record type, 1=unique system
/// id, 2=ULS file number, 3=EBF number, 4=call sign, 5=license status.
fn active_callsigns_from_hd(r: impl BufRead) -> HashSet<String> {
    let mut set = HashSet::new();
    for line in r.lines().map_while(Result::ok) {
        let f: Vec<&str> = line.splitn(7, '|').collect();
        let (Some(call), Some(status)) = (f.get(4), f.get(5)) else { continue };
        if *status == "A" && !call.is_empty() {
            set.insert(call.to_ascii_uppercase());
        }
    }
    set
}

/// Lazily yields (callsign, grid4) for each EN.dat record whose callsign is
/// in `active` and whose ZIP resolves in the bundled Gazetteer. Fields,
/// confirmed against the real file: 4=call sign, 18=ZIP code (5 or 9
/// digits, ZIP+4 truncated to the first 5; the Gazetteer only has ZIP5).
fn en_dat_pairs<'a>(
    r: impl BufRead + 'a,
    active: &'a HashSet<String>,
) -> impl Iterator<Item = (String, String)> + 'a {
    r.lines().map_while(Result::ok).filter_map(move |line| {
        let f: Vec<&str> = line.split('|').collect();
        let call = f.get(4)?.trim();
        if call.is_empty() {
            return None;
        }
        let call_upper = call.to_ascii_uppercase();
        if !active.contains(&call_upper) {
            return None;
        }
        let zip = f.get(18)?.trim();
        let zip5 = zip.get(..5)?;
        let (lat, lon) = ZCTA.get(zip5)?;
        Some((call_upper, latlon_to_maidenhead(*lat, *lon, 4)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zcta_gazetteer_parses_a_known_entry_count_order_of_magnitude() {
        // The real 2026 national ZCTA file has 33,791 data rows (33,792
        // lines incl. header) -- guard against a truncated/corrupt resource.
        assert!(ZCTA.len() > 30_000, "ZCTA entries: {}", ZCTA.len());
    }

    #[test]
    fn zcta_gazetteer_resolves_known_zips() {
        // Spot-checked against the real downloaded Gazetteer file.
        let (lat, lon) = ZCTA.get("00601").expect("00601 should be in the Gazetteer");
        assert!((17.5..19.0).contains(lat), "lat {lat}");
        assert!((-67.5..-66.0).contains(lon), "lon {lon}");
    }

    #[test]
    fn active_callsigns_from_hd_filters_on_status_a_only() {
        let hd = "\
HD|1|9400005006||AA0AI|A|HA|10/11/2023|11/23/2033\n\
HD|2|9400005359||AA0AM|C|HA|09/24/2010|09/24/2020\n\
HD|3|9607290732||AA0AQ|E|HA|09/01/2010|09/01/2020\n\
HD|4|0008956180||AA0AT|A|HA|01/27/2020|04/17/2030\n";
        let set = active_callsigns_from_hd(hd.as_bytes());
        assert_eq!(set, HashSet::from(["AA0AI".to_string(), "AA0AT".to_string()]));
    }

    #[test]
    fn active_callsigns_from_hd_tolerates_short_or_malformed_lines() {
        let hd = "garbage\nHD|1||\nHD|2|x||AA0AT|A\n";
        let set = active_callsigns_from_hd(hd.as_bytes());
        assert_eq!(set, HashSet::from(["AA0AT".to_string()]));
    }

    #[test]
    fn en_dat_pairs_only_yields_active_callsigns_with_a_resolvable_zip() {
        // AA0AI is active with a ZCTA-covered ZIP; AA0AM is active but its
        // ZIP isn't in this tiny test Gazetteer slice (simulates a real
        // unresolvable case, e.g. an APO/FPO address); W1AW isn't active at
        // all. Real EN.dat rows, field positions confirmed against the real
        // file (4=call, 18=zip).
        let en = "\
EN|1|||AA0AI|L|L1|STEENBUCK JR, KENNETH|KENNETH||STEENBUCK|JR||||790 ROUTE 9W APT 229|GLENMONT|NY|00601|||000|1|I\n\
EN|2|||AA0AM|L|L2|SOMEONE|SOMEONE|||||||1 MAIN ST|NOWHERE|ZZ|99999|||000|2|I\n\
EN|3|||W1AW|L|L3|NOT ACTIVE|NOT|||ACTIVE|||1 MAIN ST|HARTFORD|CT|00601|||000|3|I\n";
        let active = HashSet::from(["AA0AI".to_string(), "AA0AM".to_string()]);
        let pairs: Vec<_> = en_dat_pairs(en.as_bytes(), &active).collect();
        assert_eq!(pairs.len(), 1, "{pairs:?}");
        assert_eq!(pairs[0].0, "AA0AI");
        assert_eq!(pairs[0].1.len(), 4);
    }

    #[test]
    fn en_dat_pairs_truncates_zip_plus_4_to_zip5() {
        // Real EN.dat ZIP+4 is 9 concatenated digits, no hyphen (confirmed
        // against the real file, e.g. "070441440").
        let en = "EN|1|||AA0AI|L|L1|N|N|||||||ADDR|CITY|NY|006011234|||000|1|I\n";
        let active = HashSet::from(["AA0AI".to_string()]);
        let pairs: Vec<_> = en_dat_pairs(en.as_bytes(), &active).collect();
        assert_eq!(pairs.len(), 1);
    }
}
