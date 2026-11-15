//! Audio device enumeration via cpal (WASAPI / CoreAudio), and via a direct
//! ALSA PCM-name listing on Linux (see the `linux_alsa` module below for why
//! cpal's own enumeration isn't used there for *listing* purposes).

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    /// Stable-ish identifier (the device name; cpal has no opaque id).
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub default_sample_rate: u32,
    pub channels: u16,
}

fn collect(
    devices: impl Iterator<Item = cpal::Device>,
    default_name: Option<String>,
    input: bool,
) -> Vec<AudioDevice> {
    let mut out = Vec::new();
    for d in devices {
        let name = match d.name() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let cfg = if input {
            d.default_input_config().ok()
        } else {
            d.default_output_config().ok()
        };
        let (rate, channels) = match cfg {
            Some(c) => (c.sample_rate().0, c.channels()),
            None => (0, 0),
        };
        out.push(AudioDevice {
            is_default: default_name.as_deref() == Some(name.as_str()),
            id: name.clone(),
            name,
            default_sample_rate: rate,
            channels,
        });
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn list_input_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    match host.input_devices() {
        Ok(devs) => collect(devs, default, true),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn list_output_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());
    match host.output_devices() {
        Ok(devs) => collect(devs, default, false),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "linux")]
pub fn list_input_devices() -> Vec<AudioDevice> {
    linux_alsa::list_devices(true)
}

#[cfg(target_os = "linux")]
pub fn list_output_devices() -> Vec<AudioDevice> {
    linux_alsa::list_devices(false)
}

/// Direct ALSA PCM-name enumeration, bypassing cpal's own `Devices`
/// iterator for *listing* purposes only (actually opening a device for
/// capture/playback still goes through cpal as before -- see
/// `find_input_device`/`find_output_device`).
///
/// cpal's ALSA host enumerates by calling `snd_device_name_hint` and then
/// trying to *open* (`snd_pcm_open`, nonblocking) each hinted name, silently
/// dropping any name that fails to open (see cpal's `host::alsa::enumerate::
/// Devices::next`). A USB audio interface's direct hardware PCM (no `dmix`/
/// `dsnoop` sharing layer) only accepts one open handle per direction, so
/// the moment this app's own capture is actively running -- even under a
/// *different* alias like "default"/"pulse" routed to the very same
/// hardware -- cpal's probe-open on `plughw:CARD=...`/`sysdefault:CARD=...`
/// fails and the device vanishes from the list. Confirmed live on a QRP
/// Labs QMX: `arecord -L`/`aplay -L` (which only call `snd_device_name_hint`,
/// never open anything) always show it; cpal's `host.input_devices()` only
/// shows it when nothing is currently capturing from it. Reported by the
/// user as "the QMX doesn't appear in Audio Input" -- real root cause,
/// distinct from (and found after) the missing-friendly-label bug in
/// `audioDeviceLabel()`.
///
/// This module lists every hinted PCM name whose `IOID` matches the
/// requested direction (or has none, meaning both), with no open-probe at
/// all -- matching `arecord -L`'s always-show behavior. Rate/channels are
/// filled in on a best-effort basis from whatever cpal's own (busy-
/// filtered) enumeration happens to successfully open right now; a name
/// cpal couldn't currently open just shows as "--" rather than being
/// excluded.
#[cfg(target_os = "linux")]
mod linux_alsa {
    use super::AudioDevice;
    use cpal::traits::{DeviceTrait, HostTrait};
    use std::collections::HashMap;

    /// Whether a hinted PCM name belongs in the listing for `want`'s
    /// direction -- pure, so it's unit-tested without a live ALSA hint
    /// iterator. `None` (no `IOID` on the hint) means the name works both
    /// ways, matching how `arecord -L`/`aplay -L` both show it.
    pub(super) fn hint_matches_direction(hint_direction: Option<alsa::Direction>, want: alsa::Direction) -> bool {
        match hint_direction {
            None => true,
            Some(d) => d == want,
        }
    }

    /// `"null"` is ALSA's "discard/generate zero samples" plugin, not a real
    /// device -- matches cpal's own enumeration skipping it explicitly.
    pub(super) fn should_skip_hint_name(name: &str) -> bool {
        name == "null"
    }

    /// Whether a `CARD=`-scoped ALSA PCM alias is worth surfacing at all --
    /// first-pass filter, dropping multichannel/software-mixing variants no
    /// plain stereo USB interface ever uses. One physical card can register
    /// a dozen+ aliases for the exact same hardware -- confirmed live: the
    /// QMX alone has 13 output hints (`hw:`/`plughw:`/`sysdefault:`/
    /// `front:`/six `surroundNN:` variants/`iec958:`/`dmix:`/`usbstream:`)
    /// and 5 input ones. `front`/`surroundNN`/`iec958` are multichannel-
    /// output variants, `dmix`/`dsnoop` are ALSA's own software-mixing
    /// wrapper (what `sysdefault` already builds on for a consumer device),
    /// and `usbstream` is a raw-isochronous passthrough no normal app
    /// wants. Names with no `CARD=` at all (`default`, `pulse`, …) are
    /// untouched -- this filter only applies to a specific card's own
    /// aliases. See [`collapse_card_aliases`] for the second pass that
    /// collapses the three survivors (`hw:`/`plughw:`/`sysdefault:`) for
    /// one card down to a single entry.
    pub(super) fn is_primary_card_alias(name: &str) -> bool {
        if !name.contains("CARD=") {
            return true;
        }
        name.starts_with("hw:") || name.starts_with("plughw:") || name.starts_with("sysdefault:")
    }

    /// The `CARD=X` token from a `CARD=`-scoped alias (e.g.
    /// `"plughw:CARD=Transceiver,DEV=0"` -> `Some("Transceiver")`), or
    /// `None` for a generic, non-card-specific name.
    fn card_token(name: &str) -> Option<&str> {
        let after = name.split("CARD=").nth(1)?;
        Some(after.split(',').next().unwrap_or(after))
    }

    /// Preference among a card's surviving aliases (lower wins): `plughw:`
    /// first (auto rate/format conversion -- the safe general-purpose
    /// choice), then `sysdefault:`, then bare `hw:` (exact native format
    /// only, can fail outright if the app requests a mismatched rate).
    fn alias_rank(name: &str) -> u8 {
        if name.starts_with("plughw:") {
            0
        } else if name.starts_with("sysdefault:") {
            1
        } else {
            2
        }
    }

    /// Second pass: collapse multiple ALSA aliases for the *same* physical
    /// card down to the single best one. Confirmed directly against this
    /// app's own hardware: `aplay -l`/`arecord -l` (the hardware-centric
    /// listing, as opposed to `-L`'s PCM-*name* listing this module
    /// otherwise mirrors) show exactly **one** line for the QMX
    /// ("card 1: Transceiver"/"QMX Transceiver") -- after
    /// `is_primary_card_alias` cut the dozen-plus hints down to three
    /// (`hw:`/`plughw:`/`sysdefault:`), the picker still showed all three
    /// labeled "QRP Labs QMX Transceiver", reported live as "many QMX
    /// transceiver options". Matching the one-line-per-card expectation
    /// here instead of ALSA's own plugin-tier naming, which is an
    /// implementation detail no operator needs to choose between. Names
    /// with no `CARD=` token pass through unchanged, one each; relative
    /// order of the surviving names is preserved.
    pub(super) fn collapse_card_aliases(names: &[String]) -> Vec<String> {
        let mut best: HashMap<&str, &String> = HashMap::new();
        for name in names {
            if let Some(card) = card_token(name) {
                let better = match best.get(card) {
                    Some(existing) => alias_rank(name) < alias_rank(existing),
                    None => true,
                };
                if better {
                    best.insert(card, name);
                }
            }
        }
        names
            .iter()
            .filter(|name| match card_token(name) {
                Some(card) => best.get(card) == Some(name),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn list_devices(input: bool) -> Vec<AudioDevice> {
        let want = if input { alsa::Direction::Capture } else { alsa::Direction::Playback };
        let hints = match alsa::device_name::HintIter::new_str(None, "pcm") {
            Ok(h) => h,
            Err(_) => return cpal_fallback(input),
        };
        let rates = cpal_rate_channel_map(input);
        let default_name = cpal_default_name(input);

        let mut candidates = Vec::new();
        for hint in hints {
            let Some(name) = hint.name else { continue };
            if should_skip_hint_name(&name) {
                continue;
            }
            if !is_primary_card_alias(&name) {
                continue;
            }
            if !hint_matches_direction(hint.direction, want) {
                continue;
            }
            candidates.push(name);
        }

        collapse_card_aliases(&candidates)
            .into_iter()
            .map(|name| {
                let (rate, channels) = rates.get(&name).copied().unwrap_or((0, 0));
                AudioDevice {
                    is_default: default_name.as_deref() == Some(name.as_str()),
                    id: name.clone(),
                    name,
                    default_sample_rate: rate,
                    channels,
                }
            })
            .collect()
    }

    fn cpal_default_name(input: bool) -> Option<String> {
        let host = cpal::default_host();
        if input { host.default_input_device() } else { host.default_output_device() }
            .and_then(|d| d.name().ok())
    }

    /// Best-effort (rate, channels) per device name, from whichever names
    /// cpal's own enumeration can presently open -- never used to decide
    /// whether a name is *listed*, only to decorate it when possible.
    fn cpal_rate_channel_map(input: bool) -> HashMap<String, (u32, u16)> {
        let host = cpal::default_host();
        let mut map = HashMap::new();
        let devices: Vec<cpal::Device> = (if input { host.input_devices() } else { host.output_devices() })
            .map(|d| d.collect())
            .unwrap_or_default();
        for d in devices {
            let Ok(name) = d.name() else { continue };
            let cfg = if input { d.default_input_config().ok() } else { d.default_output_config().ok() };
            if let Some(c) = cfg {
                map.insert(name, (c.sample_rate().0, c.channels()));
            }
        }
        map
    }

    /// Only reached if `snd_device_name_hint` itself fails (not observed in
    /// practice) -- falls back to cpal's own enumeration rather than an
    /// empty list.
    fn cpal_fallback(input: bool) -> Vec<AudioDevice> {
        let host = cpal::default_host();
        let default = (if input { host.default_input_device() } else { host.default_output_device() })
            .and_then(|d| d.name().ok());
        let devices = if input { host.input_devices() } else { host.output_devices() };
        match devices {
            Ok(devs) => super::collect(devs, default, input),
            Err(_) => Vec::new(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hint_with_no_ioid_matches_either_direction() {
            assert!(hint_matches_direction(None, alsa::Direction::Capture));
            assert!(hint_matches_direction(None, alsa::Direction::Playback));
        }

        #[test]
        fn hint_with_an_ioid_only_matches_its_own_direction() {
            assert!(hint_matches_direction(Some(alsa::Direction::Capture), alsa::Direction::Capture));
            assert!(!hint_matches_direction(Some(alsa::Direction::Capture), alsa::Direction::Playback));
            assert!(hint_matches_direction(Some(alsa::Direction::Playback), alsa::Direction::Playback));
            assert!(!hint_matches_direction(Some(alsa::Direction::Playback), alsa::Direction::Capture));
        }

        #[test]
        fn primary_card_aliases_are_kept() {
            assert!(is_primary_card_alias("hw:CARD=Transceiver,DEV=0"));
            assert!(is_primary_card_alias("plughw:CARD=Transceiver,DEV=0"));
            assert!(is_primary_card_alias("sysdefault:CARD=Transceiver"));
        }

        #[test]
        fn secondary_card_aliases_are_filtered_out() {
            for name in [
                "front:CARD=Transceiver,DEV=0",
                "surround21:CARD=Transceiver,DEV=0",
                "surround40:CARD=Transceiver,DEV=0",
                "surround41:CARD=Transceiver,DEV=0",
                "surround50:CARD=Transceiver,DEV=0",
                "surround51:CARD=Transceiver,DEV=0",
                "surround71:CARD=Transceiver,DEV=0",
                "iec958:CARD=Transceiver,DEV=0",
                "dmix:CARD=Transceiver,DEV=0",
                "dsnoop:CARD=Transceiver,DEV=0",
                "usbstream:CARD=Transceiver",
            ] {
                assert!(!is_primary_card_alias(name), "{name} should have been filtered out");
            }
        }

        #[test]
        fn names_with_no_card_are_never_filtered_by_this_rule() {
            assert!(is_primary_card_alias("default"));
            assert!(is_primary_card_alias("pulse"));
            assert!(is_primary_card_alias("pipewire"));
        }

        fn s(names: &[&str]) -> Vec<String> {
            names.iter().map(|n| n.to_string()).collect()
        }

        #[test]
        fn collapse_keeps_plughw_over_sysdefault_and_hw_for_the_same_card() {
            let candidates = s(&[
                "hw:CARD=Transceiver,DEV=0",
                "plughw:CARD=Transceiver,DEV=0",
                "sysdefault:CARD=Transceiver",
            ]);
            assert_eq!(collapse_card_aliases(&candidates), vec!["plughw:CARD=Transceiver,DEV=0".to_string()]);
        }

        #[test]
        fn collapse_falls_back_to_sysdefault_then_hw_when_plughw_is_absent() {
            let sysdefault_and_hw = s(&["hw:CARD=Transceiver,DEV=0", "sysdefault:CARD=Transceiver"]);
            assert_eq!(collapse_card_aliases(&sysdefault_and_hw), vec!["sysdefault:CARD=Transceiver".to_string()]);

            let hw_only = s(&["hw:CARD=Transceiver,DEV=0"]);
            assert_eq!(collapse_card_aliases(&hw_only), vec!["hw:CARD=Transceiver,DEV=0".to_string()]);
        }

        #[test]
        fn collapse_keeps_each_card_independently() {
            let candidates = s(&[
                "hw:CARD=audiocodec,DEV=0",
                "plughw:CARD=audiocodec,DEV=0",
                "sysdefault:CARD=audiocodec",
                "plughw:CARD=Transceiver,DEV=0",
                "sysdefault:CARD=Transceiver",
            ]);
            let mut result = collapse_card_aliases(&candidates);
            result.sort();
            let mut expected = s(&["plughw:CARD=audiocodec,DEV=0", "plughw:CARD=Transceiver,DEV=0"]);
            expected.sort();
            assert_eq!(result, expected);
        }

        #[test]
        fn collapse_passes_through_non_card_names_unchanged() {
            let candidates = s(&["default", "pulse", "plughw:CARD=Transceiver,DEV=0"]);
            assert_eq!(
                collapse_card_aliases(&candidates),
                s(&["default", "pulse", "plughw:CARD=Transceiver,DEV=0"])
            );
        }

        #[test]
        fn null_device_is_skipped_like_cpal_skips_it() {
            assert!(should_skip_hint_name("null"));
            assert!(!should_skip_hint_name("default"));
            assert!(!should_skip_hint_name("plughw:CARD=Transceiver,DEV=0"));
        }

        /// Live integration check (runs on whatever ALSA sees in CI/dev
        /// sandboxes too -- even a machine with zero sound cards still has
        /// the always-present virtual plugins like "default"/"null"/"pulse").
        /// Asserts the *always-show* property this module exists for: a
        /// name present in `arecord -L`/`aplay -L`-equivalent output is
        /// never silently dropped just because cpal couldn't open it right
        /// now -- this test can't simulate "busy" without real hardware, so
        /// it only proves the hint pass-through itself works end to end.
        #[test]
        fn list_devices_never_panics_and_never_includes_null() {
            for input in [true, false] {
                let devices = list_devices(input);
                assert!(!devices.iter().any(|d| d.name == "null"));
            }
        }
    }
}

/// Find an input device by name, or the system default if `name` is None/unmatched.
pub fn find_input_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(want) = name {
        if let Ok(devs) = host.input_devices() {
            for d in devs {
                if d.name().ok().as_deref() == Some(want) {
                    return Some(d);
                }
            }
        }
    }
    host.default_input_device()
}

/// Find an output device by name, or the system default if `name` is None/unmatched.
pub fn find_output_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(want) = name {
        if let Ok(devs) = host.output_devices() {
            for d in devs {
                if d.name().ok().as_deref() == Some(want) {
                    return Some(d);
                }
            }
        }
    }
    host.default_output_device()
}
