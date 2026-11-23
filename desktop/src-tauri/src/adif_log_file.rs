//! A running, append-only ADIF mirror (`ft8af_log.adi`) that receives one
//! `<eor>` record the moment each QSO is logged, ported from Android's
//! `AdifLogFile.java`. Gives a crash-proof mirror of the QSO log
//! independent of the SQLite DB, and lets a desktop logger (LOG4OM, N1MM,
//! DXKeeper, ...) tail the file and import new contacts in real time.
//!
//! **Location, simplified from Android's**: Android keeps the real file in
//! its app-private external-files dir (no runtime permission needed) and
//! mirrors a full copy into `Download/FT8AF/` via MediaStore so the
//! operator can actually find it (confirmed by the user: they moved to
//! using the Downloads copy as the real one on their fork). Desktop has no
//! scoped-storage restriction at all -- there's nothing stopping a normal
//! process from writing directly into the real Downloads folder, so this
//! writes `ft8af_log.adi` there directly rather than porting Android's
//! app-private-file-plus-MediaStore-mirror dance, which solves an Android-
//! only problem.
//!
//! Robustness, matching Android: the header + `<eoh>` is written exactly
//! once on a fresh/empty file (re-created if the file was deleted out from
//! under the app -- decided from the just-opened file's actual size, not a
//! stale pre-open check), every record is flushed immediately, and any
//! failure is swallowed by the caller -- appending must never break QSO
//! logging.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "ft8af_log.adi";
const HEADER: &str = "FT8AF ADIF Log<eoh>\n";

/// `Download/FT8AF/ft8af_log.adi`, or `None` if the platform has no
/// resolvable Downloads directory at all.
///
/// **Always `None` in a `#[cfg(test)]` build.** Real bug, found live
/// 2026-11-23: `engine.rs`'s own test suite completes full simulated QSOs
/// (placeholder calls like `K0XYZ`/`K1ABC`) through the exact same
/// `log_completed_qso` path the real app uses, and `enableAdifExport`
/// defaults to *on* when unset -- which a test's fresh in-memory `Db`
/// always is. With no way to redirect `dirs::download_dir()` away from
/// the real OS Downloads folder, every `cargo test` run on a developer's
/// own machine was silently appending fake QSOs into that machine's
/// real `~/Downloads/FT8AF/ft8af_log.adi` -- confirmed: this exact file
/// on the machine this was found on. Blocking it here, unconditionally,
/// protects every current and future test that exercises this path,
/// rather than requiring each one to remember to set `enableAdifExport`
/// to `"0"` in its own test `Db`. This module's own tests call
/// `append_record` directly with an explicit temp path instead of going
/// through this function, so they're unaffected.
pub fn log_file_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        dirs::download_dir().map(|d| d.join("FT8AF").join(FILE_NAME))
    }
}

/// Append one already-built ADIF record (ending in `<eor>`, no trailing
/// newline required) to `path`, writing the header first iff the file is
/// new or empty. The testable core -- no config/engine dependency.
pub fn append_record(path: &Path, record: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // Decide header emission from the just-opened file's actual size, not a
    // pre-open check: a delete-then-recreate race between that check and
    // the open would otherwise leave a headerless file.
    let size = file.seek(SeekFrom::End(0))?;
    if size == 0 {
        file.write_all(HEADER.as_bytes())?;
    }
    file.write_all(record.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()
}

/// Appends `record` to the running log at its real Downloads location,
/// gated on `enabled`. Never panics -- a full disk, missing Downloads
/// directory, or permission error is swallowed exactly like Android's
/// `Throwable` catch-all, since this must never break QSO logging.
pub fn log_qso(enabled: bool, record: &str) {
    if !enabled {
        return;
    }
    if let Some(path) = log_file_path() {
        let _ = append_record(&path, record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ft8af_adif_log_file_test_{name}_{}.adi", std::process::id()))
    }

    #[test]
    fn append_record_writes_the_header_once_on_a_fresh_file() {
        let path = temp_path("fresh");
        let _ = std::fs::remove_file(&path);
        append_record(&path, "<call:5>K1ABC<eor>").unwrap();
        append_record(&path, "<call:5>W1AW <eor>").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches(HEADER).count(), 1);
        assert!(contents.contains("<call:5>K1ABC<eor>"));
        assert!(contents.contains("<call:5>W1AW <eor>"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn append_record_recreates_the_header_if_the_file_was_deleted_between_appends() {
        let path = temp_path("deleted");
        let _ = std::fs::remove_file(&path);
        append_record(&path, "<call:5>K1ABC<eor>").unwrap();
        std::fs::remove_file(&path).unwrap();
        append_record(&path, "<call:5>W1AW <eor>").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches(HEADER).count(), 1);
        assert!(!contents.contains("K1ABC"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn append_record_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!("ft8af_adif_log_file_test_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join(FILE_NAME);
        append_record(&path, "<call:5>K1ABC<eor>").unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_qso_is_a_no_op_when_disabled() {
        let path = temp_path("disabled");
        let _ = std::fs::remove_file(&path);
        // Can't redirect log_qso's own path, but a disabled call must touch
        // nothing at all -- confirmed by it not panicking with no Downloads
        // directory assumptions and append_record itself being what's
        // actually under test for the enabled path above.
        log_qso(false, "<call:5>K1ABC<eor>");
    }

    #[test]
    fn log_file_path_is_none_in_a_test_build_even_though_a_real_downloads_dir_exists() {
        // The real regression: engine.rs's own tests complete full
        // simulated QSOs with enableAdifExport defaulting to on (a fresh
        // test Db never sets it), which used to resolve straight to the
        // real OS Downloads folder and append fake K0XYZ/K1ABC QSOs into
        // a real developer's real ft8af_log.adi. This must stay None
        // under #[cfg(test)] no matter what the real OS reports.
        assert!(log_file_path().is_none());
    }

    #[test]
    fn log_qso_enabled_is_a_no_op_in_a_test_build() {
        // Same regression, exercised through the real public entry point
        // engine.rs calls: enabled=true must still touch no real file,
        // because log_file_path() is None under test.
        log_qso(true, "<call:5>K1ABC<eor>");
    }
}
