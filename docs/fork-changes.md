# Changes on the laniol8926/FT8AF fork (`dev`)

This is what the fork's `dev` branch adds on top of `patrickrb/FT8AF` `dev`,
as of 2026-11-01. Upstream `dev` (70 commits) was merged in on 2026-10-27.
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
| Worked-before rows (landscape): a red strike-through on the message text (drawn over it; `TextDecoration` was too faint). It marks every station worked before on this band, whatever the pill shows (Decode Highlights → Worked off, or NEW STATE / NEW GRID / POTA outranking WORKED), the same test as the grid tracker. To *hide* worked stations: Decode Highlights → Worked on, mode Hide | fork |
| Decode list (landscape) returns to the newest slot whenever the newest row changes, not only when the list grows: trimmed to its message cap or collapsed by a filter (CQ Calls), it used to keep its place with the new slot above the viewport | fork |
| Landscape QSO layout: while TX is on the QSO panel fills the left column and the waterfall runs the full height on the right; see below | fork |

## QSO sequencing (fork)

| Change | Where |
|---|---|
| A QSO is logged when our RR73/73 goes out, not when the sequencer advances to it | upstream PR #975 (open) |
| No "No signals decoded" flash at the clear-every-cycle boundary | upstream PR #976 (open) |
| **Stop After** counts a partner re-sending the message we answer (he didn't copy us), so "3" means three tries; only real progress resets it | `FT8TransmitSignal.isPartnerRepeat` |
| Giving up at the Stop After limit follows Auto-CQ too: with nobody queued or watched to work, TX goes off instead of calling CQ; it never re-picks the station it gave up on | `giveUpTarget` |
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
- **Waterfall labels (landscape):** only CQs (whole message), decodes addressed
  to us and the QSO partner (callsign) are labelled; other stations show just
  their trace. A labelled station worked before on this band is struck
  through, in landscape as in portrait (since 2026-11-02; the same mark as
  the decode list and the grid tracker).
- **TX reports use the standard form** (`+00`, `-04`, `+07`). A call target
  picked without a known SNR takes it from the station's reply; it had sent
  "R0" (a placeholder 0 written bare), which the partner didn't copy.
  (`TransmitCallsign.getSnr`, `FT8TransmitSignal.checkFunctionOrdFromMessages`)
- **White line on the QSO partner's frequency** on every waterfall, beside the
  red TX markers (portrait too, for now).
- **The waterfall keeps its history across a rotation.** The view going away
  hands its picture to the new one, scaled to the new size (frequencies stay
  aligned), if it is under 60 s old. Kept per orientation: landscape only
  restores a landscape picture, so its trimmed labels aren't replaced by
  portrait's full ones; portrait may start from the landscape picture.
  (`WaterfallView.onDetachedFromWindow`, `restoreHandedOver`)

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
  opened, and leaves out the station info and QSO sequence during a QSO;
- the panel keeps **one running log for the whole TX run**: our CQ overs,
  then whoever answers and the exchange, never cleared mid-run;
- pressing Call CQ leaves the decode list up until the first over keys (or a
  station answers), so there is no empty "Calling CQ / Waiting" panel.

On other tabs in landscape the docked QSO panel carries its own waterfall on
its right side. The decode list returns when TX is off. The waterfall keeps
its picture when its column is resized (TX on/off changes the split).

**The screen jump is fixed.** Android flashes the hidden status bar back about
4 s after each over; padding the layout for it dropped the whole screen 70 px
and slid it back. In landscape the layout now pads only for the display
cutout. (Found from new `Layout:` lines in `debug.log`.)

## Other landscape tabs (fork)

- **Logbook, POTA, Settings, More:** the status line ("RX / TX in 9s") with a
  stacked **Radio / linked** chip and the slot bar (no DT pill) share one row
  at the top instead of two rows docked at the bottom. On the Logbook it sits
  in the header beside "Logbook  55082 QSOs · All bands" (one line).
- **Logbook table.** The Recent tab is a spreadsheet: QSO Date, QSO Time,
  Callsign, Grid, State, Freq MHz, Mode, RST Sent, RST Rcvd, Status, QSL, ⋮
  (Edit/Delete), two-line headings that stay put, grid lines. **Tap a heading
  to sort** by it (again to reverse; highlighted with ▲/▼). Status shows a
  **CONF** pill when LoTW or QRZ.com confirmed the QSO, else nothing — no
  upload tags (they read as confirmations); the QSL column's marks show which
  logbook confirmed it. Portrait cards keep their pill + upload chips, with a
  QRZ.com confirmation also counting as CONFIRMED there. State is the US
  state (grid, else the logged STATE) or, for non-US, the logged country; the
  search field is a slim 36 dp. (`QsoTableRow`, `sortQsoTable`,
  `logbookPlace`)
- **QSL column.** `L✓ E↑ P· Q✓`: LoTW, eQSL, paper card, QRZ.com; ✓ received
  (green), ↑ sent (blue), · neither; Q has no "sent" (the QRZ tag shows the
  upload). Schema v25/v26 add `lotw_sent, eqsl_sent, eqsl_rcvd, qsl_sent,
  qsl_rcvd, qrz_qsl`; LoTW received stays `isLotW_QSL`. Filled by import:
  a CQRLOG export gives LoTW/eQSL/paper, a QRZ.com export gives Q
  (`APP_QRZLOG_STATUS=C`). Re-imports only set flags, so both files can be
  imported in either order. `QSL_RCVD` is the paper card when the record also
  has `LOTW_QSL_RCVD`; alone (LoTW's own report) it still means LoTW. The
  export writes all of them back (`LOTW_QSL_SENT/RCVD`, `EQSL_QSL_SENT/RCVD`,
  `QSL_SENT`, `APP_FT8AF_QSL_RCVD_CARD`, `APP_FT8AF_QRZ_CONFIRMED`).
  (`qslMarks`, `QSLRecord.qslFlags`)
- **Band on/off switches in the band picker.** "All bands & custom
  frequency" has a SHOW / HIDE BANDS list with a switch per band (same setting
  as Settings → Enabled Bands, which is hard to reach in landscape).

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
- **Grid tracker in the map panel.** The map header's **Tracker** button swaps
  the map card (Map tab and the landscape Decode panel) for the FT8CN/FT8TW
  grid tracker: offline night map, the current (newest) slot's QSO lines and
  CQ markers, redrawn whole on every decode pass, one per station pair. Every
  CQ gets a box (already-worked stations struck through); QSO-line boxes only
  for new stations. No grid shading (a big log covered the map).
  The boxes are one line, the message itself (e.g. `CQ KG4KTB EM73`): no
  dBm/ms, distance, prefix country or zone icons. A CQ box keeps its call
  button, shrunk to the text line's height (~26 dp); only that button calls,
  so touching the box while resizing the map never transmits (tap-the-box and
  hold-the-box were tried and dropped). Tapping a box closes it
  (`GridOsmMapView.setCompactInfo`; the full-screen tracker keeps full boxes).
  **The tracker is the default map** (2026-11-02, the user's choice); the
  Tracker button still switches back to the original FT8AF map, and the
  choice is remembered (prefs `ft8af_map` / `mapStyle` = tracker|classic, in
  the settings backup). The full-screen tracker stays in the map's ⚙ menu.
  (`GridTrackerPanel`)
- **The original map on the tracker's tiles.** With Tracker off, the standard
  view keeps all of the FT8AF map's function (Decodes / PSK spots / Worked
  layers, you, connection lines, station markers with labels and pulses, tap a
  station for its card, PSK filter, info pill, + / − / Me) but draws it over
  the grid tracker's map (offline night tiles, gray line) instead of the
  vector outlines; the tile map pans and zooms. Azimuthal projection is still
  the vector map (the tiles are Mercator). (`OsmStandardMap`,
  `drawStationLayers`)
- **Callsign/grid table.** `CallsignQTH` (call → grid) is filled from the log
  at startup (each call's newest grid, ~18.5k calls) and by every saved or
  imported QSO, without overwriting a grid heard on air. On-air grids (CQs
  and replies to anyone) keep updating it. Stations are then placed by grid
  instead of their country's centre, so the tracker draws far more QSO lines.
  The whole table is loaded into memory at startup, so a station's grid is
  known on its first decode (the per-call lookup is async and missed it).
  A message carrying no grid never writes an empty one.

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
- **ADIF COUNTRY import.** New `country` column (schema v24); ADIF import
  reads `COUNTRY` and re-importing fills it on existing rows. Used by the
  logbook table's State column for non-US contacts.
- **ADIF export writes `STATE` and `COUNTRY`** (and the bulk export now
  `TX_PWR`). In-app QSOs have no logged country, so the export takes it from
  the callsign database. (`AdifRecord`, `ShareLogs`, `AdifLogFile`)
- **Export/share date range is checked.** Save and Share wait until each date
  is blank or a real YYYYMMDD and the range isn't reversed; a part-typed date
  used to filter as a string prefix and match the wrong QSOs. When nothing
  matches, the sheet says so instead of sharing a header-only file. The file
  name and email subject carry the range and count
  (`FT8AF-20261101-20261101-….adi`), and each export goes to `debug.log`.
  (`ExportLogSheet.exportDateError`)
- **Upload state survives import/export.** Import marks a QSO as already in
  QRZ when the ADIF has `QRZCOM_QSO_UPLOAD_STATUS=Y` or `APP_QRZLOG_LOGID`
  (every row of a QRZ.com export), and as in Cloudlog/WRL from
  `APP_FT8AF_SYNCED_CLOUDLOG/WRL`. Export writes those flags back. Before,
  a 54k-QSO QRZ import was queued in full for the next QRZ sync. A QRZ
  "duplicate" refusal now marks the row synced, as Cloudlog's already did.
- **LoTW confirmations download.** Settings → Logging → LoTW confirmations
  takes the lotw.arrl.org login and downloads `lotwreport.adi` (incremental
  from the last QSL date, or all), marking matching QSOs LoTW-confirmed. It
  matches on call, date, band, mode and HH:MM, and never adds QSOs. Upload
  isn't possible this way: LoTW needs each QSO signed with the TQSL
  certificate. (`radio.ks3ckc.ft8af.lotw.LotwClient`)
- **STATE / COUNTRY on QSOs made in the app.** At logging, COUNTRY comes
  from the callsign database. STATE is QRZ's state when the station is at
  home (QRZ grid = worked grid, 4 chars) and the QSO isn't portable (no
  POTA/park ref, no /P /M /MM /R); otherwise the worked grid's state (US
  grids). The partner is looked up on QRZ (XML login) in the background when
  TX starts, so the answer is cached at logging time. Both go to the log,
  `ft8af_log.adi`, exports and uploads. (`DatabaseOpr.fillPlace`,
  `pickLoggedState`, `QrzXmlClient.cached/prefetch`)
