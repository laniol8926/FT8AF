//! DXCC entity + CQ zone lookup from a callsign, ported from the Android
//! app's `CallsignDatabase.java` / `CallsignInfo.java` /
//! `CallsignFileOperation.java` (data source: the AD1C "big" cty.dat country
//! file, `ft8af/app/src/main/assets/cty.dat`, embedded here as
//! `resources/cty.dat`). Backs the Decode list's New DXCC / New Zone
//! filters.
//!
//! cty.dat format: records separated by `;`, each
//! `Name:CQ:ITU:Continent:Lat:Long:GMT:DXCCPrefix:alias1,alias2,...`. Only
//! the country name, CQ zone, and the DXCC-identifying prefix (the entity
//! key Android's `dxccMap` is keyed by) are kept -- ITU zone, continent,
//! lat/long and GMT offset aren't used by anything ported here.
//!
//! Alias matching: longest-prefix match against the call (an alias is a
//! *prefix* of the call), with a `=FULLCALL` alias matching only that exact
//! call -- mirrors the SQL this replaces (`CallsignDatabase.getCallsignInfo`):
//! `WHERE SUBSTR(?,1,LENGTH(callsign))=callsign OR callsign='='||? ORDER BY
//! LENGTH(callsign) DESC LIMIT 1`. An exact `=` match is always longer than
//! any proper prefix of the same call (`call.len()+1` vs. at most
//! `call.len()`), so checking it first and otherwise taking the longest
//! prefix match gives the same result the SQL's `ORDER BY ... DESC LIMIT 1`
//! would.

use once_cell::sync::Lazy;
use std::collections::HashMap;

const CTY_DAT: &str = include_str!("../resources/cty.dat");

#[derive(Debug, Clone)]
struct DxccEntry {
    country_name: String,
    cq_zone: i64,
    dxcc_prefix: String,
}

/// A resolved callsign location: just the fields the New DXCC / New Zone
/// filters and their worked-set aggregation need.
#[derive(Debug, Clone, PartialEq)]
pub struct DxccMatch {
    pub country_name: String,
    pub cq_zone: i64,
    pub dxcc_prefix: String,
}

struct CtyTable {
    entries: Vec<DxccEntry>,
    /// Alias (including literal `=CALL` exact-match entries, '=' and all)
    /// to its entry's index in `entries`.
    aliases: HashMap<String, usize>,
}

static TABLE: Lazy<CtyTable> = Lazy::new(|| parse_cty(CTY_DAT));

fn parse_cty(data: &str) -> CtyTable {
    let mut entries = Vec::new();
    let mut aliases = HashMap::new();

    for record in data.split(';') {
        if !record.contains(':') {
            continue;
        }
        let fields: Vec<&str> = record.split(':').collect();
        if fields.len() < 9 {
            continue;
        }
        let country_name = fields[0].replace('\n', "").trim().to_string();
        let cq_zone: i64 = match fields[1].replace('\n', "").replace(' ', "").parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let dxcc_prefix = fields[7].replace('\n', "").replace(' ', "");
        let alias_field = fields[8].replace('\n', "").replace(' ', "");
        if country_name.is_empty() || dxcc_prefix.is_empty() {
            continue;
        }

        let idx = entries.len();
        entries.push(DxccEntry { country_name, cq_zone, dxcc_prefix });

        for raw in alias_field.split(',') {
            // Truncate at the first CQ-zone-override "(" or, failing that,
            // ITU-zone-override "[" -- mirrors getCallsigns() exactly,
            // including that it does NOT strip any other marker (e.g. a
            // "<lat/long>" override) unless one of these two precedes it.
            let alias = if let Some(i) = raw.find('(') {
                &raw[..i]
            } else if let Some(i) = raw.find('[') {
                &raw[..i]
            } else {
                raw
            }
            .trim();
            if alias.is_empty() {
                continue;
            }
            // First entry wins on a duplicate alias (shouldn't happen in a
            // well-formed cty.dat; this just keeps lookup deterministic if
            // it ever does).
            aliases.entry(alias.to_string()).or_insert(idx);
        }
    }

    CtyTable { entries, aliases }
}

/// Resolve a callsign to its DXCC entity + CQ zone, or `None` if nothing in
/// the table matches (an implausible/garbled callsign).
pub fn lookup(call: &str) -> Option<DxccMatch> {
    let call = call.trim().to_uppercase();
    if call.is_empty() {
        return None;
    }
    let table = &*TABLE;

    // Exact "=CALL" entries always win (see module doc) -- check first.
    let exact_key = format!("={call}");
    if let Some(&idx) = table.aliases.get(&exact_key) {
        return Some(to_match(&table.entries[idx]));
    }
    // Longest-prefix match: the call's own prefixes, longest first.
    let chars: Vec<char> = call.chars().collect();
    for len in (1..=chars.len()).rev() {
        let prefix: String = chars[..len].iter().collect();
        if let Some(&idx) = table.aliases.get(&prefix) {
            return Some(to_match(&table.entries[idx]));
        }
    }
    None
}

fn to_match(e: &DxccEntry) -> DxccMatch {
    DxccMatch {
        country_name: e.country_name.clone(),
        cq_zone: e.cq_zone,
        dxcc_prefix: e.dxcc_prefix.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_well_known_calls() {
        let m = lookup("W1AW").expect("W1AW should resolve");
        assert_eq!(m.dxcc_prefix, "K");
        assert_eq!(m.country_name, "United States");
        assert_eq!(m.cq_zone, 5);

        let m = lookup("ve3abc").expect("case-insensitive"); // lower-case input
        assert_eq!(m.dxcc_prefix, "VE");
        assert_eq!(m.country_name, "Canada");

        let m = lookup("G4XYZ").expect("G4XYZ should resolve");
        assert_eq!(m.dxcc_prefix, "G");
        assert_eq!(m.country_name, "England");
    }

    #[test]
    fn longest_prefix_wins_over_a_shorter_one() {
        // KH6 (Hawaii) is its own DXCC entity, distinct from the mainland "K".
        let m = lookup("KH6ABC").expect("KH6ABC should resolve");
        assert_eq!(m.dxcc_prefix, "KH6");
        assert_ne!(m.dxcc_prefix, "K");
    }

    #[test]
    fn non_callsign_or_empty_input_returns_none() {
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("   "), None);
    }

    #[test]
    fn parses_a_known_entry_count_order_of_magnitude() {
        // Sanity check on the embedded resource itself: cty.dat has on the
        // order of 400+ entities and tens of thousands of alias prefixes.
        // Guards against silently shipping a truncated/corrupt resource.
        let table = &*TABLE;
        assert!(table.entries.len() > 300, "entries: {}", table.entries.len());
        assert!(table.aliases.len() > 20_000, "aliases: {}", table.aliases.len());
    }
}
