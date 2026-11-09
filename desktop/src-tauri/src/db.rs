//! SQLite persistence (rusqlite, bundled). Ports the essential `qso_log` +
//! `config` schema from `DatabaseOpr.java` and the ADIF export format from
//! `ShareLogs.downQSLTableToFile`. Uses a `user_version` PRAGMA migration rather
//! than the Android app's v18 upgrade ladder.

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 7;

/// One logged QSO. Field names follow ADIF.
///
/// QSL tracking mirrors the Android app's granular per-service fields (see
/// `fork-changes.md`'s "QSL column" section), with one deliberate
/// simplification: Android's legacy `qsl_rcvd` is contextually either "paper
/// received" or a stand-in for LoTW depending on whether `isLotW_QSL` is also
/// set; here `confirmed` (the field that already existed before this file had
/// per-service tracking, and is what ADIF's own `QSL_RCVD` tag means) is
/// always "paper card received", full stop — `lotw_rcvd` is its own field, not
/// overloaded onto this one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QsoRecord {
    #[serde(default)]
    pub id: Option<i64>,
    pub call: String,
    #[serde(default)]
    pub gridsquare: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub rst_sent: String,
    #[serde(default)]
    pub rst_rcvd: String,
    pub qso_date: String,
    pub time_on: String,
    #[serde(default)]
    pub qso_date_off: String,
    #[serde(default)]
    pub time_off: String,
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub freq: String,
    #[serde(default)]
    pub station_callsign: String,
    #[serde(default)]
    pub my_gridsquare: String,
    #[serde(default)]
    pub comment: String,
    /// Paper QSL card confirmed received (ADIF `QSL_RCVD`). Drives the
    /// Logbook filter.
    #[serde(default)]
    pub confirmed: bool,
    /// Paper QSL card sent (ADIF `QSL_SENT`).
    #[serde(default)]
    pub qsl_sent: bool,
    /// LoTW confirmation sent (ADIF `LOTW_QSL_SENT`).
    #[serde(default)]
    pub lotw_sent: bool,
    /// LoTW confirmation received (ADIF `LOTW_QSL_RCVD`).
    #[serde(default)]
    pub lotw_rcvd: bool,
    /// eQSL sent (ADIF `EQSL_QSL_SENT`).
    #[serde(default)]
    pub eqsl_sent: bool,
    /// eQSL received (ADIF `EQSL_QSL_RCVD`).
    #[serde(default)]
    pub eqsl_rcvd: bool,
    /// Confirmed on QRZ.com (a QRZ.com logbook export's `APP_QRZLOG_STATUS=C`;
    /// there's no "sent" side — the upload itself is `synced_qrz`).
    #[serde(default)]
    pub qrz_qsl: bool,
    /// Logged US state or, for other countries, that country's subdivision
    /// (ADIF `STATE`). Not derived from the grid — only ever what was
    /// explicitly logged or imported; see `fork-changes.md`'s note on the
    /// grid→state lookup this app doesn't have yet.
    #[serde(default)]
    pub state: String,
    /// Logged country (ADIF `COUNTRY`).
    #[serde(default)]
    pub country: String,
    /// Already uploaded to Cloudlog/Wavelog/Nextlog. Read-only here — set by
    /// `Db::mark_synced`, never by `insert_qso`/import.
    #[serde(default)]
    pub synced_cloudlog: bool,
    /// Already uploaded to QRZ.com's logbook. Read-only here — see `synced_cloudlog`.
    #[serde(default)]
    pub synced_qrz: bool,
}

fn default_mode() -> String {
    "FT8".to_string()
}

/// Column list for every `SELECT ... FROM qso_log` that feeds [`row_to_qso`].
/// Keep this, `row_to_qso`'s positional `r.get(N)` calls, and `insert_qso_locked`'s
/// `INSERT` column list in sync — they all assume this exact order.
const QSO_SELECT_COLUMNS: &str = "id, call, gridsquare, mode, rst_sent, rst_rcvd, qso_date, \
    time_on, qso_date_off, time_off, band, freq, station_callsign, my_gridsquare, comment, \
    qsl_rcvd, qsl_sent, lotw_sent, lotw_rcvd, eqsl_sent, eqsl_rcvd, qrz_qsl, state, country, \
    synced_cloudlog, synced_qrz";

/// Result of an [`Db::import_adif`] call, for the UI to report back to the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    /// New QSOs inserted.
    pub imported: i64,
    /// Records that matched an existing QSO (not overwritten; see `import_adif`).
    pub duplicates: i64,
    /// Records with no usable `CALL`, or that failed to insert.
    pub skipped: i64,
}

/// Live progress for an in-flight [`Db::import_adif_with_progress`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProgress {
    pub processed: i64,
    pub total: i64,
}

/// Logbook confirmation filter, mirroring the Android `FilterDialog`
/// (All / QSL'd only / Not-QSL'd only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmedFilter {
    All,
    Confirmed,
    Unconfirmed,
}

impl Default for ConfirmedFilter {
    fn default() -> Self {
        ConfirmedFilter::All
    }
}

/// Pure predicate: does a QSO match the live search box + confirmation filter?
///
/// `callsign_query` is matched case-insensitively as a substring of the call
/// (empty query matches everything), mirroring `queryByCallsign` on Android.
pub fn qso_matches(rec: &QsoRecord, callsign_query: &str, filter: ConfirmedFilter) -> bool {
    let call_ok = {
        let q = callsign_query.trim();
        q.is_empty() || rec.call.to_uppercase().contains(&q.to_uppercase())
    };
    let confirm_ok = match filter {
        ConfirmedFilter::All => true,
        ConfirmedFilter::Confirmed => rec.confirmed,
        ConfirmedFilter::Unconfirmed => !rec.confirmed,
    };
    call_ok && confirm_ok
}

