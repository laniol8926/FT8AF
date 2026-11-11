//! Offline satellite tile basemap for the Map screen, backed by the same
//! bundled sqlite tile archive Android's GridTracker mode uses
//! (`resources/nightUSGS4Layer.sqlite`, osmdroid's own "file-based archive"
//! format: a single `tiles(key INTEGER PRIMARY KEY, provider TEXT, tile BLOB)`
//! table, one 256x256 baseline-JPEG blob per key, provider always
//! `"USGS National Map Satellite"`).
//!
//! The key formula and the fact that this is a plain **equirectangular**
//! (not Web Mercator) tile set were both empirically verified against the
//! real file before writing this: extracted and visually inspected tiles at
//! z=2 confirm `key = z*4^z + x*2^z + y` (x increases eastward from lon
//! -180, y increases *southward* from lat +90), and the z=0 tile shows an
//! undistorted world (normal-sized Greenland, a uniform Antarctic strip) --
//! Web Mercator would stretch high latitudes dramatically and cut off near
//! the poles. Only zoom levels 0-4 are stored (341 rows, full world
//! coverage at each level); deeper zoom just upsamples the z4 tile, matching
//! Android's own `setMaxZoomLevel(6.0)` over the same z0-4 archive.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};

/// The archive, embedded at compile time so no separate file needs shipping
/// alongside the binary (this project's release workflow is a plain
/// `--no-bundle` binary, not an installer -- see `docs/desktop-decode-
/// android-parity.md`'s "Build note"). Written out once to disk on first use
/// since `rusqlite` needs a real file path (there is no safe way to open a
/// `Connection` directly from an in-memory byte buffer without the
/// `serialize` feature, which would need unsafe FFI buffer-copy code to
/// populate from `include_bytes!` anyway).
const BUNDLED: &[u8] = include_bytes!("../resources/nightUSGS4Layer.sqlite");

/// Highest zoom level actually stored in the archive. A caller asking for
/// anything deeper should clamp to this and let the tile upsample visually.
pub const MAX_ZOOM: u32 = 4;

pub struct MapTiles {
    conn: Mutex<Connection>,
}

impl MapTiles {
    /// Writes the bundled archive to `app_data_dir` if not already present,
    /// then opens a read-only connection to that file. The archive is a
    /// static reference dataset nothing ever writes to, so read-only access
    /// and no migration/versioning logic are both deliberate.
    pub fn open(app_data_dir: &Path) -> anyhow::Result<MapTiles> {
        let path = app_data_dir.join("nightUSGS4Layer.sqlite");
        if !path.exists() {
            std::fs::write(&path, BUNDLED)?;
        }
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(MapTiles { conn: Mutex::new(conn) })
    }

    /// The raw JPEG bytes for tile `(z, x, y)`, or `None` if `z` is beyond
    /// `MAX_ZOOM`, `x`/`y` are out of range for `z`'s `2^z x 2^z` grid, or
    /// (defensively) the archive simply doesn't have that key.
    pub fn tile_for(&self, z: u32, x: u32, y: u32) -> Option<Vec<u8>> {
        if z > MAX_ZOOM {
            return None;
        }
        let n = 1u32 << z;
        if x >= n || y >= n {
            return None;
        }
        let key = (z as i64) * 4i64.pow(z) + (x as i64) * 2i64.pow(z) + y as i64;
        self.conn
            .lock()
            .query_row("SELECT tile FROM tiles WHERE key = ?1", [key], |r| r.get(0))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the bundled archive straight from its source path rather than
    /// through `MapTiles::open`'s write-to-app-data-dir step, so tests stay
    /// hermetic (no dependency on a real app data directory existing).
    fn open_bundled_for_test() -> MapTiles {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/nightUSGS4Layer.sqlite");
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open bundled tile archive");
        MapTiles { conn: Mutex::new(conn) }
    }

    /// JPEG files always start with the SOI marker 0xFFD8.
    fn looks_like_jpeg(bytes: &[u8]) -> bool {
        bytes.len() > 1000 && bytes[0] == 0xFF && bytes[1] == 0xD8
    }

    #[test]
    fn key_formula_matches_the_real_archive_at_known_coordinates() {
        // Empirically verified this session by extracting and viewing these
        // exact tiles: (0,0) is the Canadian Arctic archipelago, (0,1) is
        // North America (confirms y+1 = south), (1,0) is Greenland (confirms
        // x+1 = east), and (3,3) is Antarctica (the far SE corner at z=2).
        let tiles = open_bundled_for_test();
        let t00 = tiles.tile_for(2, 0, 0).expect("(z=2,x=0,y=0) should exist");
        let t01 = tiles.tile_for(2, 0, 1).expect("(z=2,x=0,y=1) should exist");
        let t10 = tiles.tile_for(2, 1, 0).expect("(z=2,x=1,y=0) should exist");
        let t33 = tiles.tile_for(2, 3, 3).expect("(z=2,x=3,y=3) should exist");
        for (name, bytes) in [("00", &t00), ("01", &t01), ("10", &t10), ("33", &t33)] {
            assert!(looks_like_jpeg(bytes), "tile {name} doesn't look like a JPEG");
        }
        // Distinct tiles, not accidentally all resolving to the same row.
        assert_ne!(t00, t01);
        assert_ne!(t00, t10);
        assert_ne!(t00, t33);
    }

    #[test]
    fn the_whole_world_is_a_single_tile_at_zoom_zero() {
        let tiles = open_bundled_for_test();
        let t = tiles.tile_for(0, 0, 0).expect("the single z=0 tile should exist");
        assert!(looks_like_jpeg(&t));
    }

    #[test]
    fn out_of_range_zoom_or_coordinates_return_none() {
        let tiles = open_bundled_for_test();
        assert_eq!(tiles.tile_for(5, 0, 0), None); // beyond MAX_ZOOM
        assert_eq!(tiles.tile_for(2, 4, 0), None); // x out of range for a 4x4 grid
        assert_eq!(tiles.tile_for(2, 0, 4), None); // y out of range for a 4x4 grid
    }

    #[test]
    fn the_archive_has_exactly_the_documented_row_count() {
        // 1 + 4 + 16 + 64 + 256 = 341: full coverage at each of z=0..4, no
        // gaps, nothing beyond. Guards against silently shipping a
        // truncated/corrupt resource (mirrors dxcc.rs's own resource
        // sanity-check test style).
        let tiles = open_bundled_for_test();
        let count: i64 = tiles
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM tiles", [], |r| r.get(0))
            .expect("count tiles");
        assert_eq!(count, 341);
    }
}
