//! FT8AF desktop core library.
//!
//! For Milestone 1 this exposes only the DSP layer (FFI to the C ft8_lib core).
//! Audio, scheduling, rig control, persistence and the Tauri shell are added in
//! later milestones.

pub mod adif;
pub mod adif_log_file;
pub mod audio;
pub mod bands;
pub mod callsign;
// Build-time arch-selection helpers, shared with `build.rs` via `include!`.
// Unused by the library itself (only build.rs calls them), so silence dead_code;
// they exist here purely so `cargo test` runs their unit tests.
#[allow(dead_code)]
mod build_support;
pub mod clublog;
pub mod cloudlog;
pub mod db;
pub mod dsp;
pub mod dxcc;
pub mod engine;
pub mod eqsl;
pub mod fcc_grid;
pub mod hamqth;
pub mod hrdlog;
pub mod logged_state;
pub mod lotw;
pub mod map_tiles;
pub mod mode;
pub mod os_location;
pub mod psk_reporter;
pub mod pskreporter_sender;
pub mod qmx_cat_tones;
pub mod qmx_meters;
pub mod qrz;
pub mod qso;
pub mod rig;
pub mod settings_backup;
pub mod timesync;
pub mod tx_protection;
pub mod udp;
pub mod us_states;
pub mod util;
pub mod wf;
pub mod wpx_prefix;
pub mod wrl;
