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

## Deliberately not done yet

- **New DXCC / New Zone / New State filter chips.** Scoped but not started:
  - A Rust port of `CallsignDatabase.java`/`CallsignFileOperation.java`:
    parse the AD1C "big" `cty.dat` country file (`ft8af/app/src/main/assets/cty.dat`,
    embedded as an Android asset) into entity records (country name, CQ
    zone, DXCC-identifying prefix) plus an alias-to-entity map, with
    longest-prefix-match lookup (`SUBSTR(call,1,LEN(alias))=alias`, or an
    exact `=CALL` alias) — see `CallsignDatabase.getCallsignInfo`'s SQL for
    the exact matching rule to replicate.
  - Worked-set aggregation: `worked_dxcc_prefixes()` / `worked_cq_zones()`
    (any-band, mirroring `GeneralVariables.dxccMap`/`cqMap`, populated in
    `DatabaseOpr.getQslDxccToMap`).
  - **New State** needs a second, smaller port: `us_grid_states.json`
    (grid4 → USPS state code, 713 entries) for `stateForGrid`, plus
    `GeneralVariables.workedStateFor`'s fallback chain (logged ADIF `STATE`
    when it's a plausible US state *and* the QSO is identifiably a US
    station, else grid-derived) and `isStateWorkedOn` (WAS is per band
    **and** mode, unlike DXCC/Zone). Desktop's `QsoRecord.state` already
    exists (`db.rs`) but its doc comment notes grid-derivation doesn't
    exist yet — this would close that gap.
  - None of this is faked in the meantime: the three chips are simply left
    out of `DECODE_FILTERS` rather than added as no-ops, consistent with
    how `hunt.ts` already documents the same gap for Hunt's
    `new_dxcc_first` priority.

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
