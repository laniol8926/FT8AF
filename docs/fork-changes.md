# Changes on the laniol8926/FT8AF fork (`dev`)

This is what the fork's `dev` branch adds on top of `patrickrb/FT8AF` `dev`,
as of 2026-10-26. These changes enhance the app; they don't replace upstream
behaviour. Where a control moved, the original is still in place.

Pull requests already sent upstream are noted. Everything else is on the fork
only.

## QRP Labs QMX support

| Change | Where | Upstream |
|---|---|---|
| QMX through the Hamlib backend: model id kept separately from the CI-V address, Digi mode (`MD6;`), I/Q mode off on connect (`Q90;`), open-retry backoff | `HamlibRig`, `MainViewModel.connectRig` | PR #925 (open) |
| RX capture fixes for MediaTek USB HALs: PCM16 capture converted in software, MicRecorder reopens when the input device changes, recovery after a transient AudioRecord failure, USB device matched by name when its id goes stale | `MicRecorder`, `HamRecorder` | PR #949 (open) |
| TX audio attributes and session/low-latency cleanup | `FT8TransmitSignal` | PR #952 (open; superseded by CAT tones for the QMX) |
| Default the TX audio channel to Left on first QMX connect | `MainViewModel` | fork |
| Pause RX capture while transmitting on the same USB rig | `MainViewModel.shouldPauseRxCaptureForTx`, `HamRecorder.pauseForTx` | fork |
| **Transmit by CAT tone commands (`TA`)**, plus direct-USB TX fixes | see [qmx-transmit.md](qmx-transmit.md) | fork |

## Decode screen

| Change | Upstream |
|---|---|
| Compact decode view: one line per decode with the full message | PR #964 (merged) |
| Reports next to the callsign; RR73 not treated as a grid | PR #963 (merged) |
| Tappable band/mode title | PR #961 (merged) |
| Balanced decode depth between Fast and Deep | PR #959 (merged) |
| Per-slot counts on every time-group divider that match the rows shown | fork |
| Landscape layout: controls on the title row, a split view with the decode list beside the map or waterfall, a 30-character message column, compact view by default | fork; see [landscape-layout.md](landscape-layout.md) |

## Bands, map and waterfall

- **Crash fix: "All bands & custom frequency".** Upstream `51999c96` made
  the picker scroll inside a sheet that already scrolls ("Vertically
  scrollable component was measured with infinity"). The inner scroll is
  removed. (`FrequencyPickerSheet`)
- **Band & Mode sheet.** When the operator has excluded curated bands, the
  most-used list fills in from their enabled bands, sorted by frequency,
  instead of showing just two bands. (`BandModeSheet.mostUsedBands`)
- **Map PSK filter.** It opens on the current band, and its Band row offers
  only the operator's enabled bands. (`PskFilter.initialPskFilter`,
  `pskFilterBands`)
- **Map in the landscape panel.** A compact header (layer chips on the gear
  row, 30 dp buttons), with no "Map" title or session counts.
- **Waterfall.** A **Width** button replaces the UTC clock. It opens a number
  dialog (2500–5000 Hz) and saves to config key `spectrumWidth`. The
  frequency ruler labels use the full text colour. The offset frequency moved
  from the top bar to the bottom strip next to LIVE. In the landscape panel
  the waterfall shows without the spectrum strip.

## Logbook and NEW STATE

- **NEW STATE is per band and mode.** Worked states are tracked per
  band+mode. They are loaded about a second after startup (before that, no
  NEW STATE flag is shown, rather than a wrong one), and a QSO logged in the
  app updates them immediately. (`GeneralVariables.workedStateBandModes`,
  `CallsignDatabase`)
- **ADIF STATE import.** The logbook table has a `state` column (schema
  v22). ADIF import reads `STATE`, and re-importing fills it on existing rows.
  A logged state is preferred over the grid lookup when it is a US state on a
  US callsign, since STATE also holds other countries' subdivisions, e.g.
  Western Australia "WA". This fixes states the grid table can't resolve,
  such as Delaware and Rhode Island.
  (`GeneralVariables.workedStateFor`, `QSLRecord`, `DatabaseOpr`)
- Not done yet: ADIF export doesn't write `STATE`.

## Smaller fixes

- The "Radio not linked" chip shows after a restart (control mode is now
  posted on config load).
- The landscape map/waterfall panel choice is remembered (config key
  `landscapePanel`).
- Band names no longer break on Windows-built APKs (`.gitattributes` forces
  LF on asset text files). Upstream PR #960, merged.
- ADIF import: RST parsed leniently, invalid rows reported correctly, size
  cap raised to 200 MB. Upstream PRs #951 and #950, merged.

## Testing notes

- Unit tests: `./gradlew testDebugUnitTest`. On Windows, `RatePromptStoreTest`
  always fails (DataStore file rename) and `LogFileImportTest` can run out of
  memory in the full suite (it passes alone). Neither is related to these
  changes.
- CI on the fork runs only the APK build on a push to `dev`. Unit tests and
  lint/detekt/ktlint run on pull requests, and the lint steps only warn.
- Hardware: motorola edge 2022 (Android 15) and Galaxy A32 (Android 13),
  both with a QRP Labs QMX over USB.
