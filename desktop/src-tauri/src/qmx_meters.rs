//! Parses the QRP Labs QMX's SWR/power-meter CAT replies, ported from the
//! Android app's `QmxMeterReplies.java` (`rigs/QmxMeterReplies.java` on
//! `dev`). Firmware 1.04 CAT manual: `SW;` queries SWR, reporting the ratio
//! times 100 (`SW130;` = 1.30:1); `PC;` queries output power in tenths of a
//! watt (`PC45;` = 4.5 W). Both end in `;`, like every QMX CAT reply.
//!
//! Pure: formatting/parsing only, so it's unit-tested without a real rig --
//! see [`crate::rig::Hamlib::send_raw_cat_query`] for the actual CAT
//! round-trip this feeds.

/// The SWR meter query.
pub const SWR_QUERY: &str = "SW;";
/// The output power meter query.
pub const POWER_QUERY: &str = "PC;";
/// Reply terminator passed to `rig_send_raw` (a null-terminated C string of
/// acceptable terminator bytes, per Hamlib's convention).
pub const REPLY_TERM: &[u8] = b";\0";

/// SWR ratio from a reply like `"SW130;"` (130 = 1.30:1), or `None` if it
/// isn't a valid reading.
pub fn parse_swr(reply: &str) -> Option<f32> {
    parse_scaled(reply, "SW", 100.0)
}

/// Output power in watts from a reply like `"PC45;"` (tenths of a watt: 4.5 W),
/// or `None` if it isn't a valid reading.
pub fn parse_power_watts(reply: &str) -> Option<f32> {
    parse_scaled(reply, "PC", 10.0)
}

fn parse_scaled(reply: &str, prefix: &str, divisor: f32) -> Option<f32> {
    let reply = reply.trim();
    let body = reply.strip_prefix(prefix)?.strip_suffix(';')?.trim();
    if body.is_empty() {
        return None;
    }
    if body.contains('.') {
        body.parse::<f32>().ok()
    } else {
        body.parse::<i32>().ok().map(|v| v as f32 / divisor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_swr_as_hundredths() {
        assert_eq!(parse_swr("SW130;"), Some(1.30));
        assert_eq!(parse_swr("SW100;"), Some(1.00));
    }

    #[test]
    fn parses_power_as_tenths() {
        assert_eq!(parse_power_watts("PC45;"), Some(4.5));
        assert_eq!(parse_power_watts("PC5;"), Some(0.5));
    }

    #[test]
    fn accepts_a_literal_decimal_body() {
        assert_eq!(parse_swr("SW1.3;"), Some(1.3));
    }

    #[test]
    fn rejects_non_meter_or_malformed_replies() {
        assert_eq!(parse_swr("FA14074000;"), None);
        assert_eq!(parse_power_watts(""), None);
        assert_eq!(parse_swr("SW;"), None);
        assert_eq!(parse_power_watts("PC"), None); // missing terminator
    }
}