- **Import no longer duplicates the app's QSOs.** The "already logged?"
  check matches call, date and mode case-insensitively and TIME_ON to the
  minute: CQRLOG/QRZ write `1357`, the app `135745`, and an exact match had
  re-added the same QSO. Band is not compared, as upstream (same call,
  minute and mode on another band = one QSO with a wrong band). The three pairs this caused on
  the test phones were merged on 2026-11-01 (pre-merge copies in
  `Documents\FT8AF-db-backups`). (`DatabaseOpr.checkIsQSL`)
- **Settings backup covers more.** Advanced → Backup & Restore (upstream
  #357) now also saves custom band dials, the theme and followed callsigns.
  Tick "Include sensitive data" to carry the passwords and API keys.
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
  After an uninstall + reinstall the old install's `ft8af_log.adi` blocks
  the name (and is invisible to the new install), which used to make a new
  `ft8af_log (N).adi` on every QSO. The app now reuses its own copy and takes
  the plain name back once it's free: delete the stale `ft8af_log.adi` (and
  the numbered copies) with a file manager after a reinstall.

## Desktop (`desktop/`, Tauri — ported from the Android app above)

The desktop app is a separate codebase (shares only the vendored C `ft8_lib`
DSP core), so these are rewrites of the Android features above, not shared
code. Built and tested on a Linux aarch64 SBC; see `desktop/linux/run-ft8af.sh`
for the Hamlib version workaround needed there.