/// Pure predicate: is an ADIF `YYYYMMDD` date within an optional inclusive
/// `[start, end]` range? Empty/None bounds are open; with no bounds everything
/// passes. Under a bound, a value that isn't a real 8-digit `YYYYMMDD` date is
/// excluded — lexicographic comparison is only meaningful for that fixed width,
/// so e.g. `"BAD"` must not sneak past a start-only bound just because it sorts
/// after the digits.
pub fn date_in_range(qso_date: &str, start: Option<&str>, end: Option<&str>) -> bool {
    let start = start.map(str::trim).filter(|s| !s.is_empty());
    let end = end.map(str::trim).filter(|s| !s.is_empty());
    if start.is_none() && end.is_none() {
        return true;
    }
    let d = qso_date.trim();
    if !(d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    if let Some(s) = start {
        if d < s {
            return false;
        }
    }
    if let Some(e) = end {
        if d > e {
            return false;
        }
    }
    true
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Db> {
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Db> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> anyhow::Result<Db> {
        let db = Db { conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS config (
                    key   TEXT PRIMARY KEY,
                    value TEXT
                );
                CREATE TABLE IF NOT EXISTS qso_log (
                    id               INTEGER PRIMARY KEY AUTOINCREMENT,
                    call             TEXT NOT NULL,
                    gridsquare       TEXT,
                    mode             TEXT,
                    rst_sent         TEXT,
                    rst_rcvd         TEXT,
                    qso_date         TEXT,
                    time_on          TEXT,
                    qso_date_off     TEXT,
                    time_off         TEXT,
                    band             TEXT,
                    freq             TEXT,
                    station_callsign TEXT,
                    my_gridsquare    TEXT,
                    comment          TEXT,
                    created_at       INTEGER
                );
                CREATE INDEX IF NOT EXISTS qso_log_call_idx ON qso_log(call);
                CREATE UNIQUE INDEX IF NOT EXISTS qso_log_dedup_idx
                    ON qso_log(call, qso_date, time_on, mode);
                "#,
            )?;
        }
        if version < 2 {
            // v2: QSL confirmation + per-service online-sync flags. These are
            // added via ALTER so an existing v1 log upgrades in place; on a
            // fresh DB the base table was just created above without them.
            // `IF NOT EXISTS` isn't supported by ADD COLUMN, so ignore the
            // "duplicate column" error to keep the migration idempotent.
            for col in [
                "ALTER TABLE qso_log ADD COLUMN qsl_rcvd INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN synced_cloudlog INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN synced_qrz INTEGER NOT NULL DEFAULT 0",
            ] {
                match conn.execute(col, []) {
                    Ok(_) => {}
                    Err(rusqlite::Error::SqliteFailure(_, Some(ref m)))
                        if m.contains("duplicate column name") => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if version < 3 {
            // v3: an expression index matching `find_duplicate_locked`'s WHERE
            // clause exactly, so ADIF import's duplicate lookup can use an
            // index instead of a full table scan per row. `qso_log_call_idx`
            // (v1) is on the raw `call` column and SQLite can't use it for a
            // query that compares `UPPER(call)`.
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS qso_log_import_dedup_idx
                     ON qso_log(UPPER(call), qso_date, UPPER(mode),
                                substr(time_on || '000000', 1, 4));",
            )?;
        }
        if version < 4 {
            // v4: per-service QSL tracking and logged STATE/COUNTRY, matching
            // the Android Logbook table (`fork-changes.md`'s "QSL column" and
            // "STATE/COUNTRY" sections). `qsl_rcvd` (v2) already covers paper
            // received; this adds its missing partner `qsl_sent` plus the
            // three other services and the two logged-location fields.
            for col in [
                "ALTER TABLE qso_log ADD COLUMN qsl_sent INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN lotw_sent INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN lotw_rcvd INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN eqsl_sent INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN eqsl_rcvd INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN qrz_qsl INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE qso_log ADD COLUMN state TEXT NOT NULL DEFAULT ''",
                "ALTER TABLE qso_log ADD COLUMN country TEXT NOT NULL DEFAULT ''",
            ] {
                match conn.execute(col, []) {
                    Ok(_) => {}
                    Err(rusqlite::Error::SqliteFailure(_, Some(ref m)))
                        if m.contains("duplicate column name") => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if version < 5 {
            // v5: MODE dropped from the import dedup match (see
            // `merge_matched_qso`'s doc comment for why) -- replace the v3
            // index, which included `UPPER(mode)`, with one matching the new
            // (mode-free) WHERE clause exactly.
            conn.execute_batch(
                "DROP INDEX IF EXISTS qso_log_import_dedup_idx;
                 CREATE INDEX IF NOT EXISTS qso_log_import_dedup_idx
                     ON qso_log(UPPER(call), qso_date, substr(time_on || '000000', 1, 4));",
            )?;
        }
        if version < 6 {
            // v6: index matching find_lotw_twin_locked's WHERE clause
            // (UPPER(call), qso_date, UPPER(band), UPPER(mode)) -- without it,
            // every record that misses the exact-minute match falls through
            // to a near-full-table scan for its twin. Confirmed on the real
            // ~55k-record QRZ.com file: importing it against an already-
            // populated ~45k-row log (so most records hit the twin-match
            // fallback) took over 90 seconds without this index.
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS qso_log_lotw_twin_idx
                     ON qso_log(UPPER(call), qso_date, UPPER(band), UPPER(mode));",
            )?;
        }
        if version < 7 {
            // v7: callsign -> Maidenhead-grid cache for the (not yet built)
            // Map screen, matching Android's `CallsignQTH` table exactly --
            // just callsign/grid/updated_at, no extra columns. Populated from
            // four sources (see `upsert_callsign_qth_live`/
            // `upsert_callsign_qth_if_absent_locked`'s doc comments): live
            // decodes, ADIF import, and a first-run FCC/Census bulk seed.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS callsign_qth (
                    callsign   TEXT PRIMARY KEY,
                    grid       TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );",
            )?;
        }
        conn.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION}"), [])?;
        Ok(())
    }

    // --- config key/value ---------------------------------------------------

    pub fn set_config(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO config(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_config(&self, key: &str) -> Option<String> {
        self.conn
            .lock()
            .query_row("SELECT value FROM config WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .ok()
    }

    pub fn all_config(&self) -> Vec<(String, String)> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare("SELECT key, value FROM config") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        rows
    }

    // --- QSO log -------------------------------------------------------------

    /// Insert (or update on the dedup key) a QSO. Returns its row id.
    pub fn insert_qso(&self, r: &QsoRecord) -> anyhow::Result<i64> {
        Self::insert_qso_locked(&self.conn.lock(), r)
    }

    /// Same as [`Db::insert_qso`], taking an already-locked connection so a
    /// caller holding the lock for a multi-row transaction (see
    /// `import_adif_with_progress`) doesn't re-lock the non-reentrant mutex.
    fn insert_qso_locked(conn: &Connection, r: &QsoRecord) -> anyhow::Result<i64> {
        conn.execute(
            "INSERT INTO qso_log
                (call, gridsquare, mode, rst_sent, rst_rcvd, qso_date, time_on,
                 qso_date_off, time_off, band, freq, station_callsign,
                 my_gridsquare, comment, qsl_rcvd, qsl_sent, lotw_sent, lotw_rcvd,
                 eqsl_sent, eqsl_rcvd, qrz_qsl, state, country, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
                     ?19,?20,?21,?22,?23,?24)
             ON CONFLICT(call, qso_date, time_on, mode) DO UPDATE SET
                gridsquare=excluded.gridsquare, rst_sent=excluded.rst_sent,
                rst_rcvd=excluded.rst_rcvd, qso_date_off=excluded.qso_date_off,
                time_off=excluded.time_off, band=excluded.band, freq=excluded.freq,
                station_callsign=excluded.station_callsign,
                my_gridsquare=excluded.my_gridsquare, comment=excluded.comment,
                qsl_rcvd=excluded.qsl_rcvd, qsl_sent=excluded.qsl_sent,
                lotw_sent=excluded.lotw_sent, lotw_rcvd=excluded.lotw_rcvd,
                eqsl_sent=excluded.eqsl_sent, eqsl_rcvd=excluded.eqsl_rcvd,
                qrz_qsl=excluded.qrz_qsl, state=excluded.state, country=excluded.country",
            params![
                r.call, r.gridsquare, r.mode, r.rst_sent, r.rst_rcvd, r.qso_date,
                r.time_on, r.qso_date_off, r.time_off, r.band, r.freq,
                r.station_callsign, r.my_gridsquare, r.comment, r.confirmed as i64,
                r.qsl_sent as i64, r.lotw_sent as i64, r.lotw_rcvd as i64,
                r.eqsl_sent as i64, r.eqsl_rcvd as i64, r.qrz_qsl as i64,
                r.state, r.country,
                crate::util::now_unix_ms(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_qsos(&self, limit: i64, offset: i64) -> Vec<QsoRecord> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(&format!(
            "SELECT {QSO_SELECT_COLUMNS} FROM qso_log ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![limit, offset], row_to_qso)
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Server-side search/filter mirroring the Android Logbook tab: substring
    /// match on callsign plus the QSL confirmation filter, most-recent-first.
    ///
    /// Filters and paginates at the DB layer (real `LIMIT`/`OFFSET`) instead of
    /// loading the whole log and filtering in memory, so it stays fast as the log
    /// grows. The callsign match is a case-insensitive (ASCII) `LIKE` substring
    /// with the query's `LIKE` wildcards escaped so it behaves as a literal
    /// substring — i.e. the same result set as the pure [`qso_matches`] predicate
    /// (guarded by `search_qsos_matches_predicate` in the tests).
    pub fn search_qsos(
        &self,
        callsign_query: &str,
        filter: ConfirmedFilter,
        limit: i64,
        offset: i64,
    ) -> Vec<QsoRecord> {
        let q = callsign_query.trim();
        let like = format!(
            "%{}%",
            q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
        );
        let confirm_clause = match filter {
            ConfirmedFilter::All => "",
            ConfirmedFilter::Confirmed => " AND qsl_rcvd = 1",
            ConfirmedFilter::Unconfirmed => " AND qsl_rcvd = 0",
        };
        let sql = format!(
            "SELECT {QSO_SELECT_COLUMNS}
             FROM qso_log
             WHERE call LIKE ?1 ESCAPE '\\'{confirm_clause}
             ORDER BY id DESC LIMIT ?2 OFFSET ?3"
        );
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![like, limit.max(0), offset.max(0)], row_to_qso)
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Distinct calls already logged on `band`/`mode` (both matched
    /// case-insensitively) -- mirrors Android's `GetAllQSLCallsign` "worked on
    /// this band and mode" query, used to strike through already-worked
    /// stations in the Decode list. "Worked" here means any past QSO was
    /// logged, not that it was QSL-confirmed.
    pub fn worked_calls(&self, band: &str, mode: &str) -> Vec<String> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT UPPER(call) FROM qso_log WHERE UPPER(band) = UPPER(?1) AND UPPER(mode) = UPPER(?2)",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![band, mode], |row| row.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Distinct calls already logged on `mode`, any band -- mirrors Android's
    /// `QSL_Callsign_list_other_band` (`checkQSLCallsign_OtherBand`), used by
    /// the Decode list's New-Band status (worked on some other band, but not
    /// this one -- the caller combines this with [Self::worked_calls] on the
    /// current band to get "other band only").
    pub fn worked_calls_any_band(&self, mode: &str) -> Vec<String> {
        let conn = self.conn.lock();
        let mut stmt = match conn
            .prepare("SELECT DISTINCT UPPER(call) FROM qso_log WHERE UPPER(mode) = UPPER(?1)")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![mode], |row| row.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Distinct 4+ character grid squares already logged, any band/mode --
    /// mirrors Android's "Worked Grids" check (`GeneralVariables.checkQSLGrid`,
    /// which is not band/mode-scoped, unlike [Self::worked_calls]) for the
    /// Decode list's Hunt "new grid" priority and New-Grid filter. A 2-char
    /// grid field (locator precision too coarse to mean "worked this square")
    /// is excluded, matching the 4-char floor Android's own grid-priority
    /// logic uses.
    pub fn worked_grids(&self) -> Vec<String> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT UPPER(gridsquare) FROM qso_log WHERE LENGTH(gridsquare) >= 4",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| row.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Distinct CQ WPX prefixes (see `crate::wpx_prefix`) already logged, any
    /// band/mode -- mirrors Android's `QSL_Prefix_list` for the Decode list's
    /// New-Prefix filter. Derived from the worked calls rather than stored as
    /// its own column, same as grid/call aggregation above.
    pub fn worked_prefixes(&self) -> Vec<String> {
        let mut prefixes: Vec<String> = self
            .distinct_calls()
            .iter()
            .filter_map(|c| crate::wpx_prefix::of(c))
            .collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        prefixes
    }

    /// Distinct DXCC entity prefixes (see `crate::dxcc`) already logged, any
    /// band/mode -- mirrors Android's `GeneralVariables.dxccMap`, populated
    /// by `DatabaseOpr.getQslDxccToMap` from every distinct logged call, for
    /// the Decode list's New-DXCC filter.
    pub fn worked_dxcc_prefixes(&self) -> Vec<String> {
        let mut prefixes: Vec<String> = self
            .distinct_calls()
            .iter()
            .filter_map(|c| crate::dxcc::lookup(c))
            .map(|m| m.dxcc_prefix)
            .collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        prefixes
    }

    /// Distinct CQ zones already logged, any band/mode -- mirrors Android's
    /// `GeneralVariables.cqMap`, for the Decode list's New-Zone filter.
    pub fn worked_cq_zones(&self) -> Vec<i64> {
        let mut zones: Vec<i64> = self
            .distinct_calls()
            .iter()
            .filter_map(|c| crate::dxcc::lookup(c))
            .map(|m| m.cq_zone)
            .collect();
        zones.sort_unstable();
        zones.dedup();
        zones
    }

    /// Distinct US states worked on `band`/`mode` (both matched
    /// case-insensitively), each resolved the same way a live decode's state
    /// is (`crate::us_states::worked_state_for`: the logged ADIF `STATE`
    /// when it's trustworthy, else the grid) -- mirrors Android's WAS
    /// tracking (`DatabaseOpr.getQslDxccToMap`'s state pass,
    /// `workedStateBandModes`), which is per band *and* mode unlike
    /// DXCC/Zone above. Backs the Decode list's New-State filter.
    pub fn worked_states(&self, band: &str, mode: &str) -> Vec<String> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(
            "SELECT call, gridsquare, state FROM qso_log \
             WHERE UPPER(band) = UPPER(?1) AND UPPER(mode) = UPPER(?2)",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![band, mode], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        drop(stmt);
        drop(conn);

        let mut states: Vec<String> = rows
            .iter()
            .filter_map(|(call, grid, state)| {
                let logged = if state.trim().is_empty() { None } else { Some(state.as_str()) };
                let grid = if grid.is_empty() { None } else { Some(grid.as_str()) };
                let call = if call.is_empty() { None } else { Some(call.as_str()) };
                crate::us_states::worked_state_for(logged, grid, call)
            })
            .collect();
        states.sort_unstable();
        states.dedup();
        states
    }

    /// Distinct calls already logged, any band/mode -- shared helper behind
    /// [Self::worked_dxcc_prefixes]/[Self::worked_cq_zones], which both
    /// derive their sets from the same any-band call list via
    /// `crate::dxcc::lookup`.
    fn distinct_calls(&self) -> Vec<String> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare("SELECT DISTINCT UPPER(call) FROM qso_log") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| row.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// Flag a QSO as synced to an online service so re-runs are idempotent.
    /// `service` is `"cloudlog"` or `"qrz"`; unknown services are a no-op.
    pub fn mark_synced(&self, id: i64, service: &str) -> anyhow::Result<()> {
        let column = match service {
            "cloudlog" => "synced_cloudlog",
            "qrz" => "synced_qrz",
            _ => return Ok(()),
        };
        self.conn.lock().execute(
            &format!("UPDATE qso_log SET {column} = 1 WHERE id = ?1"),
            params![id],
        )?;
        Ok(())
    }

    /// Whether a QSO has already been synced to the given service.
    pub fn is_synced(&self, id: i64, service: &str) -> bool {
        let column = match service {
            "cloudlog" => "synced_cloudlog",
            "qrz" => "synced_qrz",
            _ => return false,
        };
        self.conn
            .lock()
            .query_row(
                &format!("SELECT {column} FROM qso_log WHERE id = ?1"),
                params![id],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    pub fn count(&self) -> i64 {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM qso_log", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn delete_qso(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM qso_log WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Export the whole log as an ADIF string, matching ShareLogs' field set/order.
    pub fn export_adif(&self) -> String {
        self.export_adif_range(None, None)
    }

    /// Import ADIF text into the log. See [`Db::import_adif_with_progress`]
    /// for the matching rule and performance notes; this is that with no
    /// progress callback.
    pub fn import_adif(&self, text: &str) -> ImportSummary {
        self.import_adif_with_progress(text, |_, _| {})
    }

    /// Import ADIF text into the log, mirroring the Android app's import rule
    /// (`DatabaseOpr.checkIsQSL`): a QSO already in the log is matched on call
    /// + date, case-insensitively, with `TIME_ON` compared only to the minute
    /// (`1357` matches `135745`) — **band and mode are deliberately not part
    /// of the match**. Band: the same QSO logged by two different programs
    /// can disagree on it (the app's own `QSO: advance order` path showed
    /// this). Mode: confirmed on a real 54k-QSO log — a program can
    /// round-trip FT4/FT8 contacts through its own ADIF import/export as the
    /// generic `MFSK`/`PSK` family mode instead of the specific submode (no
    /// `SUBMODE` tag to recover it from), while a re-export from the
    /// original source (e.g. QRZ.com) still has the specific one; comparing
    /// mode made ~7,100 of ~54,500 QSOs fail to match their own duplicate,
    /// nearly doubling the log on import. A match is never overwritten by
    /// the import: see [`merge_matched_qso`] for exactly what an import may
    /// still change on an existing QSO (new QSL/upload flags turned on,
    /// never cleared; STATE/COUNTRY filled only when blank; see its doc
    /// comment for the LoTW-twin fill exception). Anything that doesn't
    /// match an existing QSO, directly or via [`Self::find_lotw_twin_locked`],
    /// is inserted as new. Records with no usable `CALL` are skipped.
    ///
    /// The whole import runs inside **one SQLite transaction**, held for the
    /// duration (hence working through `_locked` helpers rather than the
    /// public per-call methods, which would each re-lock the connection).
    /// A real CQRLOG/QRZ.com export can be tens of thousands of QSOs; one
    /// `INSERT` per row with no surrounding transaction means one disk fsync
    /// per row under SQLite's default rollback-journal mode — confirmed on
    /// real hardware to turn a ~50k-row import into many minutes with no
    /// visible progress, which looked identical to a hang. Batched into one
    /// transaction, a fresh ~50k-row import finishes in a fraction of a
    /// second. A later import of a similarly large file *against* an
    /// already-populated log is slower — every record that misses the exact
    /// match falls through to [`Self::find_lotw_twin_locked`]'s fallback
    /// query — confirmed on the user's real files at ~97s for a ~54,500-
    /// record QRZ.com file against an already-imported ~45,600-row log.
    /// Still far better than the pre-batching multi-minute silent hang, and
    /// `on_progress` keeps the UI visibly moving throughout rather than
    /// looking frozen; a further speedup (caching prepared statements across
    /// the loop instead of re-preparing per record) is a possible follow-up,
    /// not done here.
    ///
    /// `on_progress(processed, total)` is called every 200 records and once
    /// more at the end (never, on an empty import), so a caller can show a
    /// live counter on a large file instead of a silent wait.
    pub fn import_adif_with_progress(
        &self,
        text: &str,
        mut on_progress: impl FnMut(i64, i64),
    ) -> ImportSummary {
        let records = crate::adif::parse_adif(text);
        let total = records.len() as i64;
        let mut summary = ImportSummary::default();

        let conn = self.conn.lock();
        let _ = conn.execute_batch("BEGIN");
        for (idx, rec) in records.iter().enumerate() {
            match crate::adif::adif_record_to_qso(&rec.fields) {
                None => summary.skipped += 1,
                Some(qso) => {
                    // Backfill the callsign/grid cache for the Map screen --
                    // never overwrites a live-heard grid (see
                    // `upsert_callsign_qth_if_absent_locked`'s doc comment),
                    // and rides this same already-open transaction.
                    if !qso.gridsquare.is_empty() {
                        Self::upsert_callsign_qth_if_absent_locked(&conn, &qso.call, &qso.gridsquare);
                    }
                    let existing = Self::find_duplicate_locked(
                        &conn, &qso.call, &qso.qso_date, &qso.time_on,
                    )
                    .or_else(|| Self::find_lotw_twin_locked(&conn, &qso));
                    match existing {
                        Some(existing) => {
                            if let Some(merged) = merge_matched_qso(&existing, &qso) {
                                let _ =
                                    Self::update_flags_locked(&conn, existing.id.unwrap(), &merged);
                            }
                            summary.duplicates += 1;
                        }
                        None => match Self::insert_qso_locked(&conn, &qso) {
                            Ok(_) => summary.imported += 1,
                            Err(_) => summary.skipped += 1,
                        },
                    }
                }
            }
            let processed = (idx + 1) as i64;
            if processed % 200 == 0 || processed == total {
                on_progress(processed, total);
            }
        }
        let _ = conn.execute_batch("COMMIT");
        summary
    }

    /// Find an existing QSO matching the import dedup rule described on
    /// [`Db::import_adif_with_progress`]. Relies on `qso_log_import_dedup_idx`
    /// (schema v3) to stay fast at large log sizes — an unindexed `UPPER(call)`
    /// comparison is a full table scan per call, which multiplies out badly
    /// against a many-thousand-row import of a many-thousand-row existing log.
    fn find_duplicate_locked(
        conn: &Connection,
        call: &str,
        qso_date: &str,
        time_on: &str,
    ) -> Option<QsoRecord> {
        conn.query_row(
            &format!(
                "SELECT {QSO_SELECT_COLUMNS} FROM qso_log
                 WHERE UPPER(call) = UPPER(?1) AND qso_date = ?2
                   AND substr(time_on || '000000', 1, 4) = substr(?3 || '000000', 1, 4)
                 LIMIT 1",
            ),
            params![call, qso_date, time_on],
            row_to_qso,
        )
        .ok()
    }

    /// Minutes a LoTW-style record's `TIME_ON` may differ from its full
    /// twin's, before [`Self::find_lotw_twin_locked`] gives up. Matches
    /// Android's `DatabaseOpr.LOTW_TWIN_MINUTES`.
    const LOTW_TWIN_MINUTES: i32 = 15;

    /// Find a QSO one QSO's worth of data away: the same call (ci) + date +
    /// band (ci) + mode (ci), within [`Self::LOTW_TWIN_MINUTES`] of
    /// `imported`'s `TIME_ON` (nearest wins), and the *opposite*
    /// [`is_lotw_style_record`]-ness from `imported`. Only called when
    /// [`Self::find_duplicate_locked`] already found nothing — see
    /// [`merge_matched_qso`]'s doc comment for why this exists and what a
    /// match does. Ports Android's `DatabaseOpr.matchLotwTwin` exactly,
    /// including its first-seen-wins tie-break (`ORDER BY id`, strict `<`).
    fn find_lotw_twin_locked(conn: &Connection, imported: &QsoRecord) -> Option<QsoRecord> {
        let minute = minute_of_day(&imported.time_on)?;
        if imported.qso_date.trim().is_empty() {
            return None;
        }
        let incoming_lotw = is_lotw_style_record(imported);
        let side = if incoming_lotw { "NOT" } else { "" };
        let sql = format!(
            "SELECT {QSO_SELECT_COLUMNS} FROM qso_log
             WHERE UPPER(call) = UPPER(?1) AND qso_date = ?2
               AND UPPER(band) = UPPER(?3) AND UPPER(mode) = UPPER(?4)
               AND {side} {LOTW_ROW_SQL}
             ORDER BY id ASC",
        );
        let mut stmt = conn.prepare(&sql).ok()?;
        let rows = stmt
            .query_map(
                params![imported.call, imported.qso_date, imported.band, imported.mode],
                row_to_qso,
            )
            .ok()?;
        let mut best: Option<(i32, QsoRecord)> = None;
        for row in rows.flatten() {
            let Some(m) = minute_of_day(&row.time_on) else { continue };
            let gap = (m - minute).abs();
            if gap <= Self::LOTW_TWIN_MINUTES
                && best.as_ref().is_none_or(|(best_gap, _)| gap < *best_gap)
            {
                best = Some((gap, row));
            }
        }
        best.map(|(_, r)| r)
    }

    /// Write back an import duplicate's merged QSL/upload flags, STATE/
    /// COUNTRY, and (for a LoTW-twin fill) time/report/frequency/grid --
    /// see [`merge_matched_qso`]. Never touches any other column.
    fn update_flags_locked(conn: &Connection, id: i64, r: &QsoRecord) -> anyhow::Result<()> {
        conn.execute(
            "UPDATE qso_log SET qsl_rcvd=?1, qsl_sent=?2, lotw_sent=?3, lotw_rcvd=?4,
                eqsl_sent=?5, eqsl_rcvd=?6, qrz_qsl=?7, state=?8, country=?9, time_on=?10,
                mode=?11, time_off=?12, rst_sent=?13, rst_rcvd=?14, freq=?15, gridsquare=?16
             WHERE id=?17",
            params![
                r.confirmed as i64, r.qsl_sent as i64, r.lotw_sent as i64, r.lotw_rcvd as i64,
                r.eqsl_sent as i64, r.eqsl_rcvd as i64, r.qrz_qsl as i64, r.state, r.country,
                r.time_on, r.mode, r.time_off, r.rst_sent, r.rst_rcvd, r.freq, r.gridsquare, id,
            ],
        )?;
        Ok(())
    }

    /// Export the log as ADIF, optionally restricted to an inclusive
    /// `[start, end]` `YYYYMMDD` date range (mirrors `ExportLogSheet`'s
    /// date-range filter). Empty/`None` bounds are open.
    pub fn export_adif_range(&self, start: Option<&str>, end: Option<&str>) -> String {
        let mut out = String::from("FT8AF ADIF Export<eoh>\n");
        for r in self
            .list_qsos(i64::MAX, 0)
            .into_iter()
            .rev()
            .filter(|r| date_in_range(&r.qso_date, start, end))
        {
            out.push_str(&adif_qso_body(&r));
            out.push('\n');
        }
        out
    }

    // --- callsign/grid cache (`callsign_qth`, schema v7) ---------------------
    //
    // Four write paths, two priority tiers, matching Android's
    // `CallsignQTH`/`addCallsignQTH`/`fillCallsignQTHFromLog`:
    //   - Live decode (`engine.rs::publish_decodes`) is the most authoritative
    //     source -- directly heard over the air just now -- so it always wins
    //     (`upsert_callsign_qth_live`, `INSERT OR REPLACE`).
    //   - ADIF import and the FCC/Census first-run seed are both best-effort
    //     backfill for gaps only: neither may clobber a live-heard grid
    //     (`upsert_callsign_qth_if_absent_locked`, `INSERT OR IGNORE`).
    // Every write normalizes to uppercase grid4 (first 4 chars), matching the
    // `LENGTH(gridsquare) >= 4` / `UPPER(...)` convention used elsewhere in
    // this file (e.g. the ADIF export/dedup queries) -- a caller passing a
    // 6-char subsquare or stray whitespace still lands correctly.

    /// Record a station's grid from a live decode. Always overwrites any
    /// existing entry for that callsign -- see the section doc comment above.
    /// No-ops if `grid` isn't at least a 4-char locator.
    pub fn upsert_callsign_qth_live(&self, call: &str, grid: &str) {
        let Some(grid4) = normalize_grid4(grid) else { return };
        let _ = self.conn.lock().execute(
            "INSERT OR REPLACE INTO callsign_qth (callsign, grid, updated_at) VALUES (?1, ?2, ?3)",
            params![call.trim().to_ascii_uppercase(), grid4, crate::util::now_unix_ms()],
        );
    }

    /// Backfill a callsign's grid if it has none yet (ADIF import, FCC/Census
    /// seed). Never overwrites an existing row -- see the section doc comment
    /// above. Takes an already-locked connection so bulk callers (the ADIF
    /// import loop, the FCC seed's own transaction) can batch many of these
    /// under one transaction without re-locking the non-reentrant mutex.
    /// No-ops (returning `false`) if `grid` isn't at least a 4-char locator.
    /// Returns whether a row was actually inserted (`false` for a no-op grid
    /// or an already-present callsign, matching `INSERT OR IGNORE`).
    fn upsert_callsign_qth_if_absent_locked(conn: &Connection, call: &str, grid: &str) -> bool {
        let Some(grid4) = normalize_grid4(grid) else { return false };
        conn.execute(
            "INSERT OR IGNORE INTO callsign_qth (callsign, grid, updated_at) VALUES (?1, ?2, ?3)",
            params![call.trim().to_ascii_uppercase(), grid4, crate::util::now_unix_ms()],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// Batch-seed `callsign_qth` from an iterator of (callsign, grid) pairs
    /// -- the FCC/Census first-run import (`fcc_grid.rs`). One transaction
    /// for the whole batch (same batching rationale as
    /// `import_adif_with_progress`'s doc comment: a few hundred thousand
    /// individually-committed inserts would be a multi-minute per-row-fsync
    /// operation), never overwriting an existing row. Returns the number of
    /// rows actually inserted.
    ///
    /// `on_progress(processed, total, last_call)` fires every 500 items (with
    /// the callsign just processed, for a "Processing Callsign: W1AW" style
    /// readout) and once more, unconditionally, when the iterator is
    /// exhausted (clamped to `total`, `last_call` empty), so a caller always
    /// sees a definitive "done" tick even if fewer items were actually
    /// yielded than `total` predicted (e.g. some active licenses' ZIPs don't
    /// resolve in the Gazetteer).
    pub fn seed_callsign_qth_batch(
        &self,
        entries: impl Iterator<Item = (String, String)>,
        total: i64,
        mut on_progress: impl FnMut(i64, i64, &str),
    ) -> i64 {
        let conn = self.conn.lock();
        let _ = conn.execute_batch("BEGIN");
        let mut seeded = 0i64;
        let mut processed = 0i64;
        for (call, grid) in entries {
            if Self::upsert_callsign_qth_if_absent_locked(&conn, &call, &grid) {
                seeded += 1;
            }
            processed += 1;
            if processed % 500 == 0 {
                on_progress(processed.min(total), total, &call);
            }
        }
        let _ = conn.execute_batch("COMMIT");
        on_progress(total, total, "");
        seeded
    }

    /// Total rows in `callsign_qth` -- drives the frontend's first-run check
    /// (an empty table means the FCC/Census seed hasn't run yet).
    pub fn callsign_qth_count(&self) -> i64 {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM callsign_qth", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// The cached grid for a callsign, if any -- for the (not yet built) Map
    /// screen.
    pub fn qth_for_call(&self, call: &str) -> Option<String> {
        self.conn
            .lock()
            .query_row(
                "SELECT grid FROM callsign_qth WHERE callsign = ?1",
                params![call.trim().to_ascii_uppercase()],
                |r| r.get(0),
            )
            .ok()
    }
}

/// Normalize a candidate grid locator to an uppercase grid4, or `None` if
/// it's too short to be one. Shared by every `callsign_qth` write path.
fn normalize_grid4(grid: &str) -> Option<String> {
    let g = grid.trim();
    if g.len() < 4 {
        return None;
    }
    Some(crate::util::grid4(&g.to_ascii_uppercase()))
}

fn row_to_qso(r: &rusqlite::Row) -> rusqlite::Result<QsoRecord> {
    Ok(QsoRecord {
        id: r.get(0)?,
        call: r.get(1)?,
        gridsquare: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        mode: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        rst_sent: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        rst_rcvd: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        qso_date: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        time_on: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        qso_date_off: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        time_off: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        band: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        freq: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
        station_callsign: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
        my_gridsquare: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
        comment: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
        confirmed: r.get::<_, Option<i64>>(15)?.unwrap_or(0) != 0,
        qsl_sent: r.get::<_, Option<i64>>(16)?.unwrap_or(0) != 0,
        lotw_sent: r.get::<_, Option<i64>>(17)?.unwrap_or(0) != 0,
        lotw_rcvd: r.get::<_, Option<i64>>(18)?.unwrap_or(0) != 0,
        eqsl_sent: r.get::<_, Option<i64>>(19)?.unwrap_or(0) != 0,
        eqsl_rcvd: r.get::<_, Option<i64>>(20)?.unwrap_or(0) != 0,
        qrz_qsl: r.get::<_, Option<i64>>(21)?.unwrap_or(0) != 0,
        state: r.get::<_, Option<String>>(22)?.unwrap_or_default(),
        country: r.get::<_, Option<String>>(23)?.unwrap_or_default(),
        synced_cloudlog: r.get::<_, Option<i64>>(24)?.unwrap_or(0) != 0,
        synced_qrz: r.get::<_, Option<i64>>(25)?.unwrap_or(0) != 0,
    })
}

/// One QSO as a single ADIF record (ending in `<eor>`, no trailing newline).
/// Shared by [`Db::export_adif_range`] (which adds `\n` after each) and
/// [`adif_record`] (used verbatim in the WSJT-X "Logged ADIF" UDP message so
/// companion loggers — JTAlert, N1MM — capture the QSO the same way).
fn adif_qso_body(r: &QsoRecord) -> String {
    let flag = |b: bool| if b { "Y" } else { "N" };
    let mut out = String::new();
    out.push_str(&adif_field("call", &r.call));
    out.push_str(&format!("<QSL_RCVD:1>{} ", flag(r.confirmed)));
    adif_opt(&mut out, "gridsquare", &r.gridsquare);
    adif_opt(&mut out, "mode", &r.mode);
    adif_opt(&mut out, "rst_sent", &r.rst_sent);
    adif_opt(&mut out, "rst_rcvd", &r.rst_rcvd);
    adif_opt(&mut out, "qso_date", &r.qso_date);
    adif_opt(&mut out, "time_on", &r.time_on);
    adif_opt(&mut out, "qso_date_off", &r.qso_date_off);
    adif_opt(&mut out, "time_off", &r.time_off);
    adif_opt(&mut out, "band", &r.band);
    adif_opt(&mut out, "freq", &r.freq);
    adif_opt(&mut out, "station_callsign", &r.station_callsign);
    adif_opt(&mut out, "my_gridsquare", &r.my_gridsquare);
    adif_opt(&mut out, "state", &r.state);
    adif_opt(&mut out, "country", &r.country);
    out.push_str(&format!("<QSL_SENT:1>{} ", flag(r.qsl_sent)));
    out.push_str(&format!("<LOTW_QSL_SENT:1>{} ", flag(r.lotw_sent)));
    out.push_str(&format!("<LOTW_QSL_RCVD:1>{} ", flag(r.lotw_rcvd)));
    out.push_str(&format!("<EQSL_QSL_SENT:1>{} ", flag(r.eqsl_sent)));
    out.push_str(&format!("<EQSL_QSL_RCVD:1>{} ", flag(r.eqsl_rcvd)));
    // No standard ADIF field for a QRZ.com logbook confirmation; APP_FT8AF_*
    // mirrors the Android app's own extension field name.
    out.push_str(&format!("<APP_FT8AF_QRZ_CONFIRMED:1>{} ", flag(r.qrz_qsl)));
    out.push_str(&format!("<comment:{}>{} <eor>", r.comment.len(), r.comment));
    out
}

/// One QSO as a single ADIF record line (ending in `<eor>`), matching the field
/// set/order of [`Db::export_adif_range`]. Used for the WSJT-X "Logged ADIF" UDP
/// message so companion loggers (JTAlert, N1MM) can capture the QSO verbatim.
pub fn adif_record(r: &QsoRecord) -> String {
    adif_qso_body(r)
}

/// ADIF `MODE` values that are a generic digital-mode family rather than the
/// specific submode (e.g. `MFSK` covers FT4/FT8/FSQ/...). Seen in practice: a
/// program that round-trips a log through its own ADIF import/export can
/// widen FT4/FT8 to the family name when it has no `SUBMODE` tag to recover
/// the specific one from, while a fresh export from the original source still
/// has it. [`merge_matched_qso`] prefers the specific name when the two
/// disagree.
const GENERIC_MODE_FAMILIES: &[&str] = &["MFSK", "PSK", "DATA"];

fn is_generic_mode_family(mode: &str) -> bool {
    GENERIC_MODE_FAMILIES.contains(&mode.trim().to_ascii_uppercase().as_str())
}

/// HHMM[SS] (a dropped leading zero allowed) to minutes past midnight, or
/// `None`. Ports Android's `DatabaseOpr.minuteOfDay` exactly.
fn minute_of_day(time: &str) -> Option<i32> {
    let mut digits: String = time.chars().filter(char::is_ascii_digit).collect();
    if digits.len() % 2 == 1 {
        digits.insert(0, '0');
    }
    if digits.len() < 4 {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// SQL test for a LoTW-style stored row (see [`is_lotw_style_record`]): no
/// report received, `TIME_OFF` equals `TIME_ON` to the minute. Unlike
/// Android (which stores "no report" as the literal sentinel strings `-120`/
/// `-100`), desktop's ADIF import (`adif::adif_record_to_qso`) leaves
/// `rst_rcvd` as a genuinely empty string when the field is absent, so that's
/// the test here.
const LOTW_ROW_SQL: &str = "(rst_rcvd = '' AND length(time_off) >= 4 \
    AND substr(time_on,1,4) = substr(time_off,1,4))";

/// A LoTW-style record: no report received and `TIME_OFF` present, equal to
/// `TIME_ON` to the minute. One QSO can appear twice in a QRZ.com/CQRLOG
/// export — the operator's full record and a copy QRZ.com added from a LoTW
/// confirmation (no RST, usually no FREQ, `TIME_OFF` = `TIME_ON`, start time
/// a few minutes off the full record's). Confirmed on the user's real
/// ~54,500-record QRZ.com export: ADIF `RST_RCVD` is absent (not a literal
/// sentinel) on ~10,900 such copies. Ports Android's `DatabaseOpr.isLotwStyle`.
fn is_lotw_style_record(r: &QsoRecord) -> bool {
    if !r.rst_rcvd.trim().is_empty() {
        return false;
    }
    match (minute_of_day(&r.time_on), minute_of_day(&r.time_off)) {
        (Some(on), Some(off)) => on == off,
        _ => false,
    }
}

/// What a match (either [`Db::find_duplicate_locked`]'s exact minute match,
/// or [`Db::find_lotw_twin_locked`]'s fallback) may still change on an
/// existing QSO:
///
/// - New QSL/upload flags are only ever turned on, never cleared.
/// - STATE/COUNTRY are only ever filled in when the existing value is blank.
/// - `MODE` is only ever replaced with a specific submode (FT4, FT8, ...)
///   when the existing value is just the generic family name it belongs to
///   (see [`GENERIC_MODE_FAMILIES`]) — confirmed on a real ~54,500-QSO log
///   where `MODE` was excluded from duplicate matching entirely (see
///   `find_duplicate_locked`) for exactly this reason: CQRLOG's own ADIF
///   round-trip had widened every FT4/FT8 contact to `MFSK` with no
///   `SUBMODE` tag to recover from, while a QRZ.com re-export of the same
///   log still had the real mode, and comparing `MODE` during matching made
///   ~7,100 of those QSOs fail to match their own duplicate.
/// - **LoTW-twin fill**: when the *existing* row is [`is_lotw_style_record`]
///   and `imported` is not, the existing row adopts `imported`'s
///   `time_on`/`time_off`/`rst_sent`/`rst_rcvd`/frequency (if `imported` has
///   one) and grid (if the existing row's is blank) — the row that started
///   as the bare LoTW copy becomes the full QSO record. Ports Android's
///   `DatabaseOpr.fillLotwRow`. The *reverse* direction (existing is the full
///   record, `imported` is the LoTW-style copy) never reaches this branch, so
///   a LoTW-style copy's `RST_SENT` can never clobber a real stored report —
///   this exact regression was fixed on Android in commit `d82697f8`
///   ("a LoTW-style record's RST_SENT never overwrites the stored report").
///   Outside that branch, `TIME_ON` still only ever moves to a *more
///   precise* value (comparing only the shared `HHMM` prefix, so a longer
///   value is never less correct), never a less precise one, regardless of
///   which side created the row.
///
/// Otherwise a match never overwrites data the app, or the user editing in
/// place, already has. Returns `None` when `imported` wouldn't actually
/// change anything, so the caller can skip a pointless `UPDATE`.
fn merge_matched_qso(existing: &QsoRecord, imported: &QsoRecord) -> Option<QsoRecord> {
    let mut merged = existing.clone();
    let mut changed = false;
    macro_rules! or_in {
        ($field:ident) => {
            if imported.$field && !merged.$field {
                merged.$field = true;
                changed = true;
            }
        };
    }
    or_in!(confirmed);
    or_in!(qsl_sent);
    or_in!(lotw_sent);
    or_in!(lotw_rcvd);
    or_in!(eqsl_sent);
    or_in!(eqsl_rcvd);
    or_in!(qrz_qsl);
    if is_generic_mode_family(&merged.mode) && !is_generic_mode_family(&imported.mode) {
        merged.mode = imported.mode.clone();
        changed = true;
    }
    if merged.state.is_empty() && !imported.state.is_empty() {
        merged.state = imported.state.clone();
        changed = true;
    }
    if merged.country.is_empty() && !imported.country.is_empty() {
        merged.country = imported.country.clone();
        changed = true;
    }
    if is_lotw_style_record(&merged) && !is_lotw_style_record(imported) {
        merged.time_on = imported.time_on.clone();
        merged.time_off = imported.time_off.clone();
        merged.rst_sent = imported.rst_sent.clone();
        merged.rst_rcvd = imported.rst_rcvd.clone();
        if !imported.freq.trim().is_empty() {
            merged.freq = imported.freq.clone();
        }
        if merged.gridsquare.trim().is_empty() {
            merged.gridsquare = imported.gridsquare.clone();
        }
        changed = true;
    } else if imported.time_on.len() > merged.time_on.len() {
        merged.time_on = imported.time_on.clone();
        changed = true;
    }
    changed.then_some(merged)
}

fn adif_field(name: &str, value: &str) -> String {
    format!("<{}:{}>{} ", name, value.len(), value)
}

fn adif_opt(out: &mut String, name: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&adif_field(name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_dedup_and_export() {
        let db = Db::open_in_memory().unwrap();
        let rec = QsoRecord {
            call: "K1ABC".into(),
            gridsquare: "FN42".into(),
            mode: "FT8".into(),
            rst_sent: "-12".into(),
            rst_rcvd: "-08".into(),
            qso_date: "20260604".into(),
            time_on: "123045".into(),
            band: "20m".into(),
            freq: "14.074000".into(),
            station_callsign: "K0XYZ".into(),
            my_gridsquare: "EN37".into(),
            ..Default::default()
        };
        db.insert_qso(&rec).unwrap();
        db.insert_qso(&rec).unwrap(); // dedup -> still one row
        assert_eq!(db.count(), 1);

        let adif = db.export_adif();
        assert!(adif.starts_with("FT8AF ADIF Export<eoh>\n"));
        assert!(adif.contains("<call:5>K1ABC "));
        assert!(adif.contains("<gridsquare:4>FN42 "));
        assert!(adif.contains("<band:3>20m "));
        assert!(adif.trim_end().ends_with("<eor>"));
    }

    #[test]
    fn export_writes_the_granular_qsl_flags_and_logged_state_country() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1ABC".into(),
            mode: "FT8".into(),
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            confirmed: true,
            lotw_rcvd: true,
            eqsl_sent: true,
            qrz_qsl: true,
            state: "CA".into(),
            country: "UNITED STATES".into(),
            ..Default::default()
        })
        .unwrap();
        let adif = db.export_adif();
        assert!(adif.contains("<state:2>CA "));
        assert!(adif.contains("<country:13>UNITED STATES "));
        assert!(adif.contains("<QSL_RCVD:1>Y "));
        assert!(adif.contains("<LOTW_QSL_RCVD:1>Y "));
        assert!(adif.contains("<EQSL_QSL_SENT:1>Y "));
        assert!(adif.contains("<APP_FT8AF_QRZ_CONFIRMED:1>Y "));
        // Flags that weren't set still round-trip explicitly as N, not omitted.
        assert!(adif.contains("<QSL_SENT:1>N "));
        assert!(adif.contains("<LOTW_QSL_SENT:1>N "));
        assert!(adif.contains("<EQSL_QSL_RCVD:1>N "));

        // And a re-import of the app's own export recovers all of it.
        let mut imported = crate::adif::parse_adif(&adif);
        let qso = crate::adif::adif_record_to_qso(&imported.remove(0).fields).unwrap();
        assert!(qso.confirmed && qso.lotw_rcvd && qso.eqsl_sent && qso.qrz_qsl);
        assert!(!qso.qsl_sent && !qso.lotw_sent && !qso.eqsl_rcvd);
        assert_eq!(qso.state, "CA");
        assert_eq!(qso.country, "UNITED STATES");
    }

    #[test]
    fn export_omits_the_non_adif_qsl_manual_tag() {
        // QSL_MANUAL is not in the ADIF spec (issue #697), and strict importers
        // reject or silently drop unknown tags. Both emitters are covered: the
        // file export and adif_record(), which is what goes out over the WSJT-X
        // "Logged ADIF" UDP message to JTAlert/N1MM.
        let db = Db::open_in_memory().unwrap();
        let confirmed = rec("K1ABC", "20260604", "120000", true);
        let unconfirmed = rec("W1AW", "20260604", "130000", false);
        db.insert_qso(&confirmed).unwrap();
        db.insert_qso(&unconfirmed).unwrap();

        let adif = db.export_adif();
        assert!(!adif.contains("QSL_MANUAL"));
        assert!(!adif_record(&confirmed).contains("QSL_MANUAL"));

        // QSL_RCVD is a real ADIF field and still carries the confirmed flag,
        // which is the only QSL information these records ever held.
        assert!(adif.contains("<call:5>K1ABC <QSL_RCVD:1>Y "));
        assert!(adif.contains("<call:4>W1AW <QSL_RCVD:1>N "));
        assert!(adif_record(&confirmed).contains("<QSL_RCVD:1>Y "));
    }

    #[test]
    fn config_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get_config("my_call"), None);
        db.set_config("my_call", "K0XYZ").unwrap();
        db.set_config("my_call", "K0ABC").unwrap();
        assert_eq!(db.get_config("my_call").as_deref(), Some("K0ABC"));
    }

    fn rec(call: &str, date: &str, time: &str, confirmed: bool) -> QsoRecord {
        QsoRecord {
            call: call.into(),
            mode: "FT8".into(),
            qso_date: date.into(),
            time_on: time.into(),
            confirmed,
            ..Default::default()
        }
    }

    #[test]
    fn confirmed_status_round_trips_and_reflects_in_adif() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "120000", true)).unwrap();
        db.insert_qso(&rec("W1AW", "20260604", "130000", false)).unwrap();

        let rows = db.list_qsos(10, 0);
        let k1abc = rows.iter().find(|r| r.call == "K1ABC").unwrap();
        let w1aw = rows.iter().find(|r| r.call == "W1AW").unwrap();
        assert!(k1abc.confirmed);
        assert!(!w1aw.confirmed);

        let adif = db.export_adif();
        // Confirmed QSO exports QSL_RCVD:Y, unconfirmed exports N.
        assert!(adif.contains("<call:5>K1ABC <QSL_RCVD:1>Y "));
        assert!(adif.contains("<call:4>W1AW <QSL_RCVD:1>N "));
    }

    #[test]
    fn editing_a_qso_toggles_confirmed_via_insert() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_qso(&rec("K1ABC", "20260604", "120000", false)).unwrap();
        assert!(!db.list_qsos(1, 0)[0].confirmed);
        // save_qso re-inserts the same dedup key with confirmed flipped.
        let mut edited = rec("K1ABC", "20260604", "120000", true);
        edited.id = Some(id);
        db.insert_qso(&edited).unwrap();
        assert_eq!(db.count(), 1);
        assert!(db.list_qsos(1, 0)[0].confirmed);
    }

    #[test]
    fn search_filters_by_callsign_and_confirmation() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "120000", true)).unwrap();
        db.insert_qso(&rec("K1XYZ", "20260604", "130000", false)).unwrap();
        db.insert_qso(&rec("W1AW", "20260604", "140000", true)).unwrap();

        // Substring match, case-insensitive.
        let k1 = db.search_qsos("k1", ConfirmedFilter::All, 100, 0);
        assert_eq!(k1.len(), 2);
        assert!(k1.iter().all(|r| r.call.contains("K1")));

        // Confirmation filter.
        let confirmed = db.search_qsos("", ConfirmedFilter::Confirmed, 100, 0);
        assert_eq!(confirmed.len(), 2);
        let unconfirmed = db.search_qsos("", ConfirmedFilter::Unconfirmed, 100, 0);
        assert_eq!(unconfirmed.len(), 1);
        assert_eq!(unconfirmed[0].call, "K1XYZ");

        // Combined.
        let combined = db.search_qsos("k1", ConfirmedFilter::Confirmed, 100, 0);
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].call, "K1ABC");
    }

    #[test]
    fn search_qsos_matches_predicate_and_paginates() {
        let db = Db::open_in_memory().unwrap();
        let recs = [
            rec("K1ABC", "20260604", "120000", true),
            rec("K1XYZ", "20260604", "130000", false),
            rec("W1AW", "20260604", "140000", true),
            rec("K1ABC/P", "20260604", "150000", false),
        ];
        for r in &recs {
            db.insert_qso(r).unwrap();
        }

        // Parity: the SQL-level search returns exactly the rows the pure
        // qso_matches predicate accepts, across query/filter combinations.
        for (q, f) in [
            ("", ConfirmedFilter::All),
            ("k1", ConfirmedFilter::All),
            ("k1", ConfirmedFilter::Confirmed),
            ("w", ConfirmedFilter::Unconfirmed),
            ("zzz", ConfirmedFilter::All),
        ] {
            let got: std::collections::BTreeSet<String> = db
                .search_qsos(q, f, i64::MAX, 0)
                .into_iter()
                .map(|r| r.call)
                .collect();
            let want: std::collections::BTreeSet<String> = db
                .list_qsos(i64::MAX, 0)
                .into_iter()
                .filter(|r| qso_matches(r, q, f))
                .map(|r| r.call)
                .collect();
            assert_eq!(got, want, "query={q:?} filter={f:?}");
        }

        // LIMIT/OFFSET paginate at the DB layer, newest-first, with no overlap.
        let page1 = db.search_qsos("", ConfirmedFilter::All, 2, 0);
        let page2 = db.search_qsos("", ConfirmedFilter::All, 2, 2);
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_eq!(page1[0].call, "K1ABC/P"); // last inserted -> highest id
        assert!(page1.iter().all(|a| page2.iter().all(|b| a.call != b.call)));
    }

    #[test]
    fn worked_calls_matches_band_and_mode_case_insensitively() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1ABC".into(),
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "w1aw".into(), // lower-case call: result is still upper-cased
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "20M".into(), // differently-cased band: still matches "20m"
            mode: "ft8".into(), // differently-cased mode: still matches "FT8"
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1XYZ".into(), // other band -- excluded
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "40m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1QRS".into(), // same band, other mode -- excluded
            qso_date: "20260604".into(),
            time_on: "123000".into(),
            band: "20m".into(),
            mode: "FT4".into(),
            ..Default::default()
        })
        .unwrap();

        let mut worked = db.worked_calls("20m", "FT8");
        worked.sort();
        assert_eq!(worked, vec!["K1ABC".to_string(), "W1AW".to_string()]);

        assert!(db.worked_calls("30m", "FT8").is_empty());
    }

    #[test]
    fn worked_calls_any_band_matches_mode_only_case_insensitively() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1ABC".into(),
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "w1aw".into(), // lower-case call, other band: both still counted
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "40m".into(),
            mode: "ft8".into(), // differently-cased mode: still matches "FT8"
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1QRS".into(), // other mode -- excluded
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT4".into(),
            ..Default::default()
        })
        .unwrap();

        let mut worked = db.worked_calls_any_band("FT8");
        worked.sort();
        assert_eq!(worked, vec!["K1ABC".to_string(), "W1AW".to_string()]);
    }

    #[test]
    fn worked_grids_ignores_band_mode_and_short_grids() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1ABC".into(),
            gridsquare: "fn31".into(), // lower-case: result is still upper-cased
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1XYZ".into(), // different band/mode: still counts (not scoped)
            gridsquare: "EM12".into(),
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "40m".into(),
            mode: "FT4".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1QRS".into(),
            gridsquare: "FN31".into(), // duplicate of the first (case-insensitive)
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1SHORT".into(), // 2-char grid: too coarse, excluded
            gridsquare: "EM".into(),
            qso_date: "20260604".into(),
            time_on: "123000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();

        let mut grids = db.worked_grids();
        grids.sort();
        assert_eq!(grids, vec!["EM12".to_string(), "FN31".to_string()]);
    }

    #[test]
    fn worked_prefixes_derives_and_dedupes_across_bands() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "w1aw".into(), // lower-case: result is still upper-cased
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1XYZ".into(), // different band: still counts (not scoped)
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "40m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "VE3ABC".into(),
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "FN42".into(), // not a callsign -- yields no prefix, excluded
            qso_date: "20260604".into(),
            time_on: "123000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();

        let prefixes = db.worked_prefixes();
        assert_eq!(prefixes, vec!["K1".to_string(), "VE3".to_string(), "W1".to_string()]);
    }

    #[test]
    fn worked_dxcc_prefixes_derives_and_dedupes_across_bands() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "w1aw".into(), // lower-case: still resolves/upper-cases fine
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1XYZ".into(), // different band, same DXCC entity ("K") as W1AW -- dedupes
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "40m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "JA1ABC".into(),
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            // Not a callsign, but prefix matching can't tell: "FN42" starts
            // with "F" (France's single-letter DXCC prefix), so it resolves
            // the same way the original SQL/Java would -- a known, faithful
            // quirk of longest-prefix matching, not something to special-case.
            call: "FN42".into(),
            qso_date: "20260604".into(),
            time_on: "123000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();

        let prefixes = db.worked_dxcc_prefixes();
        assert_eq!(prefixes, vec!["F".to_string(), "JA".to_string(), "K".to_string()]);
    }

    #[test]
    fn worked_cq_zones_derives_and_dedupes_across_bands() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "W1AW".into(), // United States -- CQ zone 5
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1XYZ".into(), // also zone 5 -- dedupes with W1AW
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "40m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "JA1ABC".into(), // Japan -- CQ zone 25
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();

        let zones = db.worked_cq_zones();
        assert_eq!(zones, vec![5, 25]);
    }

    #[test]
    fn worked_states_matches_band_and_mode_and_resolves_grid_and_logged_state() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1ABC".into(),
            gridsquare: "FN42".into(), // resolves to MA via us_grid_states.json
            qso_date: "20260604".into(),
            time_on: "120000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "W1AW".into(), // US call, no grid: trusts the logged STATE
            state: "ct".into(),  // lower-case: result is still upper-cased
            qso_date: "20260604".into(),
            time_on: "121000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "VK6ABC".into(), // non-US call, no grid, "WA" isn't trustworthy here
            state: "WA".into(),    // Western Australia, not Washington
            qso_date: "20260604".into(),
            time_on: "122000".into(),
            band: "20m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();
        db.insert_qso(&QsoRecord {
            call: "K1QRS".into(), // different band -- excluded
            gridsquare: "EM12".into(),
            qso_date: "20260604".into(),
            time_on: "123000".into(),
            band: "40m".into(),
            mode: "FT8".into(),
            ..Default::default()
        })
        .unwrap();

        let mut states = db.worked_states("20m", "FT8");
        states.sort();
        assert_eq!(states, vec!["CT".to_string(), "MA".to_string()]);

        assert!(db.worked_states("30m", "FT8").is_empty());
    }

    #[test]
    fn qso_matches_predicate() {
        let confirmed = rec("K1ABC", "20260604", "120000", true);
        let unconfirmed = rec("W1AW", "20260604", "120000", false);
        assert!(qso_matches(&confirmed, "", ConfirmedFilter::All));
        assert!(qso_matches(&confirmed, "abc", ConfirmedFilter::All));
        assert!(!qso_matches(&confirmed, "zzz", ConfirmedFilter::All));
        assert!(qso_matches(&confirmed, "", ConfirmedFilter::Confirmed));
        assert!(!qso_matches(&confirmed, "", ConfirmedFilter::Unconfirmed));
        assert!(qso_matches(&unconfirmed, "", ConfirmedFilter::Unconfirmed));
        assert!(!qso_matches(&unconfirmed, "", ConfirmedFilter::Confirmed));
    }

    #[test]
    fn export_date_range_filters_rows() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("A1AA", "20260601", "120000", false)).unwrap();
        db.insert_qso(&rec("B2BB", "20260615", "120000", false)).unwrap();
        db.insert_qso(&rec("C3CC", "20260630", "120000", false)).unwrap();

        let mid = db.export_adif_range(Some("20260610"), Some("20260620"));
        assert!(!mid.contains("A1AA"));
        assert!(mid.contains("B2BB"));
        assert!(!mid.contains("C3CC"));

        // Open-ended bounds.
        let from = db.export_adif_range(Some("20260615"), None);
        assert!(!from.contains("A1AA"));
        assert!(from.contains("B2BB"));
        assert!(from.contains("C3CC"));

        // No bounds == whole log.
        let all = db.export_adif_range(None, None);
        assert!(all.contains("A1AA") && all.contains("B2BB") && all.contains("C3CC"));
    }

    #[test]
    fn date_in_range_predicate() {
        assert!(date_in_range("20260615", None, None));
        assert!(date_in_range("20260615", Some(""), Some("")));
        assert!(date_in_range("20260615", Some("20260601"), Some("20260630")));
        assert!(!date_in_range("20260701", Some("20260601"), Some("20260630")));
        assert!(!date_in_range("20260501", Some("20260601"), None));
        assert!(date_in_range("20260615", None, Some("20260615")));
        assert!(!date_in_range("20260616", None, Some("20260615")));

        // Malformed dates must not sneak past a bound via lexicographic ordering.
        assert!(!date_in_range("BAD", Some("20240101"), None));
        assert!(!date_in_range("2026061", Some("20260101"), None)); // 7 digits
        assert!(!date_in_range("2026-06-15", None, Some("20260630"))); // wrong shape
        assert!(!date_in_range("", Some("20260101"), None));
        // With no bounds the range is fully open, so even a malformed value passes.
        assert!(date_in_range("BAD", None, None));
    }

    #[test]
    fn sync_flags_are_idempotent_per_service() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_qso(&rec("K1ABC", "20260604", "120000", false)).unwrap();
        assert!(!db.is_synced(id, "cloudlog"));
        assert!(!db.is_synced(id, "qrz"));

        db.mark_synced(id, "cloudlog").unwrap();
        assert!(db.is_synced(id, "cloudlog"));
        assert!(!db.is_synced(id, "qrz"));

        // Unknown service is a no-op, not an error.
        db.mark_synced(id, "bogus").unwrap();
        assert!(!db.is_synced(id, "bogus"));
    }

    #[test]
    fn migrates_v1_log_in_place() {
        // Simulate an existing v1 database (no qsl_rcvd / sync columns) and
        // confirm the v2 migration adds them without dropping rows.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER
            );
            CREATE UNIQUE INDEX qso_log_dedup_idx
                ON qso_log(call, qso_date, time_on, mode);
            INSERT INTO qso_log(call, mode, qso_date, time_on)
                VALUES('K1ABC', 'FT8', '20260604', '120000');
            PRAGMA user_version = 1;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        let rows = db.list_qsos(10, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].call, "K1ABC");
        assert!(!rows[0].confirmed); // defaults to unconfirmed
    }

    #[test]
    fn migrates_v2_log_in_place_and_adds_the_import_dedup_index() {
        // A v2 database (has qsl_rcvd/sync columns, but not yet the v3 import
        // dedup index) must upgrade cleanly and end up with the new index.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER,
                qsl_rcvd INTEGER NOT NULL DEFAULT 0,
                synced_cloudlog INTEGER NOT NULL DEFAULT 0,
                synced_qrz INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX qso_log_dedup_idx
                ON qso_log(call, qso_date, time_on, mode);
            INSERT INTO qso_log(call, mode, qso_date, time_on)
                VALUES('K1ABC', 'FT8', '20260604', '120000');
            PRAGMA user_version = 2;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        assert_eq!(db.count(), 1);
        let has_index: bool = db
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='qso_log_import_dedup_idx'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0;
        assert!(has_index, "v3 migration must create the import dedup index");
    }

    #[test]
    fn migrates_v3_log_in_place_and_adds_qsl_state_country_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER,
                qsl_rcvd INTEGER NOT NULL DEFAULT 0,
                synced_cloudlog INTEGER NOT NULL DEFAULT 0,
                synced_qrz INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX qso_log_dedup_idx
                ON qso_log(call, qso_date, time_on, mode);
            INSERT INTO qso_log(call, mode, qso_date, time_on, qsl_rcvd)
                VALUES('K1ABC', 'FT8', '20260604', '120000', 1);
            PRAGMA user_version = 3;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        let rows = db.list_qsos(10, 0);
        assert_eq!(rows.len(), 1);
        // The pre-existing paper-received flag survives the upgrade...
        assert!(rows[0].confirmed);
        // ...and every new column defaults to false/empty, not an error.
        assert!(!rows[0].lotw_rcvd && !rows[0].qsl_sent && !rows[0].qrz_qsl);
        assert_eq!(rows[0].state, "");
        assert_eq!(rows[0].country, "");
    }

    #[test]
    fn migrates_v4_log_in_place_and_replaces_the_import_dedup_index() {
        // A v4 database still has the old mode-including dedup index; the v5
        // migration must drop it and create the new mode-free one, without
        // losing any rows.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER,
                qsl_rcvd INTEGER NOT NULL DEFAULT 0,
                synced_cloudlog INTEGER NOT NULL DEFAULT 0,
                synced_qrz INTEGER NOT NULL DEFAULT 0,
                qsl_sent INTEGER NOT NULL DEFAULT 0,
                lotw_sent INTEGER NOT NULL DEFAULT 0,
                lotw_rcvd INTEGER NOT NULL DEFAULT 0,
                eqsl_sent INTEGER NOT NULL DEFAULT 0,
                eqsl_rcvd INTEGER NOT NULL DEFAULT 0,
                qrz_qsl INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT '',
                country TEXT NOT NULL DEFAULT ''
            );
            CREATE UNIQUE INDEX qso_log_dedup_idx
                ON qso_log(call, qso_date, time_on, mode);
            CREATE INDEX qso_log_import_dedup_idx
                ON qso_log(UPPER(call), qso_date, UPPER(mode), substr(time_on || '000000', 1, 4));
            INSERT INTO qso_log(call, mode, qso_date, time_on)
                VALUES('K1ABC', 'FT8', '20260604', '120000');
            PRAGMA user_version = 4;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        assert_eq!(db.count(), 1, "existing rows must survive the migration");

        // Querying sqlite_master for the index's own SQL confirms it no
        // longer references mode, rather than just checking it exists.
        let index_sql: String = db
            .conn
            .lock()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='qso_log_import_dedup_idx'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !index_sql.to_ascii_uppercase().contains("MODE"),
            "the replaced index must not reference mode: {index_sql}"
        );
    }

    #[test]
    fn migrates_v5_log_in_place_and_adds_the_lotw_twin_index() {
        // The real upgrade path for an existing user's database (including
        // the one this session's own earlier mode-dedup fix just migrated to
        // v5): rows and the v5 index must survive, and the new v6 index
        // must appear alongside it.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER,
                qsl_rcvd INTEGER NOT NULL DEFAULT 0,
                synced_cloudlog INTEGER NOT NULL DEFAULT 0,
                synced_qrz INTEGER NOT NULL DEFAULT 0,
                qsl_sent INTEGER NOT NULL DEFAULT 0,
                lotw_sent INTEGER NOT NULL DEFAULT 0,
                lotw_rcvd INTEGER NOT NULL DEFAULT 0,
                eqsl_sent INTEGER NOT NULL DEFAULT 0,
                eqsl_rcvd INTEGER NOT NULL DEFAULT 0,
                qrz_qsl INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT '',
                country TEXT NOT NULL DEFAULT ''
            );
            CREATE UNIQUE INDEX qso_log_dedup_idx
                ON qso_log(call, qso_date, time_on, mode);
            CREATE INDEX qso_log_import_dedup_idx
                ON qso_log(UPPER(call), qso_date, substr(time_on || '000000', 1, 4));
            INSERT INTO qso_log(call, mode, qso_date, time_on)
                VALUES('K1ABC', 'FT8', '20260604', '120000');
            PRAGMA user_version = 5;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        assert_eq!(db.count(), 1, "existing rows must survive the migration");
        let has_index: bool = db
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='qso_log_lotw_twin_idx'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0;
        assert!(has_index, "v6 migration must create the LoTW-twin index");
    }

    #[test]
    fn import_adif_inserts_new_qsos() {
        let db = Db::open_in_memory().unwrap();
        let adif = "FT8AF ADIF Export<eoh>\n\
            <call:5>K1ABC <gridsquare:4>FN42 <mode:3>FT8 <qso_date:8>20260604 \
            <time_on:6>123045 <band:3>20m <eor>\n";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.count(), 1);
        let row = &db.list_qsos(10, 0)[0];
        assert_eq!(row.call, "K1ABC");
        assert_eq!(row.gridsquare, "FN42");
        assert_eq!(row.band, "20m");
    }

    #[test]
    fn import_adif_skips_a_record_with_no_call() {
        let db = Db::open_in_memory().unwrap();
        let adif = "<eoh><qso_date:8>20260604 <time_on:6>120000 <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 0, duplicates: 0, skipped: 1 });
        assert_eq!(db.count(), 0);
    }

    #[test]
    fn import_adif_matches_duplicates_to_the_minute_ignoring_band() {
        // The app logged this QSO itself with exact seconds and a band; an
        // external logger's export rounds TIME_ON to the minute and disagrees
        // on band. Per the Android rule, that's still the same QSO: band is
        // never part of the match, and TIME_ON only needs to agree to the minute.
        let db = Db::open_in_memory().unwrap();
        let mut existing = rec("K1ABC", "20260604", "123045", false);
        existing.band = "20m".into();
        db.insert_qso(&existing).unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:4>1230 \
            <band:3>40m <QSL_RCVD:1>Y <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 0, duplicates: 1, skipped: 0 });
        assert_eq!(db.count(), 1, "must not insert a second row for the same QSO");

        // The existing row is never overwritten (band stays what the app logged)...
        let row = &db.list_qsos(10, 0)[0];
        assert_eq!(row.band, "20m");
        // ...except its QSL_RCVD flag, which the import is allowed to turn on.
        assert!(row.confirmed);
    }

    #[test]
    fn import_adif_never_clears_an_already_confirmed_flag() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "123045", true)).unwrap();
        // Re-importing a copy that has no QSL_RCVD at all must not un-confirm it.
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 <eor>";
        db.import_adif(adif);
        assert!(db.list_qsos(10, 0)[0].confirmed);
    }

    #[test]
    fn import_adif_upgrades_time_precision_when_cqrlog_then_qrz_imported() {
        // CQRLOG's export gives only HHMM; importing it first creates the row
        // with that coarse time. QRZ.com's export of the same QSO carries the
        // real HHMMSS -- importing it second must upgrade the stored time,
        // not leave the row stuck at CQRLOG's rounded value.
        let db = Db::open_in_memory().unwrap();
        let cqrlog = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:4>1230 <eor>";
        let qrz = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 <eor>";
        assert_eq!(db.import_adif(cqrlog), ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.import_adif(qrz), ImportSummary { imported: 0, duplicates: 1, skipped: 0 });
        assert_eq!(db.count(), 1, "must still be treated as the same QSO, not a second row");
        assert_eq!(db.list_qsos(10, 0)[0].time_on, "123045");
    }

    #[test]
    fn import_adif_keeps_time_precision_when_qrz_then_cqrlog_imported() {
        // Reverse order: QRZ (precise) first, then CQRLOG (coarse) -- the
        // precise time must survive, not get overwritten by the rounded one.
        let db = Db::open_in_memory().unwrap();
        let qrz = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 <eor>";
        let cqrlog = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:4>1230 <eor>";
        db.import_adif(qrz);
        db.import_adif(cqrlog);
        assert_eq!(db.count(), 1);
        assert_eq!(db.list_qsos(10, 0)[0].time_on, "123045");
    }

    #[test]
    fn import_adif_turns_on_granular_qsl_flags_and_fills_blank_state_country() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "123045", false)).unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 \
            <LOTW_QSL_RCVD:1>Y <EQSL_QSL_SENT:1>Y <STATE:2>CA <COUNTRY:13>UNITED STATES <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 0, duplicates: 1, skipped: 0 });
        let row = &db.list_qsos(10, 0)[0];
        assert!(row.lotw_rcvd);
        assert!(row.eqsl_sent);
        assert_eq!(row.state, "CA");
        assert_eq!(row.country, "UNITED STATES");
        // Flags the import didn't mention stay off.
        assert!(!row.qsl_sent && !row.lotw_sent && !row.eqsl_rcvd && !row.qrz_qsl);
    }

    #[test]
    fn import_adif_never_overwrites_an_already_logged_state() {
        let db = Db::open_in_memory().unwrap();
        let mut existing = rec("K1ABC", "20260604", "123045", false);
        existing.state = "TX".into();
        db.insert_qso(&existing).unwrap();
        // A re-import disagreeing on STATE must not clobber what's already logged.
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 \
            <STATE:2>CA <eor>";
        db.import_adif(adif);
        assert_eq!(db.list_qsos(10, 0)[0].state, "TX");
    }

    #[test]
    fn import_adif_maps_qrz_export_status_to_qrz_qsl() {
        let db = Db::open_in_memory().unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 \
            <APP_QRZLOG_STATUS:1>C <eor>";
        db.import_adif(adif);
        assert!(db.list_qsos(10, 0)[0].qrz_qsl);
    }

    #[test]
    fn merge_matched_qso_only_fills_blanks_and_only_turns_flags_on() {
        let existing = QsoRecord {
            call: "K1ABC".into(),
            confirmed: true,
            state: "TX".into(),
            ..Default::default()
        };
        // Imported disagrees on STATE (must be ignored, already logged) and
        // offers a new flag plus a COUNTRY the existing row doesn't have.
        let imported = QsoRecord {
            confirmed: false,
            lotw_rcvd: true,
            state: "CA".into(),
            country: "UNITED STATES".into(),
            ..Default::default()
        };
        let merged = merge_matched_qso(&existing, &imported).expect("something changed");
        assert!(merged.confirmed, "existing true flag must survive the import's false");
        assert!(merged.lotw_rcvd, "a new true flag from the import must be adopted");
        assert_eq!(merged.state, "TX", "blank-only fill must not overwrite an existing STATE");
        assert_eq!(merged.country, "UNITED STATES", "a blank COUNTRY is filled in");

        // Nothing new to contribute -> no-op.
        assert!(merge_matched_qso(&merged, &imported).is_none());
    }

    #[test]
    fn merge_matched_qso_upgrades_time_on_precision_but_never_downgrades_it() {
        // CQRLOG exports TIME_ON as HHMM; QRZ.com exports HHMMSS, the actual
        // time of the QSO. Whichever import created the row, the more precise
        // value must win.
        let coarse = QsoRecord { call: "K1ABC".into(), time_on: "1230".into(), ..Default::default() };
        let precise = QsoRecord { time_on: "123045".into(), ..Default::default() };

        // CQRLOG (coarse) logged first, QRZ (precise) imported after: upgrade.
        let merged = merge_matched_qso(&coarse, &precise).expect("more precise time must count as a change");
        assert_eq!(merged.time_on, "123045");

        // QRZ (precise) logged first, CQRLOG (coarse) imported after: no downgrade.
        assert!(
            merge_matched_qso(&precise, &coarse).is_none(),
            "a less precise TIME_ON must never replace a more precise one"
        );
    }

    #[test]
    fn merge_matched_qso_upgrades_generic_mode_to_the_specific_submode() {
        let generic = QsoRecord { call: "K1ABC".into(), mode: "MFSK".into(), ..Default::default() };
        let specific = QsoRecord { mode: "FT4".into(), ..Default::default() };

        // Generic (MFSK) logged first, specific (FT4) imported after: upgrade.
        let merged = merge_matched_qso(&generic, &specific).expect("a specific mode must count as a change");
        assert_eq!(merged.mode, "FT4");

        // Specific (FT4) logged first, generic (MFSK) imported after: no downgrade.
        assert!(
            merge_matched_qso(&specific, &generic).is_none(),
            "a generic family mode must never replace a specific submode"
        );

        // Two different specific submodes (not expected in practice, but the
        // rule is "only replace a generic label", not "always take the import's"):
        // neither side is generic, so nothing changes.
        let other_specific = QsoRecord { mode: "FT8".into(), ..Default::default() };
        assert!(merge_matched_qso(&specific, &other_specific).is_none());
    }

    #[test]
    fn minute_of_day_parses_hhmm_and_hhmmss_and_rejects_garbage() {
        assert_eq!(minute_of_day("0113"), Some(73));
        assert_eq!(minute_of_day("011345"), Some(73));
        assert_eq!(minute_of_day("113"), Some(73)); // dropped leading zero
        assert_eq!(minute_of_day(""), None);
        assert_eq!(minute_of_day("ab"), None);
        assert_eq!(minute_of_day("2500"), None); // hour out of range
        assert_eq!(minute_of_day("0060"), None); // minute out of range
    }

    #[test]
    fn is_lotw_style_record_requires_no_report_and_time_off_equal_to_time_on() {
        let lotw_style = QsoRecord {
            time_on: "0114".into(),
            time_off: "0114".into(),
            rst_rcvd: "".into(),
            ..Default::default()
        };
        assert!(is_lotw_style_record(&lotw_style));

        let has_report = QsoRecord { rst_rcvd: "-21".into(), ..lotw_style.clone() };
        assert!(!is_lotw_style_record(&has_report), "a real report means it's not a LoTW copy");

        let different_minute =
            QsoRecord { time_off: "0120".into(), ..lotw_style.clone() };
        assert!(!is_lotw_style_record(&different_minute));

        let no_time_off = QsoRecord { time_off: "".into(), ..lotw_style };
        assert!(!is_lotw_style_record(&no_time_off));
    }

    #[test]
    fn merge_matched_qso_lotw_twin_never_lets_the_copys_rst_sent_overwrite_a_stored_report() {
        // K3JOS 20190617: the stored row is the full QSO (-06/-21); the "import"
        // here is the LoTW-style copy, whose RST_SENT (-21) is really what we
        // received, not what we sent. The fill branch must not fire at all --
        // the existing/stored side isn't the LoTW-style one -- so nothing about
        // the reports can change. This is the exact regression fixed on
        // Android in d82697f8.
        let full = QsoRecord {
            call: "K3JOS".into(),
            time_on: "0113".into(),
            time_off: "0114".into(),
            rst_sent: "-06".into(),
            rst_rcvd: "-21".into(),
            ..Default::default()
        };
        let copy = QsoRecord {
            time_on: "0114".into(),
            time_off: "0114".into(),
            rst_sent: "-21".into(),
            rst_rcvd: "".into(),
            lotw_rcvd: true,
            ..Default::default()
        };
        let merged = merge_matched_qso(&full, &copy).expect("the LoTW flag is still a change");
        assert_eq!(merged.rst_sent, "-06", "the copy's RST_SENT must never overwrite the real report");
        assert_eq!(merged.rst_rcvd, "-21");
        assert!(merged.lotw_rcvd);
    }

    #[test]
    fn merge_matched_qso_lotw_twin_fills_a_stored_lotw_row_from_the_full_record() {
        // KC3OL 20201214: the copy (no FREQ) was stored first; the full record
        // (with FREQ) arrives later and must fill in the stored row's
        // times/reports/frequency/grid.
        let stored_lotw = QsoRecord {
            call: "KC3OL".into(),
            time_on: "2046".into(),
            time_off: "2046".into(),
            rst_sent: "599".into(),
            rst_rcvd: "".into(),
            freq: "".into(),
            gridsquare: "".into(),
            ..Default::default()
        };
        let full = QsoRecord {
            time_on: "2045".into(),
            time_off: "2046".into(),
            rst_sent: "O".into(),
            rst_rcvd: "O".into(),
            freq: "18.1".into(),
            gridsquare: "FN42".into(),
            ..Default::default()
        };
        let merged =
            merge_matched_qso(&stored_lotw, &full).expect("a full record must fill a stored LoTW row");
        assert_eq!(merged.time_on, "2045");
        assert_eq!(merged.time_off, "2046");
        assert_eq!(merged.rst_sent, "O");
        assert_eq!(merged.rst_rcvd, "O");
        assert_eq!(merged.freq, "18.1");
        assert_eq!(merged.gridsquare, "FN42");
    }

    #[test]
    fn merge_matched_qso_lotw_twin_keeps_a_non_blank_grid_and_a_blank_import_freq() {
        let stored_lotw = QsoRecord {
            time_on: "2046".into(),
            time_off: "2046".into(),
            rst_rcvd: "".into(),
            freq: "".into(),
            gridsquare: "FN42".into(), // already has a grid
            ..Default::default()
        };
        let full = QsoRecord {
            time_on: "2045".into(),
            time_off: "2046".into(),
            freq: "".into(), // no FREQ on this particular full record either
            gridsquare: "EM72".into(),
            ..Default::default()
        };
        let merged = merge_matched_qso(&stored_lotw, &full).expect("time_on still changes");
        assert_eq!(merged.freq, "", "an empty incoming FREQ must not overwrite anything");
        assert_eq!(merged.gridsquare, "FN42", "a non-blank grid is never overwritten");
    }

    #[test]
    fn import_adif_merges_a_lotw_copy_into_its_full_record_without_overwriting_reports() {
        // K3JOS 20190617, real example from the Android port: full 0113-0114
        // 40m FT8 7.07645 -06/-21, plus a LoTW-style copy 0114/0114 whose
        // RST_SENT (-21) must never overwrite the full record's real reports.
        // The exact-minute match doesn't fire (0113 != 0114 to the minute);
        // this exercises the 15-minute, same-band/mode twin match instead.
        let db = Db::open_in_memory().unwrap();
        let full = "<call:5>K3JOS <mode:3>FT8 <qso_date:8>20190617 <time_on:4>0113 \
            <time_off:4>0114 <band:3>40m <freq:7>7.07645 <rst_sent:3>-06 <rst_rcvd:3>-21 <eor>";
        let copy = "<call:5>K3JOS <mode:3>FT8 <qso_date:8>20190617 <time_on:4>0114 \
            <time_off:4>0114 <band:3>40m <rst_sent:3>-21 <LOTW_QSL_RCVD:1>Y <eor>";
        db.import_adif(full);
        let summary = db.import_adif(copy);
        assert_eq!(summary, ImportSummary { imported: 0, duplicates: 1, skipped: 0 });
        assert_eq!(db.count(), 1, "must merge into one row, not two");
        let row = &db.list_qsos(10, 0)[0];
        assert_eq!(row.rst_sent, "-06");
        assert_eq!(row.rst_rcvd, "-21");
        assert!(row.lotw_rcvd);
    }

    #[test]
    fn import_adif_fills_a_lotw_copy_imported_first_once_the_full_record_arrives() {
        // Same QSO as above, reversed import order: the LoTW-style copy
        // arrives first (nothing to match, so it's inserted as a real QSO),
        // then the full record comes in and must fill it rather than create
        // a second row.
        let db = Db::open_in_memory().unwrap();
        let copy = "<call:5>K3JOS <mode:3>FT8 <qso_date:8>20190617 <time_on:4>0114 \
            <time_off:4>0114 <band:3>40m <rst_sent:3>-21 <eor>";
        let full = "<call:5>K3JOS <mode:3>FT8 <qso_date:8>20190617 <time_on:4>0113 \
            <time_off:4>0114 <band:3>40m <freq:7>7.07645 <rst_sent:3>-06 <rst_rcvd:3>-21 <eor>";
        assert_eq!(db.import_adif(copy), ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.import_adif(full), ImportSummary { imported: 0, duplicates: 1, skipped: 0 });
        assert_eq!(db.count(), 1);
        let row = &db.list_qsos(10, 0)[0];
        assert_eq!(row.time_on, "0113");
        assert_eq!(row.rst_sent, "-06");
        assert_eq!(row.rst_rcvd, "-21");
        assert_eq!(row.freq, "7.07645");
    }

    #[test]
    fn import_adif_a_lotw_style_record_with_no_full_twin_is_inserted_as_a_real_qso() {
        // UT5MB 2024-06-07 0254 20M FT4: confirmed via LoTW/QRZ/eQSL with no
        // full record anywhere in the log -- a real QSO, not a copy of
        // anything, so it must be inserted rather than silently dropped.
        let db = Db::open_in_memory().unwrap();
        let adif = "<call:5>UT5MB <mode:3>FT4 <qso_date:8>20240607 <time_on:4>0254 \
            <time_off:4>0254 <band:3>20m <LOTW_QSL_RCVD:1>Y <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.count(), 1);
        assert!(db.list_qsos(10, 0)[0].lotw_rcvd);
    }

    #[test]
    fn import_adif_lotw_twin_requires_matching_band_and_mode() {
        // A LoTW-style record on a different band from its would-be twin must
        // not merge into it -- the twin match is deliberately stricter on
        // band/mode than the plain exact-minute match (which ignores both).
        let db = Db::open_in_memory().unwrap();
        let full = "<call:5>W3XOX <mode:3>FT8 <qso_date:8>20170728 <time_on:4>0100 \
            <time_off:4>0101 <band:3>20m <rst_sent:3>-06 <rst_rcvd:3>-21 <eor>";
        let copy = "<call:5>W3XOX <mode:3>FT8 <qso_date:8>20170728 <time_on:4>0102 \
            <time_off:4>0102 <band:3>40m <rst_sent:3>-21 <eor>";
        db.import_adif(full);
        let summary = db.import_adif(copy);
        assert_eq!(summary, ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.count(), 2, "different band -> not a twin, inserted separately");
    }

    #[test]
    fn import_adif_is_case_insensitive_on_call() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "123045", false)).unwrap();
        let adif = "<call:5>k1abc <mode:3>ft8 <qso_date:8>20260604 <time_on:6>123045 <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary.duplicates, 1);
        assert_eq!(db.count(), 1);
    }

    #[test]
    fn import_adif_matches_a_duplicate_even_when_mode_disagrees() {
        // Confirmed on a real ~54,500-QSO log (see `merge_matched_qso`'s doc
        // comment): CQRLOG's own ADIF round-trip widened every FT4/FT8 QSO to
        // the generic `MFSK` family with no SUBMODE to recover the real mode
        // from, while a QRZ.com re-export of the same log still had it.
        // Comparing MODE during import made ~7,100 QSOs fail to match their
        // own duplicate and get inserted a second time.
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "123045", false)).unwrap(); // MODE defaults to FT8
        let mut existing_mfsk = rec("W1AW", "20260604", "130000", false);
        existing_mfsk.mode = "MFSK".into();
        db.insert_qso(&existing_mfsk).unwrap();

        let adif = "<call:5>K1ABC <mode:4>MFSK <qso_date:8>20260604 <time_on:6>123045 <eor>\
            <call:4>W1AW <mode:3>FT4 <qso_date:8>20260604 <time_on:6>130000 <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 0, duplicates: 2, skipped: 0 });
        assert_eq!(db.count(), 2, "must not insert a second row for either QSO");
    }

    #[test]
    fn import_adif_a_different_minute_is_a_new_qso() {
        let db = Db::open_in_memory().unwrap();
        db.insert_qso(&rec("K1ABC", "20260604", "123045", false)).unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>124500 <eor>";
        let summary = db.import_adif(adif);
        assert_eq!(summary, ImportSummary { imported: 1, duplicates: 0, skipped: 0 });
        assert_eq!(db.count(), 2);
    }

    #[test]
    fn import_adif_can_be_run_twice_without_duplicating() {
        let db = Db::open_in_memory().unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>123045 <eor>\n\
            <call:4>W1AW <mode:3>FT8 <qso_date:8>20260604 <time_on:6>130000 <eor>";
        let first = db.import_adif(adif);
        assert_eq!(first, ImportSummary { imported: 2, duplicates: 0, skipped: 0 });
        let second = db.import_adif(adif);
        assert_eq!(second, ImportSummary { imported: 0, duplicates: 2, skipped: 0 });
        assert_eq!(db.count(), 2);
    }

    #[test]
    fn import_adif_with_progress_reports_processed_and_total() {
        let db = Db::open_in_memory().unwrap();
        // 3 records: below the every-200 batch interval, so progress must
        // still fire once at the very end (processed == total), not be
        // silently dropped for a small import.
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 <time_on:6>120000 <eor>\
            <call:4>W1AW <mode:3>FT8 <qso_date:8>20260604 <time_on:6>130000 <eor>\
            <call:5>N0CALL <mode:3>FT8 <qso_date:8>20260604 <time_on:6>140000 <eor>";
        let mut calls = Vec::new();
        let summary = db.import_adif_with_progress(adif, |processed, total| {
            calls.push((processed, total));
        });
        assert_eq!(summary.imported, 3);
        assert_eq!(calls, vec![(3, 3)]);
    }

    #[test]
    fn import_adif_with_progress_fires_every_200_records_on_a_large_import() {
        let db = Db::open_in_memory().unwrap();
        let mut adif = String::new();
        for i in 0..450 {
            adif.push_str(&format!(
                "<call:5>K0{i:03} <mode:3>FT8 <qso_date:8>20260604 <time_on:6>{:06} <eor>",
                120000 + i,
            ));
        }
        let mut calls = Vec::new();
        let summary = db.import_adif_with_progress(&adif, |processed, total| {
            calls.push((processed, total));
        });
        assert_eq!(summary.imported, 450);
        assert_eq!(calls, vec![(200, 450), (400, 450), (450, 450)]);
    }

    #[test]
    fn import_adif_with_progress_backfills_callsign_qth_from_gridsquare() {
        let db = Db::open_in_memory().unwrap();
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 \
                     <time_on:6>120000 <gridsquare:4>FN42 <eor>";
        db.import_adif_with_progress(adif, |_, _| {});
        assert_eq!(db.qth_for_call("K1ABC"), Some("FN42".to_string()));
    }

    #[test]
    fn import_adif_with_progress_never_overwrites_a_live_heard_grid() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_callsign_qth_live("K1ABC", "EN37");
        let adif = "<call:5>K1ABC <mode:3>FT8 <qso_date:8>20260604 \
                     <time_on:6>120000 <gridsquare:4>FN42 <eor>";
        db.import_adif_with_progress(adif, |_, _| {});
        assert_eq!(
            db.qth_for_call("K1ABC"),
            Some("EN37".to_string()),
            "ADIF backfill must never clobber a live-heard grid"
        );
    }

    #[test]
    fn migrates_v6_log_in_place_and_adds_callsign_qth() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE qso_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call TEXT NOT NULL, gridsquare TEXT, mode TEXT, rst_sent TEXT,
                rst_rcvd TEXT, qso_date TEXT, time_on TEXT, qso_date_off TEXT,
                time_off TEXT, band TEXT, freq TEXT, station_callsign TEXT,
                my_gridsquare TEXT, comment TEXT, created_at INTEGER,
                qsl_rcvd INTEGER NOT NULL DEFAULT 0,
                synced_cloudlog INTEGER NOT NULL DEFAULT 0,
                synced_qrz INTEGER NOT NULL DEFAULT 0,
                qsl_sent INTEGER NOT NULL DEFAULT 0,
                lotw_sent INTEGER NOT NULL DEFAULT 0,
                lotw_rcvd INTEGER NOT NULL DEFAULT 0,
                eqsl_sent INTEGER NOT NULL DEFAULT 0,
                eqsl_rcvd INTEGER NOT NULL DEFAULT 0,
                qrz_qsl INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT '',
                country TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO qso_log(call, mode, qso_date, time_on)
                VALUES('K1ABC', 'FT8', '20260604', '120000');
            PRAGMA user_version = 6;
            "#,
        )
        .unwrap();
        let db = Db::from_conn(conn).unwrap();
        assert_eq!(db.count(), 1, "existing rows must survive the migration");
        assert_eq!(db.callsign_qth_count(), 0);
        db.upsert_callsign_qth_live("K1ABC", "FN42");
        assert_eq!(db.callsign_qth_count(), 1);
    }

    #[test]
    fn upsert_callsign_qth_live_always_replaces() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_callsign_qth_live("K1ABC", "FN42");
        db.upsert_callsign_qth_live("K1ABC", "EN37");
        assert_eq!(db.qth_for_call("K1ABC"), Some("EN37".to_string()));
    }

    #[test]
    fn upsert_callsign_qth_live_ignores_a_too_short_grid() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_callsign_qth_live("K1ABC", "FN4");
        assert_eq!(db.qth_for_call("K1ABC"), None);
    }

    #[test]
    fn upsert_callsign_qth_live_normalizes_case_and_truncates_to_grid4() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_callsign_qth_live("k1abc", "fn42aa");
        assert_eq!(db.qth_for_call("K1ABC"), Some("FN42".to_string()));
    }

    #[test]
    fn seed_callsign_qth_batch_never_overwrites_a_live_heard_row() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_callsign_qth_live("K1ABC", "EN37");
        let seeded = db.seed_callsign_qth_batch(
            vec![("K1ABC".to_string(), "FN42".to_string())].into_iter(),
            1,
            |_, _, _| {},
        );
        assert_eq!(seeded, 0, "the FCC seed must not count a no-op as seeded");
        assert_eq!(db.qth_for_call("K1ABC"), Some("EN37".to_string()));
    }

    #[test]
    fn seed_callsign_qth_batch_fills_gaps_and_reports_progress() {
        let db = Db::open_in_memory().unwrap();
        let entries: Vec<(String, String)> =
            (0..1200).map(|i| (format!("K{i}"), "FN42".to_string())).collect();
        let mut ticks = Vec::new();
        let seeded = db.seed_callsign_qth_batch(entries.into_iter(), 1200, |p, t, call| {
            ticks.push((p, t, call.to_string()))
        });
        assert_eq!(seeded, 1200);
        assert_eq!(db.callsign_qth_count(), 1200);
        // Every 500 plus a final unconditional (total, total, "") tick, each
        // carrying the callsign just processed (for "Processing Callsign:
        // W1AW" style UI) -- K499/K999 are the 500th/1000th entries (0-indexed).
        assert_eq!(
            ticks,
            vec![
                (500, 1200, "K499".to_string()),
                (1000, 1200, "K999".to_string()),
                (1200, 1200, String::new()),
            ]
        );
    }

    #[test]
    fn custom_bands_persist_via_config() {
        // The custom dial list (issue #470) rides on the config store, so it must
        // survive a reload the same way any other config value does.
        use crate::bands;
        let db = Db::open_in_memory().unwrap();

        let mut custom =
            bands::parse_custom_bands(&db.get_config(bands::CUSTOM_BANDS_KEY).unwrap_or_default());
        assert!(custom.is_empty());

        bands::upsert_custom_band(&mut custom, "3B9 DX", 14_090_000);
        db.set_config(bands::CUSTOM_BANDS_KEY, &bands::serialize_custom_bands(&custom))
            .unwrap();

        // Reload from the store (as a fresh app launch would).
        let reloaded =
            bands::parse_custom_bands(&db.get_config(bands::CUSTOM_BANDS_KEY).unwrap());
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].dial_hz, 14_090_000);
        assert_eq!(reloaded[0].name, "3B9 DX");

        let merged = bands::merged_bands(&reloaded);
        assert!(merged.iter().any(|b| b.dial_hz == 14_090_000 && b.custom));
    }
}
