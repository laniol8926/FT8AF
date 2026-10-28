# Changes on the laniol8926/FT8AF fork (`dev`)

This is what the fork's `dev` branch adds on top of `patrickrb/FT8AF` `dev`,
as of 2026-10-28. Upstream `dev` (70 commits) was merged in on 2026-10-27.
These changes enhance the app; they don't replace upstream behaviour. Where a
control moved, the original is still in place. Layout changes are landscape
only; portrait is as upstream.

Pull requests already sent upstream are noted. Everything else is on the fork
only.

## QRP Labs QMX support

| Change | Where | Upstream |
|---|---|---|
| QMX through the Hamlib backend: model id kept separately from the CI-V address, Digi mode (`MD6;`), I/Q mode off on connect (`Q90;`), open-retry backoff | `HamlibRig`, `MainViewModel.connectRig` | PR #925 (open) |
| RX capture fixes for MediaTek USB HALs: PCM16 capture converted in software, MicRecorder reopens when the input device changes, recovery after a transient AudioRecord failure, USB device matched by name when its id goes stale | `MicRecorder`, `HamRecorder` | PR #949 (open) |
| TX audio attributes and session/low-latency cleanup | `FT8TransmitSignal` | PR #952 (merged) |
| Default the TX audio channel to Left on first QMX connect | `MainViewModel` | fork |
| Pause RX capture while transmitting on the same USB rig | `MainViewModel.shouldPauseRxCaptureForTx`, `HamRecorder.pauseForTx` | fork |
| **Transmit by CAT tone commands (`TA`)**, plus direct-USB TX fixes | see [qmx-transmit.md](qmx-transmit.md) | PR #968 (open, stacked on #925) |
| Drop a stranded carrier (`TA0;` + PTT-off) whenever the QMX reconnects while idle | `MainViewModel.unkeyStrandedQmx` | PR #968 |
| TUNE with CAT tones honours the PTT Delay; key-down logs the RX pause | `FT8TransmitSignal.runQmxCatTune` | fork |
| **SWR and power meters** (`SW;`/`PC;` while keyed): SWR and PWR chips on the landscape title row and in the QSO panel; power keeps the operator identity power current | `HamlibRig`, `QmxMeterReplies`; see [qmx-transmit.md](qmx-transmit.md) | fork |

## Decode screen

| Change | Upstream |
|---|---|
| Compact decode view: one line per decode with the full message | PR #964 (merged) |
| Reports next to the callsign; RR73 not treated as a grid | PR #963 (merged) |
| Tappable band/mode title | PR #961 (merged) |
| Balanced decode depth between Fast and Deep | PR #959 (merged) |
| Per-slot counts on every time-group divider that match the rows shown | fork |
| Landscape layout: controls on the title row, a split view with the decode list beside the map or waterfall, a 30-character message column, compact view by default | fork; see [landscape-layout.md](landscape-layout.md) |
| Worked-before rows: a red strike-through on the message text (drawn over it; `TextDecoration` was too faint), like the waterfall's worked labels | fork |
| Landscape QSO layout: while TX is on the QSO panel fills the left column and the waterfall runs the full height on the right; see below | fork |

## QSO sequencing (fork)

| Change | Where |
|---|---|
| A QSO is logged when our RR73/73 goes out, not when the sequencer advances to it | upstream PR #975 (open) |
| No "No signals decoded" flash at the clear-every-cycle boundary | upstream PR #976 (open) |
| **Stop After** counts a partner re-sending the message we answer (he didn't copy us), so "3" means three tries; only real progress resets it | `FT8TransmitSignal.isPartnerRepeat` |
| **TX watchdog** refreshes when the QSO progresses or a station is tapped (it switched TX off mid-QSO); 1–4 minute options added; a timeout is logged | `doOnSecTimer`, `TransmissionSettings` |
| **Auto-CQ after QSO decides what follows a logged QSO**, however it started: off → TX off (WSJT-X "Disable Tx after sending 73"), on → CQ. The QSO panel ✕ follows the same rule | `shouldStopAfterQso`, `userCancelQso` |
| After TX stopped itself, a repeat from the station just logged (his R+report or RR73) is answered (RR73 / 73, at most twice) without logging the QSO again; a station ended by Max 73 Sends is left alone | `answerRepeatAfterLog`, `isSameQsoAsLogged` |
| A late (deep-pass) reply swaps the over on the air within the late-start tolerance, not only inside the audio slack | `shouldRestartForNewOrder(..., lateStartToleranceMs)` |
| An over cut off by Pause doesn't use up a Max 73 Send | `togglePause` |
| QSO start (TIME_ON) stamped once on our first over, kept when the same station is picked up again within 5 min | `keepsQsoStart` |
| The UI is handed copies of the TX target; a target reset in place left the panel and decode list showing the old station | `TransmitCallsign.copy` |

## Operating aids (fork)

- **Pause** (QSO panel header, next to LOG): holds TX while it stays armed;
  pressed mid-over it keys up at once so the waterfall shows our own slot.
  Released inside our slot, TX starts late within the late-start tolerance,
  otherwise at our next slot. A held slot doesn't count toward Stop After.
- **TX frequency can't change while an over is on the air** (waterfall tap or
  Settings), with a toast; between overs it changes as before.