- **ADIF import** (`Db::import_adif_with_progress`, `adif.rs`): the desktop
  app could only export before. Byte-safe parser (never panics on a
  multi-byte UTF-8 char split by a tag's declared length — real CQRLOG/QRZ.com
  exports with accented names/comments did this); runs in one SQLite
  transaction (a per-row INSERT with no transaction took many minutes on a
  ~50k-row file under SQLite's default rollback-journal mode); live
  "Processing record: N / M" progress and a done/failed banner in the UI.
- **Duplicate matching**, fixed against two real ~54,500-QSO exports (CQRLOG
  + QRZ.com) of the same log that otherwise landed around 61–65k instead of
  ~53k: **MODE is excluded from the match** (mirrors the Android fix above
  and the existing band exclusion) — a program's own ADIF round-trip can
  widen FT4/FT8 to the generic MFSK/PSK family with no SUBMODE to recover the
  real mode from. SUBMODE is read and preferred over MODE on import. A
  duplicate match upgrades a generic MODE to the specific submode and
  TIME_ON to whichever side is more precise (HHMM vs HHMMSS) if the other
  side has it, same "only ever add, never remove" rule as the QSL flags.
- **Logbook table**: ported the table format above — sortable Date/Time/
  Call/Grid/State/Freq/Mode/RST Sent/RST Rcvd/Status/QSL columns, the CONF/
  WORKED status pill, CL/QRZ sync chips, and the four-mark QSL column
  (L/E/P/Q). Schema gained per-service QSL tracking and logged STATE/COUNTRY
  to back it. **Known gap**: State only shows what was explicitly logged or
  imported — no grid→state lookup like Android's `UsStateLookup` yet.
- **Logbook table rendering**, confirmed necessary at this log's real size:
  removed the artificial 500-row fetch cap (sorting a capped,
  insertion-ordered window silently sorted the wrong thing once the log
  exceeded it), which then needed the sort memoized (was recomputing over
  the whole table on every unrelated re-render) and real row virtualization
  (`computeVirtualWindow`) — rendering all ~54,500 rows as real `<tr>`s with
  no windowing pinned the webview's renderer at 100% CPU indefinitely.
- **Open items**: no grid-to-state/country lookup (see above); no map, POTA,
  QRZ photos, Bluetooth, or PSKReporter yet (desktop README).

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

- In landscape the bottom row of Settings → Radio & Audio can sit behind
  other content (Enabled Bands was hard to reach).
- A non-debug build with a higher version code (e.g. upstream
  0.150.0-dev.1498) can't be replaced by our debug build without
  uninstalling it first, which clears the app's data.
- Other `ToastMessage` notices (e.g. late start) still go only to the debug
  list, which the Compose UI doesn't show.
- The voice-command mic button covers the QSO sheet's close button.
