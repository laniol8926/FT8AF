//! Which side of a stereo audio path to use, independently for receive and
//! transmit -- ported from the Android fork's
//! `com.k1af.ft8af.wave.AudioChannelSelect`. Real-hardware motivation (same
//! as there): a splitter cable feeding two radios, a dual-receiver rig
//! putting main/sub on L/R, or an interface that leaves an unused channel
//! floating and noisy. [`BOTH`] (RX: average every channel, TX: fan the same
//! sample to every channel) is the default and what every build before this
//! selector did in both directions.
//!
//! Config-table keys match Android's exactly (`rxAudioChannel`/
//! `txAudioChannel`, plain int strings `"0"`/`"1"`/`"2"`) so a future
//! settings-import can read either app's export without translation.

/// Use every channel -- RX averages them, TX sends the same sample to each.
pub const BOTH: i32 = 0;
/// Channel 0 only; every other channel is discarded (RX) or silenced (TX).
pub const LEFT: i32 = 1;
/// Channel 1 only; every other channel is discarded (RX) or silenced (TX).
/// No-op (same as [`BOTH`]) on a device with only one channel -- there is no
/// second channel to isolate.
pub const RIGHT: i32 = 2;

/// Clamp an arbitrary int to a valid selection, defaulting to [`BOTH`] --
/// mirrors `AudioChannelSelect.clamp`'s leniency (a config value surviving a
/// bad settings import must never panic here).
pub fn clamp(selection: i32) -> i32 {
    if selection == LEFT || selection == RIGHT {
        selection
    } else {
        BOTH
    }
}

/// Parse a persisted config string, defaulting to [`BOTH`] on anything
/// missing or unparseable -- mirrors `AudioChannelSelect.parse`.
pub fn parse(value: Option<&str>) -> i32 {
    value
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map(clamp)
        .unwrap_or(BOTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_passes_through_valid_selections() {
        assert_eq!(clamp(BOTH), BOTH);
        assert_eq!(clamp(LEFT), LEFT);
        assert_eq!(clamp(RIGHT), RIGHT);
    }

    #[test]
    fn clamp_defaults_invalid_values_to_both() {
        assert_eq!(clamp(-1), BOTH);
        assert_eq!(clamp(3), BOTH);
        assert_eq!(clamp(999), BOTH);
    }

    #[test]
    fn parse_reads_valid_strings() {
        assert_eq!(parse(Some("0")), BOTH);
        assert_eq!(parse(Some("1")), LEFT);
        assert_eq!(parse(Some("2")), RIGHT);
        assert_eq!(parse(Some(" 1 ")), LEFT); // tolerate incidental whitespace
    }

    #[test]
    fn parse_defaults_missing_or_garbage_to_both() {
        assert_eq!(parse(None), BOTH);
        assert_eq!(parse(Some("")), BOTH);
        assert_eq!(parse(Some("not a number")), BOTH);
        assert_eq!(parse(Some("7")), BOTH);
    }
}
