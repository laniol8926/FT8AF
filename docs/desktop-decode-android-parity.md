# Desktop Decode screen: Android UI parity (in progress)

Tracks `feat/decode-waterfall-android-design`'s push to bring the desktop
Decode screen in line with the Android app's layout and data (chips, meters,
columns), beyond the Hunt mode / QMX meters / blank-webview work already on
this branch. Not pushed or PR'd yet — still changing.

## Done this round

- **Call CQ / MORE button**: widened (`min-width` + right padding on
  `.pill-cq`) so the MORE badge no longer overlaps the "CQ" text.
- **Persistent bottom message bar removed**: `TxBar` (TX stage picker +
  Stop TX) is no longer always visible. It's now behind a "More" toggle
  button appended to the tab bar, matching Android's `TabBar.kt` "More
  controls" entry that opens the operate-controls drawer (`moreOpen` state
  in `App()`, toggled by the new tab-bar button; no drag gesture like
  Android's drawer, just a click-to-toggle panel — desktop has no touch
  surface to drag).
- **SWR/PWR meter chips** added to the top bar (after the Even/Odd pill),
  only rendered once `meter.swr_ratio`/`meter.power_watts` is non-null,
  matching Android's `CompactSwrChip`/`CompactPowerChip` (`TxStrip.kt`). The
  backend plumbing (`qmx_meters.rs`, the `meter` engine event) already
  existed from the Hunt-mode/meters work; this wires it into the UI.
- **Message text column**: fixed `30ch` width (was `flex: 1`), matching
  Android's landscape 30-character message column
  (`docs/landscape-layout.md`).
- **New Prefix** and **Needed** filter chips, real (not stubs):
  - `src-tauri/src/wpx_prefix.rs` + `src/wpxPrefix.ts`: parallel ports of
    Android's `WpxPrefix.java` (CQ WPX prefix extraction), each with the
    full `WpxPrefixTest.java` case list mirrored as unit tests.
  - `Db::worked_prefixes` (+ `worked_prefixes` Tauri command, `api.workedPrefixes()`):
    any-band worked-prefix set, mirroring Android's `QSL_Prefix_list`.
  - "Needed" reuses the existing `workedCalls` set (current-band "worked",
    same semantics as Android's `checkQSLCallsign` despite the name — see
    `db.rs`'s `worked_calls` doc comment) rather than a separate
    QSL-confirmed list, matching Android's actual "Needed" filter
    (`DecodeScreen.kt`).

## Done this round, part 2: New DXCC / New Zone / New State

Fully real, not stubs -- the DXCC-entity, CQ-zone, and US-state reference
data this previously depended on (hunt.ts's `new_dxcc_first` gap, and the
entry below for this doc's own earlier state) is now ported:

- **`src-tauri/src/dxcc.rs`**: a Rust port of
  `CallsignDatabase.java`/`CallsignFileOperation.java`. Parses the AD1C
  "big" `cty.dat` country file (copied in at
  `src-tauri/resources/cty.dat`, same bytes as the Android asset) into
  entity records (country name, CQ zone, DXCC-identifying prefix) plus an
  alias-to-entity map, with the same longest-prefix-match lookup
  (`SUBSTR(call,1,LEN(alias))=alias`, or an exact `=CALL` alias) as
  `CallsignDatabase.getCallsignInfo`'s SQL. `lookup(call)` is a pure
  function over a `once_cell::Lazy`-parsed table, unit-tested including a
  sanity check on the embedded resource's own size (catches a
  truncated/corrupt `cty.dat` at test time, not at runtime). Note a
  faithful quirk carried over from the original: prefix matching can't
  tell a real callsign from a grid square that happens to start with a
  valid single-letter prefix (e.g. `"FN42"` resolves to France's `"F"`) --
  not a bug, see the `worked_dxcc_prefixes` test's comment.
- **`src-tauri/src/us_states.rs`**: `state_for_grid` (grid4 → USPS code via
  `resources/us_grid_states.json`, 713 entries, same bytes as the Android
  asset) plus `worked_state_for`, a full port of
  `GeneralVariables.workedStateFor`'s fallback chain (logged ADIF `STATE`
  when it's a plausible US state *and* the QSO is identifiably a US
  station -- via grid, or a hand-rolled `US_CALL` pattern when the grid's
  too short/missing -- else grid-derived). `QsoRecord.state` already
  existed in `db.rs`; its doc comment used to note grid-derivation didn't
  exist yet -- that gap is now closed.
- **Worked-set aggregation** in `db.rs`: `worked_dxcc_prefixes()` /
  `worked_cq_zones()` (any-band, mirroring `GeneralVariables.dxccMap`/
  `cqMap`) and `worked_states(band, mode)` (per band **and** mode, unlike
  DXCC/Zone -- WAS is earned per band/mode, mirroring
  `workedStateBandModes`), each with a unit test, plus matching Tauri
  commands and `api.*` wrappers.
- **Live enrichment**: `engine.rs`'s `publish_decodes` now stamps
  `dxcc_prefix`/`cq_zone`/`state` onto every `UiMessage` at decode time
  (one `dxcc::lookup` + `us_states::state_for_grid` call per message),
  matching Android's `CallsignDatabase.getMessagesLocation` doing the same
  stamping rather than the frontend re-deriving it per render. The live
  per-message state check is grid-only (no callsign fallback), matching
  Android's live-decode path -- the richer logged-state fallback in
  `worked_state_for` is only for aggregating the *worked* set from the log.
- All three chips are now in `DECODE_FILTERS`, in Android's own order
  (`DecodeScreen.kt`'s `filterOptions`): All, CQ Calls, CQ POTA, New DXCC,
  New Zone, New State, New Grid, New Prefix, Needed, For Me.

## Deliberately not done yet

- **Decode row header / column restructuring** (Time | Call | SNR | Δt).
  Each row currently renders the full decoded message as one text field
  (`.decode-row-msg`) plus separate SNR/Freq/Δt fields — there's no isolated
  "Call" column to label. Needed: pull the callsign out of the message text
  into its own field, then add the header row above `.decode-list`. Not
  started.

## Build note

`npm run tauri -- build --no-bundle` is the only way to get a real,
launchable binary (plain `cargo build` skips `beforeBuildCommand`, producing
a dev-mode binary missing the `custom-protocol` feature). A full release
build on this board took ~36 minutes on the first build after this round's
dependency churn; expect faster incremental builds after that.