- **White line on the QSO partner's frequency** on every waterfall, beside the
  red TX markers.

## Timing (fork)

- **Receive audio latency correction.** With the clock set by NTP or GPS the
  decode DTs still read about +1 s on the motorola + QMX: the captured audio
  runs behind the air. While NTP/GPS owns the clock and sync-from-decodes is
  on, the self-sync estimator shifts only the receive window (config key
  `rxLatencyMs`, ~740–890 ms here), so DT reads ~0 and the early decode pass
  no longer cuts off the end of every signal. TX timing is untouched. The
  saved shift is restored before the first slot after a restart.
- **A slot that starts while RX capture is paused for TX is still decoded**
  (zero-padded to stay aligned); before, the partner's whole slot was lost
  after a late over.

## Landscape QSO layout (fork)

On the Decode screen in landscape, while TX is on (calling CQ or in a QSO):

- the right panel is the waterfall, full height, even if Map was picked;
- the decode list is hidden and the QSO panel fills the left column: header
  (station, report, SWR, PWR, PAUSE, LOG, ✕ — no "QSO" tag or "tap to
  view"), the exchange list using the whole height, the TX buttons;
- the QSO sheet doesn't open by itself, overlays only the left column when
  opened, and leaves out the station info and QSO sequence during a QSO.

On other tabs in landscape the docked QSO panel carries its own waterfall on
its right side. The decode list returns when TX is off.

## Diagnostics added to `debug.log`

Every decode addressed to us (`RX to me:`), every skipped TX slot and why,
who switched TX off (`QSO: TX off, from …`), refused TX-frequency changes,
late decodes too late to swap, receive-latency shifts, repeat answers.

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
- **TX power in the log.** In-app QSOs record the operator identity power as
  ADIF `TX_PWR` (new `tx_pwr` column, schema v23). It's written to the ADIF
  log file and exports, the QRZ, Cloudlog and Wavelog uploads, and both UDP
  messages (CQRLOG reads the "QSO logged" Tx power). Import reads `TX_PWR`. A
  QSO is logged without `TX_PWR` when no power is set, never with 0.
- **Operator power is a decimal** ("4.7W", not rounded), editable as one in
  Settings. Older whole-watt values still load.
- **The running `ft8af_log.adi` can be found.** It's still written to the
  app's own files folder, which file managers can't open on Android 11+.
  After each logged QSO the whole file is also copied to
  **Download/FT8AF/ft8af_log.adi**, next to the exports (via MediaStore on
  Android 10+). (`AdifLogFile.mirrorToDownloads`)

## Smaller fixes

- The "Radio not linked" chip shows after a restart (control mode is now
  posted on config load).
- **WSJT-X UDP survives a restart.** The UDP service first started before the
  saved config was read, so it always saw "disabled" and stayed off until the
  toggle was touched again. It now restarts once the config loads, with the
  saved `udp_enabled`, host and port. (`ComposeMainActivity`, config-loaded
  callback)
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
- After the upstream merge, rigs were appended to `rigaddress.txt`, so a
  saved rig index can point at a different rig: re-pick "Qrp Labs QMX
  (Hamlib)" once. A rig model change in Settings now reconnects on USB
  without a restart.

## Open items

- A periodic vertical shift of the landscape display (seen 2026-10-28).
- ADIF export doesn't write `STATE`; email share has no attachment.
- Other `ToastMessage` notices (e.g. late start) still go only to the debug
  list, which the Compose UI doesn't show.
- The voice-command mic button covers the QSO sheet's close button.
