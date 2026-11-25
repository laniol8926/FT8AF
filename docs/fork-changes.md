# Changes on the laniol8926/FT8AF fork (`dev`)

This is what the fork's `dev` branch adds on top of `patrickrb/FT8AF` `dev`,
as of 2026-11-11. Upstream `dev` was last merged in on 2026-11-11 (3c59b078).
These changes enhance the app; they don't replace upstream behaviour. Where a
control moved, the original is still in place. Layout changes are landscape
only; portrait is as upstream.

Pull requests already sent upstream are noted. Everything else is on the fork
only.

## Installing the fork build

The newest Android build is always at this link (bookmark it on the phone):

**https://github.com/laniol8926/FT8AF/releases/latest/download/FT8AF-AI5II.apk**

Open it on the phone, let the download finish, tap the file to install. It
installs over earlier fork builds and keeps your data. It can't replace an
upstream (Play Store or upstream release) build without uninstalling that
first, which clears the app's data, so export settings and the ADIF before.

How the link keeps working, for whoever posts a release (Windows PC or the
Quadra):

- Tag `ai5ii-android-<commit>`, published as a normal release marked
  **Latest** (not a pre-release; GitHub's `/releases/latest` skips those).
- Two copies of the debug APK: `FT8AF-AI5II-fork-<commit>-debug.apk` (to tell
  builds apart) and `FT8AF-AI5II.apk` (the fixed name the link uses).
- `gh release create ai5ii-android-<commit> -R laniol8926/FT8AF --latest
  --target <full sha> --notes-file notes.md FT8AF-AI5II-fork-<commit>-debug.apk FT8AF-AI5II.apk`
  (`--target` needs the full 40-character SHA.)
- Older releases stay on the Releases page as history; each one's notes list
  what changed since the one before.

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
| **CQ Watchdog** (Settings → Transmission, 2026-11-16): TX goes off after N CQ cycles in a row with no caller; Unlimited (default) changes nothing. Weak-signal cycles and the cycle after Pause don't count; the count starts over when a caller is picked up, a new CQ run starts, or TX goes off. A cycle with no decodes at all isn't counted (TX watchdog covers a dead band by time). Ported from the Quadra session's desktop work; config key `cqNoReplyLimit` | `FT8TransmitSignal.shouldGiveUpCq`, `setTransmit`; upstream PR #1008 (open) |
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
  check matches call case-insensitively, date, and TIME_ON to the minute:
  CQRLOG/QRZ write `1357`, the app `135745`, and an exact match had re-added
  the same QSO. Band is not compared, as upstream (same call and minute on
  another band = one QSO with a wrong band). The three pairs this caused on
  the test phones were merged on 2026-11-01 (pre-merge copies in
  `Documents\FT8AF-db-backups`). (`DatabaseOpr.checkIsQSL`)
- **LoTW-sourced copies merge into their QSO** (2026-11-03). QRZ.com keeps
  many QSOs twice: the full record and a copy added from LoTW (no
  RST_RCVD, usually no FREQ, TIME_OFF = TIME_ON, a few minutes off) — ~7,400
  on the AI5II log. A copy joins the full record with the same call, date,
  band and mode within 15 minutes, in either import order: its confirmations
  carry over, and a full record arriving later fills in the copy's times,
  frequency, reports and empty grid. A copy's RST_SENT (really the report
  received) never overwrites a stored report. A LoTW-only QSO with no full
  record (e.g. UT5MB 2024-06-07) stays a QSO of its own. The logbook shows
  the "no report" values −120 / −100 blank. (`checkIsQSL`, `matchLotwTwin`)
  Since 2026-11-16 the mode match also accepts a generic family on either
  side (MFSK, MFSK8, PSK, DATA vs FT8, FT4, PSK31): CQRLOG exports FT8 as
  MFSK without SUBMODE, which left LoTW-style copies unmerged (desktop re-import
  of the AI5II log: 1,986 → 1,815 unmerged). Two different specific modes
  (SSB vs FT8) still don't merge, since the time window is already ±15 min.
  Not sent upstream: `matchLotwTwin` itself is fork-only.
- **Import commits in batches** of 200 QSOs (yielding the database to the UI
  when it waits), instead of one flash write per statement: minutes became
  much faster on the phones. The file is streamed one record at a time.
- **Mode is not compared either** (2026-11-02, made on the Quadra with the
  desktop fix, built and tested on Windows). A log re-exported by one
  program can carry FT8/FT4 as generic `MFSK` with no `SUBMODE`, while a
  fresh export from the source still says FT8, so the same QSO came in
  twice. On a match the stored row's call, time and mode are copied onto
  the record, so the flag/STATE/COUNTRY updates still hit that row.
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
- **LoTW-copy merge on import** (`Db::find_lotw_twin_locked`/`merge_matched_qso`
  in `db.rs`), ported from the Android fix above: a LoTW-style record (no
  report, `TIME_OFF` = `TIME_ON` to the minute — desktop's "no report" is a
  genuinely empty `RST_RCVD`, not Android's literal `-120`/`-100` sentinel,
  since that's what a real QRZ.com export actually does) that misses the
  ordinary exact-minute match instead looks for a same call/date/band/mode
  row within 15 minutes of the *opposite* LoTW-style-ness, nearest wins. A
  match fills the stored row's time/reports/frequency/grid from whichever
  side is the full record — a LoTW-style copy's `RST_SENT` can never
  overwrite a real stored report, the same regression Android fixed in
  `d82697f8`. Verified against the user's real files (byte-for-byte the
  files Android was verified against): **45,712** final rows against
  Android's validated **45,615** — the ~97 difference matches the same
  known "band/mode-altered copies don't merge" gap Android flagged, not a
  new bug. A new index (`qso_log_lotw_twin_idx`, schema v6) backs the twin
  lookup; importing a large file against an already-large log is still
  slower than a fresh import (confirmed: ~97s for the ~54,500-record QRZ.com
  file against an already-imported ~45,600-row log, dominated by per-record
  SQL statement overhead, not a missing index) — the live progress counter
  keeps this visibly moving rather than looking frozen, same as before.
- **Logbook table**: ported the table format above — sortable Date/Time/
  Call/Grid/State/Freq/Mode/RST Sent/RST Rcvd/Status/QSL columns and the
  four-mark QSL column (L/E/P/Q). Schema gained per-service QSL tracking and
  logged STATE/COUNTRY to back it. Status pill matches the user's decision
  above: **CONF only for LoTW or QRZ.com confirmed, otherwise blank** — no
  WORKED tier, no upload-service chips in this cell (the QSL column already
  shows which service confirmed it). RST shows blank (not a literal
  `-120`/`-100`) for a "no report" value and sorts such rows last
  (`displayReport`, `isNoReport` in `logbook.ts`). **Known gap**: State only
  shows what was explicitly logged or imported — no grid→state lookup like
  Android's `UsStateLookup` yet.
- **Logbook table rendering**, confirmed necessary at this log's real size:
  removed the artificial 500-row fetch cap (sorting a capped,
  insertion-ordered window silently sorted the wrong thing once the log
  exceeded it), which then needed the sort memoized (was recomputing over
  the whole table on every unrelated re-render) and real row virtualization
  (`computeVirtualWindow`) — rendering all ~54,500 rows as real `<tr>`s with
  no windowing pinned the webview's renderer at 100% CPU indefinitely.
- **Import dialogs**, matching the Android app's own popups exactly (the
  user supplied screenshots of both): a modal "Importing ADIF log…" dialog
  with a progress bar and live "N of M records" count while running, and a
  modal "Import complete" dialog with Added/Updated/Skipped lines and a
  "Done" button, replacing the earlier inline one-line banner.
- **Table layout bug, status/QSL columns**: `display: flex` had been set
  directly on the `<td>` elements for the Status and QSL cells, which
  overrides their implicit `display: table-cell` and pulls them out of the
  table's column grid entirely in WebKit — the QSL marks visually rendered
  stacked under the Status column instead of their own. Fixed by moving
  `display: flex` to an inner wrapper `<div>`, leaving the `<td>` itself as a
  normal table cell.
- **Column header polish**: Freq/RST Sent/RST Rcvd headers stack onto two
  lines (`white-space: pre-line`, an explicit `\n` in the label — not
  ordinary word-wrap, which breaks at an unpredictable width) to free up
  table width; the active sort column's header is yellow, not blue; a
  confirmed QSO's row is no longer tinted — only the Status pill and QSL
  checkmark carry the green, matching Android (the user's reference
  screenshot showed call/grid/etc. in their normal colors regardless of
  confirmation).
- **Stale on-disk CSS, a real gotcha for this dev machine specifically**: the
  app's customization-preserving `styles.css` (in `app_data_dir()`, see
  `resolve_styles` in `main.rs`) predated the stamping mechanism that
  protects genuine hand-edits, so it was never recognized as safe to refresh
  — every CSS-only change this session was silently invisible until the
  stale file was manually cleared. The stamping logic itself is intentional
  and unchanged (tested, protects real user edits); this was a one-time
  migration gap on a file older than the feature.
- **Waterfall**: a white frequency scale (0/500/.../3500 Hz, `WaterfallScale`)
  above the canvas, aligned with the same hz-to-pixel fraction the canvas
  and TX marker already use; a white HH:MM:SS label (current slot's start
  time, from the existing `cycle` tick) overlaid at the top-left, in the
  same position each new 15 s boundary line appears; display width reduced
  to 50% of the window (the scale ruler shrinks with it, staying aligned).
- **Decode screen reformatted toward the Android layout** (`App.tsx`'s
  `DecodeScreen`): columns reordered to Message (fixed 30ch monospace,
  `.msg-col`) / dB / Hz / DT / Status; the per-row UTC column and the
  meaningless "↩ answer" action column removed (the per-cycle time-separator
  row already shows the timestamp once, and the row itself is already
  clickable to answer); the time-separator for the *current* (topmost, since
  decodes are prepended newest-first) cycle now renders too — the original
  `i > 0` check skipped exactly that one. Waterfall moved from a stacked
  block above the table to its own right-hand panel (`decode-split` /
  `decode-detail` / `decode-waterfall`, each ~50% width), sized to fill the
  panel's full height (CSS-only — the canvas's backing pixel buffer,
  `WF_HEIGHT`, was separately bumped from 160 to 500 rows so at least 3 full
  15 s cycles of history stay visible before scrolling off, not just one).
  Status column is a placeholder (no cell content yet) — a real WORKED/NEW
  PFX/NEW BAND pill needs a DXCC/zone lookup per call, not just the worked
  lookup below.
- **Worked-station strikethrough** (`Db::worked_calls` in `db.rs`, `worked_calls`
  command, `api.workedCalls` in `ipc.ts`): the Message cell is struck through
  in red when `call_from` has already been worked on the currently-tuned band
  (`bands::band_for_freq_hz(dial_hz)`, the same function `engine.rs` uses when
  stamping a logged QSO's own `band`) and mode (desktop only ever logs "FT8"
  today). Mirrors Android's `GetAllQSLCallsign` "worked on this band [and
  mode]" query — any past logged QSO counts, not just a QSL-confirmed one.
  Refetched on band change and after every `qso_completed` event.
- **QMX TX-audio-level fixes**, ported from the Android investigation
  documented in [qmx-transmit.md](qmx-transmit.md) (the "took a lot of
  effort to figure out what worked" one):
  - **Left-channel TX audio** (`rig::needs_left_channel_tx_audio`,
    `audio/output.rs`'s `write_frame`): for a Hamlib QMX, the waveform goes
    out on channel 0 only, with every other channel silenced, instead of
    fanning the same sample to all channels. Real-hardware finding
    (confirmed on three independent apps, Android included): the QMX's
    mono-to-stereo upmix when every channel carries the same sample
    underdrives its modulator with no app-side control over the per-channel
    gain that upmix applies; an explicit single-channel signal reaches full
    scale instead.
  - **CAT-tone TX** (new `qmx_cat_tones.rs`; `rig::needs_cat_tone_tx`,
    `Hamlib::send_raw_cat` via the public `rig_send_raw` Hamlib API;
    `Engine::service_cat_tx`/`start_transmit_cat_tones`/`CatToneTx` in
    `engine.rs`): when even the left-channel fix isn't enough to reach the
    QMX's 3-bar input-level threshold, TX goes out as one `TAnnnn.nn;` CAT
    command per FT8 symbol instead of audio at all — the QMX synthesizes the
    FSK tones itself, so there's no audio chain left to underdrive. Both
    fixes currently auto-enable together for any Hamlib QMX (desktop has no
    per-setting UI toggle yet, unlike Android's two independent Settings
    switches). Deliberately **not** run on a second timer thread: Hamlib's
    `RIG*` here is only ever touched from the engine thread (see the
    `unsafe impl Send for Hamlib` note in `rig.rs`), so `service_cat_tx` is
    just another per-tick action on the existing run loop, polled at a
    tighter `TX_CAT_TICK_MS` (10 ms) while a CAT-tone transmission is in
    flight so the 160 ms-spaced symbols stay close to on time without a
    second thread needing synchronized access to the rig connection.
    **Confirmed on real hardware** (2026-11-04): calling CQ over CAT tones
    reaches full RF output on a QMX that audio alone (left-channel included)
    only drove to partial output.
- **Waterfall freezes during TX** (`waterfall_row_due` in `engine.rs`, gated
  on `self.ptt` so it covers both the audio and CAT-tone TX paths the same
  way): a transmitting QMX has no RX signal worth scrolling into view either
  way — silence/leakage for an audio over, nothing at all for a CAT-tone
  one — so new rows stop until PTT drops, instead of scrolling in whatever
  the mic/line input happened to be picking up mid-transmission.
- **Tune button** (`EngineCommand::StartTune`/`StopTune`, `Engine::start_tune`,
  `TuneHandle`, `output::prepare_tune`/`TunePlayback` in `audio/output.rs`):
  a single on/off toggle next to Grid in the bottom bar, for driving a
  steady carrier to tune the antenna without calling CQ. Keys PTT the same
  way a normal over does, then either loops a steady tone (synthesized
  directly at the output device's sample rate, trimmed to a whole number of
  cycles so the loop point has no audible click — no FT8 waveform, no
  resampling) or, on a CAT-tone QMX, sends one steady `TAnnnn.nn;` with
  nothing further until key-up. Red background while on (`.tune-toggle.active`,
  same color as the TX badge); mutually exclusive with a normal
  transmission in both directions (`start_transmit` and `start_tune` each
  refuse to start over the other); Call CQ and the Tx1–Tx5 stage buttons
  disable while tuning. Shares `finalize_tx`'s teardown with Stop TX, so
  Stop TX (and rig disconnect/reconnect) also ends an active tune safely.
- **Open items**: no grid-to-state/country lookup (see above); the ~97s
  large-import-against-a-large-log case could be faster still by caching
  prepared statements across the import loop instead of re-preparing per
  record (not done); the same small (~48-record) band/mode-altered-copy gap
  Android flagged; no map, POTA, QRZ photos, Bluetooth, or PSKReporter yet
  (desktop README); no manual odd/even TX-slot picker, no FT4/FT2 mode (the
  shared decoder core already has FT4 wired into the Rust FFI bindings, just
  hardcoded off — TX encode and the 15 s slot-cycle timing are FT8-only
  throughout `engine.rs`); Status column has no WORKED/NEW PFX/NEW BAND pill
  yet, just the worked-call strikethrough; no per-setting UI toggle for the
  two QMX TX-audio fixes or for Tune's TUNE level (unlike Android, both are
  an unconditional auto-on/rig's-own-power-setting for any Hamlib QMX).
- **QSO completion/logging timing**, a cluster of real bugs found live
  during hands-on QMX/FT-857D testing of Call CQ and Answer CQ:
  - **QSO logged before its closing message was actually sent** — ported
    from a fix already made on the Android side. `complete()` used to log
    the instant the partner's confirming RR73/RRR/73 was *received*
    (`TxStage::Report`/`RReport`'s own match arms), before our own closing
    message (RR73 for the caller, the courtesy 73 for the answerer) had
    gone out. If that transmission then failed (missed TX window, rig
    busy, encode failure), the QSO was logged with the closing message
    never sent. Logging now only ever happens from `notify_transmitted`
    (confirmed sent, not just queued), for either closing message, unified
    under one `log_if_not_yet` (`qso.rs`).
  - **CQ Watchdog counted a slot before any CQ had ever gone out**:
    `is_own_tx_slot` (`engine.rs`'s `handle_decoded`) inferred "haven't
    transmitted yet" from `tx_parity.is_none()`, but `StartCq` seeds
    `tx_parity` from `preferred_tx_parity` immediately — with a fixed TX
    Period (not "Auto"), `tx_parity` is never `None` even though the very
    first real over can still miss its slot's TX window (an operator's
    click lands at an arbitrary point in the cycle). A dedicated
    `has_transmitted_this_run` flag (set only in `maybe_transmit`'s success
    branch) tracks the real signal directly; without it the watchdog gave
    up one real CQ transmission earlier than its configured limit.
  - **TX Period didn't adjust to the station being answered**: `Answer`
    only locked the transient `tx_parity` for that one run; the lasting
    *preference* (`preferred_tx_parity`, what the next CQ start reuses,
    e.g. via Auto-CQ after the QSO) was left alone, silently reverting to
    whatever was set before. `Answer` now updates both for the duration of
    that answer excursion (one or more stations, if the operator abandons
    one to answer another before it resolves), then restores the
    preference back to the operator's own habitual CQ-calling slot once
    the excursion actually ends — on `QsoOutcome::Completed` or a new
    `QsoOutcome::GaveUpTarget` (the TX Watchdog or Max 73 Sends forcing an
    end), via `Engine::preferred_tx_parity_restore`. Deliberately never
    touches the *active* `tx_parity` lock itself, only the preference: a
    watchdog give-up flips `qso.rs`'s own stage back to Cq internally
    without going through `EngineCommand::StartCq`, so a late reply
    reopening a just-abandoned target still finds `tx_parity` correct —
    the specific edge case checked live before building this ("what would
    happen if I've reverted to the message panel because I've given up
    but get a late reply from him"). An explicit `SetTxPeriod` cancels a
    pending restore outright, since the operator manually changing their
    preference mid-excursion is the existing "until i change it" rule
    firing.
  - **Answer derived "opposite the DX" from click time, not the DX's own
    slot**: confirmed live with two screenshots — "my first call to him
    was in the correct slot but every transmission after that was in the
    wrong slot" / "he is transmitting in an odd slot but so am i". Decode
    results land ~13s into a 15s slot, so a slightly late operator click
    can land one slot after the DX's CQ — the *same* parity as the DX, not
    opposite — pinning the whole QSO onto the wrong slot from the start.
    `AnswerArgs` gained `msg_utc_ms` (the target's own decoded-message
    timestamp, already available at all three frontend Answer call sites:
    the decode row, Hunt auto-answer, and the Map screen's CQ marker);
    `Answer` now derives the reply parity from the DX's actual slot
    instead of whatever slot we happen to be in when the command arrives.
    The one caller with no decode timestamp (a UDP-driven WSJT-X Reply
    request) falls back to the old click-time behavior.
  - **Docked panel staying up for one extra retry cycle** after a QSO was
    already logged (the caller's RR73-sent side, still quietly retrying/
    waiting for an optional courtesy 73 in the background) — **superseded
    later the same session**, see below.
  - **Auto-upload-on-log never marked a QSO synced to QRZ**:
    `log_completed_qso` discarded `insert_qso`'s returned row id
    (`Ok(_)`), so `record.id` stayed `None` all the way to the QRZ-upload
    thread; `upload_and_mark_one`'s success path
    (`if let Some(id) = q.id { mark_synced(id, "qrz") }`) silently never
    fired. The upload itself always succeeded (QRZ matches by content, not
    id) — only the local `synced_qrz` flag was wrong, with no UI path to
    self-heal it (the Logbook's "Sync to logging services" button isn't
    wired to the batch re-upload yet). Fixed by filling in the real id
    right after insert.
  - **A late-arriving decode got attributed to the wrong (too-new) slot**:
    the decode worker runs on its own thread and is only polled
    opportunistically, so a result landing just late enough to cross a
    slot boundary was labeled with whatever slot was *current* when
    picked up, not the one its audio was actually captured for — flipping
    its parity and making the own-transmit-slot skip (above) silently
    treat a genuine partner-silence miss as "ours." A `pending_decode_slots`
    FIFO (pushed on dispatch, popped on drain) fixes this, including under
    a backlog of more than one outstanding decode.
- **`maybe_transmit` could key up on a stale slot under load**: the QMX
  CAT-tone TX path blocks the engine thread for a transmission's full
  ~12.64s, one CAT command per symbol. A queued command or a backlogged
  decode result that finally gets processed right after the engine
  unblocks from that could carry a slot label a full cycle behind real
  time — `tx_slot_eligible`'s window check is a bare modulo, so it
  couldn't tell a stale label from a fresh one, and could key up believing
  it was still replying opposite the DX while the radio actually
  transmitted during whatever slot real time had already moved into.
  Found from a live diagnostic trace showing an ~11.6s gap between two
  adjacent eligibility checks for the same labeled slot. Fixed:
  `maybe_transmit` now always derives the current slot fresh from
  `self.cur_slot()`, never trusting a caller's value.
- **`publish_decodes` was stamping `UiMessage.utc_ms` with publish time**
  (`self.now()`) instead of the decode's true slot — confirmed live: "the
  message box shows him calling CQ at the :00 time slot but he actually
  called in the previous :45 slot when looking at the waterfall." The
  decode worker runs on its own thread and is only polled
  opportunistically, so a result published late enough to cross a slot
  boundary showed a timestamp a full slot too new. This also silently
  undermined the Answer slot-parity fix above, since `AnswerArgs::
  msg_utc_ms` derives the reply parity from this exact timestamp. Fixed to
  take the slot's own start time (`slot_id * cycle_ms`) instead.
- **Docked QSO panel now shows every station's activity on the RX
  frequency**, not just the answered target's: `buildQsoLog` used to
  exclude a repeated CQ from the target entirely (Android's #254) and
  filter out every other station's traffic. Live feedback was explicit
  the other way — seeing the target's repeated CQ (is he still calling,
  not yet heard us?) and seeing anyone else on frequency (did he answer
  someone else, is another station on top of him?) are both real,
  actionable information while deciding whether to keep calling. Both
  exclusions removed.
- **CQ-answer give-up is now smarter and separately configurable**: a new
  `target_working_someone_else` check gives up immediately (not after a
  multi-cycle watchdog) the moment the target is seen exchanging with a
  different station, and a new "CQ Answer Attempts" setting
  (`cq_answer_limit`) lets the operator cap how many times to retry
  answering a CQ that's never replied even once, separately from (and
  never exceeding) the broader "TX Watchdog" that governs retries once a
  QSO is actually under way.
- **TX Watchdog ("Stop After" N) counted every non-advancing cycle as a
  failed attempt, even a genuine reply** — a major behavior change,
  confirmed live with a worked example: "TX Watchdog set to 3. I call a
  station, he answers me and the TX Watchdog has gotten incremented, I
  respond to him but get no response from his station, TX Watchdog get
  incremented, in the next slot I get a response from him, and reply, tx
  watchdog gets incremented again and is at 3 so I give up... that should
  not happen." The partner repeating himself (because he hasn't copied our
  last reply) used to count exactly like silence, ported in spirit from
  Android's own `isPartnerRepeat` — reverted, since a live, responding
  partner is fundamentally not the same situation as one who's gone quiet.
  `process_rx` now resets the no-reply counter on *any* genuine message
  from the target, whether or not it advances the QSO stage; only true
  silence still counts. One deliberate exception, confirmed live ("i agree
  rr73 is different"): the courtesy-73 retry stage keeps counting every
  non-advancing cycle unconditionally, since its own cap (Max 73 Sends)
  bounds how many times *we* resend our own closing message regardless of
  whether he's still there — a different kind of safety net than "has he
  gone silent."
- **Docked panel no longer flickers back to "decodes" mid-RR73-retry**
  (supersedes the "Docked panel staying up for one extra retry cycle" fix
  above, from earlier the same session): that fix reverted to the decode
  list the instant RR73 was confirmed sent, reopening only for the
  duration of each retry's own over — live testing showed this looks like
  a "phantom" retransmission, since the panel visibly flickers back and
  forth for a retry the operator has no way to anticipate. New, simpler
  rule, stated live: "we shouldn't switch back to the decode panel until
  either (1) he responds or (2) the send 73 limit is reached whichever
  comes first." The docked panel now just follows the engine's own
  `active` state for the whole RR73 phase, with no special early revert.
- **QRZ pending-upload backlog now retries automatically on launch.** The
  per-QSO auto-upload fired the instant a QSO logs is a single attempt
  with no retry (e.g. no internet at the time), and the only UI control
  that looks like it should catch up a backlog ("Sync to logging
  services") is a stale stub predating this feature, not wired to it.
  `Engine::new` now spawns a background sweep of every `synced_qrz = 0`
  record on startup, gated on a real API key being set (not just the
  enable toggle) — stated live: "the qrz upload should only run at
  startup if the qrz api key is not empty because an empty key means a
  station does not subscribe to the qrz auto upload." A record that still
  fails (e.g. network not up yet this early in boot) stays unsynced and
  just tries again next launch, same as before this existed.
- **New: "Mark QSOs as Uploaded to All Logs"** (Settings -> Logging &
  Awards, no Android equivalent). Importing an ADIF file directly on a
  logging service's own website (e.g. QRZ.com) uploads those QSOs without
  ever touching this app's own `synced_qrz`/`synced_cloudlog`/`lotw_sent`
  columns, so from this app's point of view they still look pending
  forever — and the QRZ startup-retry sweep above would otherwise keep
  trying to re-upload them. This is a blunt "stop trying, I already
  handled it another way" action (`db.rs`'s `mark_all_synced`): flips
  every currently-unsynced QSO's QRZ/Cloudlog/LoTW-sent flags to synced in
  one sweep, with a confirm step, never contacting any of the services
  themselves.
- **LoTW "Download new"/"Download all" confirmations fixed, two real bugs
  found live** (user: "the LOTW download confirmations both new and all
  is not working"). Confirmed via a real request against the live LoTW
  endpoint with the account's own credentials:
  - **"Download all" never actually downloaded all.** `full_refresh`
    omitted the `qso_qslsince` parameter entirely, trusting LoTW to treat
    that as "no limit" -- it doesn't. LoTW silently substitutes its own
    undocumented short default window instead, so "download all" was
    quietly re-fetching almost nothing while claiming to refresh
    everything. Fixed by sending an explicit, safely-ancient
    `qso_qslsince` (1970-01-01, long before LoTW itself existed) instead
    of omitting the parameter.
  - **Any HTTP error response leaked the LoTW password straight into the
    UI.** `ureq::Error`'s own `Display` output embeds the full request
    URL for both its variants, and that URL carries the plaintext
    password in its query string (this module's own top doc comment
    already warns never to log the URL for exactly this reason -- the
    error path just wasn't covered by that rule). Reproduced live: LoTW
    allows only one report request per account at a time and answers a
    second concurrent one with HTTP 503 "Page Request Limit!" -- likely
    what the user actually hit, probably from a prior slow/interrupted
    request leaving the account briefly locked. Fixed with a new
    `describe_request_error` that builds its own message from the status
    code and response body alone, never from the error's own `Display`;
    the 503/"Page Request Limit" case gets a specific, actionable message
    instead of a generic one. The matching-and-apply pipeline itself
    (`parse_report`/`apply_lotw_confirmations`) was verified correct by
    replaying a real downloaded report against a copy of the production
    database -- the downloads were failing before any of that code ever
    ran. `apply_lotw_confirmations` remains UPDATE-only as before -- a
    confirmation with no local match is still never inserted as a new
    QSO, which was never broken and stays that way.
  - A full-history pull on a large log can legitimately take minutes to
    generate server-side, so `full_refresh` now gets a 600s read timeout
    instead of the usual 120s.
- **New: Cloudlog/Wavelog/Nextlog and World Radio League logbook upload**
  (Settings -> Logging & Awards), requested directly by the user even
  though they don't personally use either service ("others use them") --
  neither could be live-verified against a real server/account the way
  QRZ/LoTW were, so this is confirmed correct against Android's own source
  and unit-tested, not confirmed against a live account.
  - **Cloudlog/Wavelog/Nextlog** (`cloudlog.rs`), ported from
    `CloudlogEndpoint.java`/`ThirdPartyService.java`'s Cloudlog section/
    `CloudlogFailureHint.kt`: same shape as QRZ -- an enable toggle +
    address/API key/Station ID dialog with a station picker, Test
    Connection (`api/auth/[key]`), a manual catch-up upload, and
    auto-upload the moment a QSO is logged. Self-hosted installs vary a
    lot (no scheme typed, no `mod_rewrite`, basic-auth-in-the-URL), so the
    address-normalizing/candidate-URL-walking logic is ported faithfully,
    including remembering which URL shape (plain vs `index.php/`) last
    answered so later uploads skip the 404 round-trip.
    **Upload only** -- Android also has an optional two-way sync that
    downloads QSOs from the Wavelog logbook and auto-inserts the ones
    missing locally (issue #965 there, `CloudlogRemoteLog.java`). The user
    explicitly chose not to port that half (a materially bigger, riskier
    feature -- it inserts new QSO rows from a remote server -- that they
    have no way to verify live), via an explicit scope question. A Station
    ID / API key rejection stops the rest of a catch-up batch (every
    remaining row would fail the same way); any other failure just moves
    on to the next row.
    **Real bug fixed before this ever shipped, found while porting**:
    `ureq`'s own redirect handling would have mishandled exactly the
    308-on-POST Wavelog/Nextlog is documented to send when the configured
    address has no trailing slash (silently either dropping the POST body
    by converting to GET, or returning the redirect page's body as if it
    were a real API response). Redirects are now followed manually
    (same-host only, never downgrading https to http -- the request can
    carry the API key), matching Android's own `sendPostRequest` exactly.
  - **World Radio League** (`wrl.rs`), ported from `WrlApi.java` (issue
    #800 there): a Developer API key (no server address -- WRL is a fixed
    cloud service), a logbook picker, Test Connection (`GET /v1/me`), a
    manual catch-up upload, and auto-upload on log. A fresh WRL account
    has no logbook at all and refuses every contact until it has one, so
    the dialog also offers to create one. A 429 with a short `Retry-After`
    is waited out and retried in-process (a catch-up upload of a real
    backlog runs straight into WRL's 60-writes/minute window); an
    auth/quota/5xx rejection stops the rest of a catch-up batch.
    **Disclosed gap**: WRL's POTA/SOTA/WWFF/IOTA activity fields
    (`myActivities`/`theirActivities`) have no data source on desktop --
    there is no POTA/SIG tracking on a logged QSO anywhere yet (same gap
    `logged_state.rs` already flags). The field-building logic is ported
    in full and unit-tested for when that data exists, but nothing is ever
    actually sent today.
  - `db.rs` v10 migration adds `synced_wrl` (Cloudlog/QRZ already had
    `synced_cloudlog`/`synced_qrz`); "Mark QSOs as Uploaded to All Logs"
    (above) now also flips `synced_wrl` in its sweep.
- **New: HamQTH, Club Log and HRDLog.net upload** (Settings -> Logging &
  Awards), requested directly by the user. First of a planned two-part
  addition -- eQSL (needing both upload and a two-part download-then-import
  step, since eQSL's own "download" is a real ADIF file of confirmed QSOs
  rather than a confirmations-only report like LoTW's) is deferred to a
  follow-up. `db.rs` v11 migration adds `synced_hamqth`/`synced_clublog`/
  `synced_hrdlog` (eQSL needs none -- its upload/confirmation flags
  already exist as `eqsl_sent`/`eqsl_rcvd`, v4); "Mark QSOs as Uploaded to
  All Logs" now flips all three too, but deliberately still excludes
  `eqsl_sent`/`eqsl_rcvd` -- unlike the others, `eqsl_rcvd` specifically
  means the *other* station also confirmed via eQSL, a per-QSO fact this
  app has no basis to assert blanket-true for every row.
  - **HamQTH** (`hamqth.rs`), using HamQTH's modern v2 API (confirmed
    live against hamqth.com's own developer docs): `POST /api/v2/login`
    with the operator's callsign + password (HamQTH's own account
    identifier *is* the callsign, so the Settings dialog only asks for a
    password) returns a bearer token, cached in memory and reused across
    uploads; `POST /api/v2/log/qso` uploads one QSO as ADIF-shaped JSON.
    A 409 (QSO already exists) counts as success; a 401 triggers one
    fresh-login retry before giving up on the whole batch.
  - **Upload disabled entirely, 2026-11-23 -- real risk, not a confirmed
    bug**: the user read hamqth.com's own "Upload log" page, which says an
    ADIF upload "replaces all QSOs of this callsign on HamQTH, so always
    upload your complete log." That page describes hamqth.com's own bulk
    FILE-upload feature specifically -- a real live single-QSO test
    against `/api/v2/log/qso` got back `409 "The QSO is in the log
    already"`, which behaves like per-QSO tracking, not a blind
    whole-log replace, so it's NOT confirmed this app's actual upload
    path shares that destructive behavior. But the severity if it does
    (every incrementally-uploaded QSO silently wiping out the operator's
    whole real HamQTH log except itself) was judged too high to keep
    running automatically. Both auto-upload call sites (`engine.rs`'s
    startup catch-up and per-QSO on-log hook) were removed, and
    `hamqth.rs`'s `upload_pending`/`upload_one_logged` now refuse to do
    anything as a second safety layer. `check_connection` (login-only,
    never touches the log) is untouched and still useful. The operator
    uploads to HamQTH manually via hamqth.com's own feature now.
  - **Club Log** (`clublog.rs`) -- **not the same service as Cloudlog**
    above, easy to confuse by name. Uses Club Log's modern per-user token
    API (`clublog.org` -> Edit Your Settings -> API tab -> Tokens),
    confirmed live from the user's own account -- NOT the older
    `realtime.php`/`putlogs.php` email+password+per-application-key
    system most existing third-party documentation describes, which is a
    different, legacy mechanism requiring a key issued to the *software*
    by Club Log's support team rather than something any user can
    generate themselves (a real blocker, raised to the user, that the
    modern token API turned out to sidestep entirely). `POST /api/v1/qso`
    (one QSO) and `POST /api/v1/qso/batch` (up to 100 per call -- used for
    the catch-up upload, a real efficiency QRZ/HamQTH/HRDLog.net can't
    match since none of them have a batch endpoint); `"duplicate"` and
    `"updated"` both count as success same as `"created"`. No dedicated
    test-connection endpoint is documented, so Test Connection posts an
    empty batch and reads whether the token was accepted (422, the
    documented response to an empty list) or rejected (401) -- inferred
    from the documented validation order, not itself a documented
    connectivity check.
  - **Real bug found live 2026-11-23, not a code bug but a token
    config issue**: "our app upload did not work for clublog" -- QSOs
    this app had marked synced never actually appeared on the user's
    real account. A real single-QSO upload (something Test Connection's
    empty-batch probe never exercises) revealed the cause: the user's
    token was created with Club Log's own "Test mode" checkbox on.
    Every upload gets a completely normal `"status":"created"` reply,
    but `"test_mode":true` plus `"Test mode: parameters ok, nothing
    stored."` means nothing is ever actually written. `item_outcome`
    now checks for `test_mode` first, before anything else, so this can
    never again be silently counted as success -- surfaces a clear
    "this token is in Test Mode" error instead. The operator needs a
    fresh, non-test-mode token, and every QSO's `synced_clublog` flag
    needs resetting once they have one, since every prior "success" was
    fake.
  - **HRDLog.net** (`hrdlog.rs`) -- **lower confidence than every other
    integration in this app**, flagged loudly in its own top doc comment,
    the Settings dialog's own description text, and here: HRDLog.net's
    documentation pages (docs/, API.aspx, etc.) returned 403s and generic
    error pages during development and could not be read, so this is
    built from the best available secondhand description (third-party
    logging-program source and forum posts, cross-checked across two
    independent searches) -- `POST https://robot.hrdlog.net/NewEntry.aspx`
    with `Callsign`/`Code`/`App`/`ADIF` form fields. Because the real
    response shape is unconfirmed, Test Connection deliberately doesn't
    try to classify pass/fail itself -- it shows HRDLog's raw response
    text (from a request with a blank ADIF field, so nothing is actually
    logged) for the operator to judge.
  - **Live-verified 2026-11-22/23, and a real TLS-stack bug found and
    fixed**: the user's real Callsign/Code was correct from the start --
    confirmed by live-testing with `curl`/`openssl s_client` against the
    real endpoint, which got HRDLog's normal "missing QSO fields"
    response every time. The app itself still failed every time with
    `ureq`'s generic "Connection Failed", traced to this project's
    rustls-based TLS client specifically: robot.hrdlog.net (or something
    in front of it) resets the TLS handshake for a rustls `ClientHello`
    (even with an explicit ALPN list added to try to close the gap), but
    accepts OpenSSL's. Fixed by giving `hrdlog.rs` alone its own
    `native-tls`-backed HTTP client -- every other integration in this
    app keeps using the project's usual rustls default. Uses
    `native-tls`'s `vendored` feature (OpenSSL compiled from source and
    statically linked into the binary at build time) rather than the
    system's `libssl`, specifically so this doesn't reintroduce the
    runtime system-OpenSSL dependency the original rustls choice was
    made to avoid (see `Cargo.toml`'s own comment) -- no target machine
    needs `libssl` installed, including the ARM SBC this app also
    targets. `describe_transport_error` (shared diagnostic helper) now
    surfaces the real underlying cause (e.g. "Connection reset by peer
    (os error 104)") instead of only ureq's generic label, which is what
    made this look like a one-off network blip at first. Confirmed live
    after the fix: Test Connection returns success against the real
    account.
  - **Real field-name bug found and fixed, 2026-11-23**: a real "Upload
    now" against the user's actual pending QSOs returned "0 uploaded, 3
    failed" with HRDLog's generic "Missing Call QSO_Date Time_On Band
    Mode" rejection, even though all three records had every one of those
    fields populated -- reproduced directly: a fully well-formed ADIF
    record under this module's `"ADIF"` field name got the exact same
    rejection every time, meaning the field never reached HRDLog's parser
    at all. Confirmed the real field name against two independent real
    source files reading `NewEntry.aspx` directly -- IW1QLH's own
    `HRDLOG-net-library` (IW1QLH runs hrdlog.net itself) and the
    `wsjtx-tools` project's `hrdlog-upload.py` -- both use `"ADIFData"`,
    not `"ADIF"`. Also added real duplicate detection (`<insert>1`
    stored / `<insert>0` already-present / `<error>...` rejected,
    confirmed by the same two sources) now that the real response format
    is known, matching Club Log's/eQSL's own
    `Stored`/`AlreadyPresent`/`Failed` result shape. **Not live-tested
    against the real server** -- the user's HRDLog.net account became
    inaccessible account-wide partway through this investigation
    (confirmed unrelated to this app's code: CQRLOG, a completely
    separate program, started failing identically against the same
    account at the same time), so live verification is pending that
    clearing up on HRDLog's end.
- **New: eQSL upload + inbox download** (Settings -> Logging & Awards,
  `eqsl.rs`), the fourth and last of this batch, requested directly by the
  user. Unlike the others, eQSL has no real client anywhere in the Android
  fork at all (confirmed: no `SettingsBackup.kt` entry, no client module --
  `eqsl_sent`/`eqsl_rcvd` there are just imported-ADIF flags, same as
  desktop before this), so this is a genuinely new feature, not a port.
  **Same confidence caveat as HRDLog.net** -- eQSL's help pages are mostly
  login-gated or don't exist as static docs, so this is built from one
  directly-confirmed fact (`ImportADIF.cfm` requires an `ADIFData`
  parameter) plus cross-referenced forum reports, not an official spec.
  Two genuinely separate operations, both exposed in one Settings dialog:
  - **Upload** (`POST /qslcard/ImportADIF.cfm`, `EQSL_USER`/`EQSL_PSWD`/
    `ADIFData`): one QSO per call, deliberately not batched even though
    the real endpoint may accept several `<eor>`-separated records in one
    body -- this project has no confirmed per-record result format for a
    multi-record response, and guessing one wrong risks marking an upload
    "synced" when it wasn't, worse than the lost efficiency.
  - **Download** (`GET /qslcard/DownloadInBox.cfm` with `UserName`/
    `Password`/`QTHNickname`/`ConfirmedQSLOnly`/`RcvdSince`): a genuine
    two-step fetch, confirmed from real-world forum reports rather than
    official docs -- the first call generates a file server-side and
    answers with HTML naming it rather than the ADIF directly, so a
    second `GET` against the named `.adi` URL fetches the real file.
    "Download all" uses an explicit, safely-ancient `RcvdSince`
    (`200001010000`) rather than omitting the parameter, the same lesson
    from the LoTW "download all" fix earlier this session (confirmed
    there, not assumed here, that omitting a since-style parameter isn't
    safe to read as "unbounded"). Applied via `db.rs`'s new
    `apply_eqsl_inbox`, which -- per the user's explicit, repeated
    instruction that nothing in this whole batch of work should ever add
    a QSO to the logbook -- reuses the same ADIF-import dedup matchers an
    ordinary import uses but **drops the insert-if-unmatched branch
    entirely**: a confirmed QSL with no local match is reported back
    (`unmatched`) but never inserted as a new QSO row. Unlike LoTW's own
    confirmations-only report, eQSL's inbox is a real per-QSO ADIF export,
    so reusing the import matchers (rather than LoTW's bespoke
    exact-match-only UPDATE) is the correct fit here, not a shortcut.
  - `eqsl_sent`/`eqsl_rcvd` already existed (v4) -- no new migration
    needed. "Mark QSOs as Uploaded to All Logs" now also flips
    `eqsl_sent`, but still deliberately excludes `eqsl_rcvd`: unlike every
    sync flag in this whole batch (pure "did WE try to upload this"
    bookkeeping), `eqsl_rcvd` specifically means the *other* station also
    confirmed via eQSL -- a per-QSO fact this app has no basis to assert
    blanket-true just because the operator caught up their own uploads
    elsewhere.
  - **Live-verified 2026-11-22** against the user's real eQSL.cc account:
    Test Connection's header-only probe came back exactly the response
    this module already classified as success (`<!-- Reply form eQSL.cc
    ADIF Real-time Interface --> <HTML> <HEAD></HEAD> <BODY> </BODY>
    </HTML>`), and a real "Upload now" run reported `3 uploaded, 18
    already there` -- the 18 came back from eQSL's own server as
    `"duplicate"`, confirming both the upload field shape and the
    duplicate-detection string against the real server, not just a
    plausible guess.
  - **Inbox download live-tested 2026-11-23, one real bug found and
    fixed**: eQSL's actual "nothing new" wording is `"You have no log
    entries"` -- nothing like the three guessed phrases this module had
    before (`"no new"`/`"0 new"`/`"nothing new"`, none ever confirmed
    against a real response), so a real zero-new-confirmations result
    was being misread as a download error. After the fix: "Download
    new" against a ~9-hour-old checkpoint correctly reported 0/0/no
    error, and a full "Download all" against the real account's full
    29,724-record inbox completed in ~21 seconds -- 22 newly confirmed,
    9,742 unmatched (reported, never inserted), total QSO count
    unchanged before and after (45,481), confirming the "never insert"
    rule holds under a real large import, not just unit tests.
    `extract_adif_url` itself needed no fix -- it already correctly
    parses the real server's relative-href link format.
  - **Progress feedback fixed 2026-11-23**, reported live: "need some
    type of progress indicator when downloading confirmations from eQSL
    so it doesn't look like the application is just sitting there doing
    nothing" (eQSL's download is a genuine two-step fetch -- generate
    server-side, then retrieve the file -- so it can legitimately take a
    while, especially "Download all"). Found a real bug while fixing
    this: the dialog had one shared `busy` boolean, so clicking "Download
    all" flipped "Download new"'s own button label to "Working…" too,
    while "Download all" itself never changed from its static label at
    all. Replaced with a single `action` state naming which specific
    operation is running, so each button (Test Connection/Upload now/
    Download new/Download all) shows its own correct label plus a live
    elapsed-seconds counter (e.g. "Downloading… (14s)").

- **QRZ.com confirmation import, clarified and tested 2026-11-23**: two
  real nuances the user raised about importing a QRZ.com logbook export,
  both already handled correctly by the existing `qrz_qsl`/
  `merge_matched_qso` logic -- now with regression tests confirming it
  rather than just asserted in a doc comment:
  - A real QRZ.com export is a mix of confirmed/unconfirmed entries
    (`APP_QRZLOG_STATUS=C` vs. anything else, e.g. `U`) -- re-importing
    it onto already-logged QSOs only flips `qrz_qsl` for the ones QRZ
    itself marks confirmed, never the whole batch.
  - QRZ.com's own logbook also shows a QSO as confirmed when LoTW
    confirmed it, so a real export record can carry both
    `APP_QRZLOG_STATUS=C` and the standard `LOTW_QSL_RCVD=Y` tag
    together -- `qrz_qsl` and `lotw_rcvd` are set independently from the
    two separate fields, never conflated as the same fact.

- **Real data-integrity bug found and fixed 2026-11-23: the Rust test
  suite was writing fake QSOs into the real `~/Downloads/FT8AF/
  ft8af_log.adi`.** `engine.rs`'s own tests complete full simulated QSOs
  using placeholder callsigns (`K0XYZ`/`K1ABC`, etc.), and the "ADIF log
  file" mirror feature (`adif_log_file.rs`) defaults to *on* when its
  config key is unset -- which a test's fresh in-memory `Db` always is.
  With `log_file_path()` unconditionally resolving to the real OS
  Downloads directory, every `cargo test` run on a developer's machine
  silently appended a fake record into that machine's real log mirror
  file. Confirmed the actual damage on the machine this was found on:
  226 of the file's 255 lines were fake (`station_callsign=K0XYZ`),
  built up across however many test runs happened before this was
  caught -- only 28 lines were real QSOs. The original file was backed
  up and the fake lines removed, keeping the real ones in order. Fixed
  at the source: `log_file_path()` now unconditionally returns `None`
  under `#[cfg(test)]`, protecting every current and future test that
  exercises the QSO-completion path, not just a specific test someone
  remembers to patch.

- **New: Caller Queue (pileup-while-busy), 2026-11-23** -- real live bug
  report: operator was mid-courtesy-close on a QSO (repeating RR73) when a
  different station (Italy) called him directly; clicking that new
  station's decode row did nothing. Root-caused (not a bug exactly, a real
  gap): once our own closing message is confirmed sent, the active-QSO
  panel deliberately locks to a read-only transcript for the rest of the
  courtesy-73 retries -- an earlier version reopened the decode list
  instantly on RR73 sent, but live testing showed a "phantom RR73" flicker,
  so the operator asked for the current locked behavior. Nothing inside
  that locked panel could reach a brand-new caller, though.
  - Ported Android's real fix: a **Caller Queue**. While busy with any QSO
    (calling CQ or answering one), every OTHER station that sends a message
    directed at the operator is queued (capped at 10, pruned after one full
    cycle of silence), shown as tappable chips inside the active-QSO panel,
    and tapping one switches to that target without ever reopening the
    decode list. `qso.rs`'s new `QueuedCaller`/`offer_callers` is a
    **separate pass** over each decoded slot from `process_rx` -- which
    early-returns at `Bye73` before ever looking for a third party, exactly
    why the bug happened -- so it runs on every cycle regardless of stage.
    `dequeue_next` supports FIFO or strongest-first (Settings -> Transmission
    -> "Work strongest caller first", Android's own `pileupStrongestFirst`
    key, now real instead of a disabled stub); `dequeue_specific` (the
    manual tap) always honors the operator's choice even if the chip is
    stale. The queue survives switching targets via a normal `answer()` (so
    other queued callers aren't lost), only cleared on `stop()`/`start_cq()`.
  - **Deliberately narrower than Android's `forceLogAndMoveOn`**, which can
    force-log a QSO that only reached a signal report (no RR73 yet):
    tapping a queued caller here reuses plain `answer()`'s existing
    "abandon, don't log the old one" behavior (already proven safe by an
    existing test covering a manual mid-retry target switch). This fully
    covers the reported bug -- the old QSO is already logged by the time
    `Bye73` is reached -- without adding the new record-building logic
    Android's more aggressive behavior needs; a disclosed, deliberate scope
    cut for this pass, not an oversight.
  - Auto-working the queue on natural QSO completion/give-up (Android:
    `dequeueNextCaller`) is **not** wired in this pass either -- only the
    manual tap. `dequeue_next` is still built (the chip display order needs
    the identical comparator either way), so this is a small follow-on, not
    a rearchitecture, whenever wanted.
  - 11 new `qso.rs` tests (including the exact bug-reproduction scenario:
    drive a QSO to `Bye73`, feed a third-party directed decode, confirm it
    queues and a tap cleanly switches targets), 3 new `engine.rs` tests, 7
    new `callerQueue.ts` tests (display ordering/SNR labels, mirroring
    Android's own `CallerQueueDisplayTest.kt`).
  - **Real bug found live right after shipping, fixed same evening**:
    tapping a queued caller sent "HISCALL MYCALL MYGRID" (a fresh
    Grid-stage reply) instead of "HISCALL MYCALL <report>" -- wrong,
    because the caller-queue tap was reusing `qso.answer()`, which is only
    correct for breaking into someone ELSE's CQ bulletin (nothing known
    about them yet). A caller-queue entry is the opposite case: he already
    sent US a directed message, so we already have everything needed to
    reply with a signal report immediately -- exactly what `process_rx`'s
    own `TxStage::Cq` branch already does the instant a bare CQ picks up
    its first responder. New `qso.rs::answer_caller()` classifies the
    caller's actual captured message content (grid-shaped -> Report stage,
    report-shaped -> RReport stage, else falls back to `answer()`'s safe
    Grid-stage baseline) instead of always restarting cold. 3 more
    `qso.rs` tests for the three branches.

- **FT2 now transmits, not just decodes, 2026-11-24.** `Mode::can_transmit()`
  (removed) refused FT2 TX on the premise "there is no `ft2_encode`
  anywhere" -- false: FT2 reuses FT4's exact tone-generation algorithm
  bit-for-bit (same LDPC, XOR scramble, Costas layout, 105-symbol
  structure -- `constants.h`'s own `FTX_PROTOCOL_FT2` doc comment already
  said so), and `dsp/encode.rs`'s `encode_tones_mode` already dispatched
  FT2 to the shared `ft4_encode`, exactly matching Android's
  `ModeProfile.encode()` (which has never gated FT2 TX at all). No
  native/FFI change needed -- only the one artificial refusal in
  `engine.rs::start_transmit`, plus the UI's matching "receive-only"
  greying-out of Call CQ/More/Hunt for FT2 in `App.tsx`/`bandModePicker.tsx`.
  Flagged in `mode.rs`: FT2's TX timing margin is genuinely tighter than
  FT4/FT8's (~1.23s slack vs ~2.46s/~2.36s, since its whole cycle is only
  3.75s) -- worth live-watching for a late-start/clipped-Costas symptom
  specifically on this mode.

- **CQ Answer Attempts / TX Watchdog / CQ Watchdog: no more "unlimited"
  or "Same as TX Watchdog," 2026-11-24.** Two separate live Windows
  reports converged into one real fix:
  - Live bug: with CQ Answer Attempts left at "Same as TX Watchdog" (0,
    deferring to a finite TX Watchdog), the give-up still fired after a
    single real transmission -- despite every test confirming the
    deferral math correct in isolation. The option was eliminated
    outright rather than chased further: `qso::DEFAULT_CQ_ANSWER_LIMIT`
    (3) now floors a literal 0 unconditionally, no "defer" semantics left
    at all. A side effect, accepted as a deliberate improvement: a
    fresh/unconfigured install's Grid-stage CQ-answer retries now cap at
    the default even when TX Watchdog is ALSO left unlimited (nobody
    asked for literally unlimited retries chasing a CQ caller who never
    replies even once).
  - Separately, the operator's own policy call: "we do not want this app
    to become a BOT... I've seen stations calling CQ for hours at
    night... the rule is there must be a control operator present." TX
    Watchdog and CQ Watchdog both previously allowed 0 = literally
    unlimited (CQ Watchdog's own doc comment even said "for
    unattended/robot operation") -- removed the same way, with real
    floors (`DEFAULT_TX_WATCHDOG_LIMIT` = 5, `DEFAULT_CQ_WATCHDOG_LIMIT` =
    40) generous enough for a long stretch of normal operation, never
    indefinite. Max 73 Sends is deliberately untouched -- it already has
    a hard failsafe (`RR73_GIVEUP_CYCLES_FAILSAFE`) even at "Auto".
  - `engine.rs` migrates any legacy-persisted 0 for any of these three
    settings to its real default and writes it back, so Settings shows
    an explicit number immediately rather than a stale "0"/"Unlimited"/
    "Same as TX Watchdog" the UI no longer has a label for.
  - **The actual root cause of the live "gives up after 1-2 tries"
    report, found via a temporary file-based diagnostic log** (eprintln!
    produced nothing at all -- release Windows builds compile with
    `windows_subsystem = "windows"`, which detaches stdout/stderr even
    from an attached console, so the log writes to
    `%TEMP%\ft8af_watchdog_diag.log` instead): the give-up was never a
    counting bug. `target_working_someone_else` -- a separate mechanism
    that gives up *immediately*, bypassing the counted watchdog entirely,
    the instant the target's next decode looks like he's replying to a
    different station -- was firing correctly on REAL decodes (confirmed
    live: legitimate RR73/grid-report/signal-report messages to other
    callsigns, not garbled false positives, e.g. a popular/busy CQ-caller
    in a pileup answering a different simultaneous caller instead of us).
    The operator decided this should no longer be automatic: "take that
    out... I will decide whether to cancel calling him or continue...
    the watchdog is still in place." Removed the function and its call
    site entirely -- the target appearing to work someone else is now
    just a normal silent miss, counted toward the configured cap like
    anything else, never an instant override of it.
  - **Left over, not yet cleaned up**: the diagnostic logging itself
    (`qso::log_watchdog_diag`, the Grid/Report/RReport miss logging, and
    `engine.rs`'s per-`start_transmit`-attempt logging) is still in the
    code, explicitly marked temporary. Harmless (append-only to a temp
    file, best-effort), but should be removed once this is confirmed
    fully resolved across more real operating sessions.

- **Decode worker thread hardened against a native-decoder panic,
  2026-11-24.** Found while chasing the above (a real gap, not confirmed
  as THE root cause): `spawn_decoder_thread` had no panic guard around
  the native FT8/FT4/FT2 decode call. A panic there would kill the
  worker thread silently and permanently (decoding dead for the rest of
  the session, since the channel send it never reaches means every
  future `slot_tx.send` just fails), and -- more subtly -- that slot's
  already-queued `pending_decode_slots` entry would never get consumed,
  misattributing every later decode result to a stale slot id (the same
  desync class already documented on this project's own
  `pending_decode_slots` fix, just a different trigger). Wrapped the
  per-slot decode in `catch_unwind` so a panic degrades to "nothing
  decoded this slot," same as genuine silence.

- **Windows 10 x86 (32-bit) build leg added, 2026-11-24.**
  `build.rs`/`build_support.rs` already had full `i686-pc-windows-msvc`
  cross-compile support wired in from an earlier session (the clang-cl
  `--target` flag for the C DSP core, a `hamlib/x86` vendored-DLL slot)
  but no CI leg had ever exercised it, and `hamlib/x86` shipped only a
  README placeholder with no actual DLLs. Vendored the matching win32
  Hamlib 4.7.1 release's 4 DLLs (checksum-verified against Hamlib's own
  SHA256SUM), and restructured `desktop.yml`'s build matrix to an
  explicit `include` list so `windows-latest` can appear twice -- once
  native (x64, unchanged), once cross-compiling to i686. Worked on the
  first real CI attempt.

- **Native Debian Bookworm arm64 `.deb` build added, 2026-11-24.** The
  existing Linux leg (`ubuntu-latest`) produces an amd64 `.deb` built
  against Ubuntu 24.04's library versions -- a different distro AND
  architecture than this project's actual ARM SBC targets
  (Armbian/Debian Bookworm; confirmed directly on one real device that
  `libwebkit2gtk-4.1-dev`/`libayatana-appindicator3-dev` are both in
  Bookworm's own main+security repos, no backports needed). New
  `build-debian-arm64` job runs on a native `ubuntu-24.04-arm` hosted
  runner (no QEMU/cross-compilation) inside a `debian:bookworm` container,
  so every linked library is genuinely Bookworm's own package version.
  Scoped to just the `deb` bundle target (no `rpmbuild`/AppImage tooling
  in the minimal container). Deliberately non-gating for now (same
  pattern as the existing best-effort `coverage` job) since this is the
  first run of this exact runner+container+Tauri combination -- worked
  on the first attempt; confirmed on a real device that the package's
  declared `Depends:` (`libwebkit2gtk-4.1-0`, `libgtk-3-0`) are already
  satisfied at matching versions. A 32-bit `armhf` build (for e.g. a
  Raspberry Pi Zero 2 W) was explicitly deferred -- no native 32-bit ARM
  hosted runner exists, so it would need slow QEMU emulation, and
  separately, a 512MB-RAM Pi Zero 2 W running a full WebKitGTK app is its
  own open question regardless of whether it builds.

- **RPM version-string bundling bug fixed, plus a new on-demand manual
  build trigger, 2026-11-23/24.** The prerelease `app_version` formula
  (`"<next-patch>-<run_number>"`) contains a literal hyphen, which RPM's
  version-string validator rejects (`deb` tolerates it fine, which is why
  this went unnoticed) -- switched the separator to `+`. Also added a
  `workflow_dispatch` trigger to `desktop.yml` (a `desktop-manual.N`
  throwaway prerelease tag, any branch, never colliding with the real
  `desktop-v*`/`desktop-dev.N` schemes) so a bundled build including a
  Windows installer can be produced on demand from a feature branch
  without touching `main` -- used throughout this session's live-testing
  loop on real Windows hardware.

- **ADIF import now derives COUNTRY/STATE when the source left them
  blank, 2026-11-23.** Live-logged QSOs already backfilled blank COUNTRY
  (DXCC prefix lookup) and STATE (grid-square lookup) via
  `engine.rs::log_completed_qso`; import only ever copied the ADIF tags
  verbatim. A source log with no `COUNTRY` tag left a non-US contact's
  Logbook "Place" column blank (confirmed live: `VE3ILI` -> should be
  Canada, `KG4OJT` -> should be Guantanamo Bay, a distinct DXCC entity
  from the mainland US) even though the callsign alone is enough to
  derive it. Wired the same derivation into `db.rs::import_one_record_
  locked`, filling only genuinely blank fields -- an explicitly-logged
  value in the source ADIF is never overridden.

- **Logbook top bar gets real icons, Map's receiver-count card removed,
  2026-11-24.** Added `lucide-react` (desktop's first icon dependency)
  and swapped the Logbook top bar's Upload/Download/Share Unicode arrow
  glyphs for real icons, matching Android's `LogbookScreen.kt` TopBar
  (Compose's own Material Icons Extended are baked into that library at
  compile time, not portable files, so there was nothing to literally
  copy). Separately, removed the Map's "N receivers / Furthest: <call> /
  Strongest: <call>" glass-card overlay by direct request, deleting the
  now-fully-orphaned `signalReach.ts` module.

- **Decoder research, 2026-11-24 -- next session's starting point.**
  Investigated whether Nexus (`github.com/kd9taw/Nexus`, another Rust ham
  radio app) has a decoder worth borrowing. Finding: its FT8 decoder is a
  thin Rust FFI wrapper around WSJT-X/JTDX's own vendored **Fortran**
  sources (`libtempo/vendor/wsjtx/lib/ft8var/`), not a pure-Rust
  reimplementation and not a C library like this project's own `ft8_lib`
  -- nothing algorithmic worth porting; FT8AF's existing decoder (mature
  C, LDPC+OSD, already measured +2.6%/+5.3% sensitivity gains from an
  earlier session's OSD port) covers the same ground through a simpler,
  more portable toolchain. The real gap is **modes**: Nexus supports
  Q65/FST4/FST4W/MSK144/JT65/WSPR (all full TX+RX per its own README)
  plus a JS8Call-compatible mode (`crates/js8/`, TX status unconfirmed)
  that FT8AF lacks entirely. Getting any of the WSJT-X-family modes would
  require the same heavy Fortran+CMake vendoring Nexus took -- there's no
  clean C-library shortcut the way `ft8_lib` is for FT8/FT4/FT2. The
  operator's stated priority list is specifically **FT8, FT4, FT2, and
  JS8** (not the full WSJT-X mode roster) -- FT8/FT4/FT2 now have full
  TX+RX parity (see above); JS8 is the open item to scope next, starting
  with whether Nexus's `crates/js8/` is itself a standalone, portable
  implementation (unlike the Fortran-backed WSJT-X modes) or still
  Fortran/C-dependent underneath.

- **Deep decode tier now really does subtract-and-redecode, 2026-11-25.**
  Settings -> Advanced's Decode Depth picker's Balanced and Deep tiers
  were identical in effect (both just turned on more LDPC iterations);
  Deep now also runs the real successive-interference-cancellation loop
  Android's own Deep tier runs (`FT8SignalListener.java`), wiring up a
  C implementation (`ft8af_glue/ft8_subtract.c`) that was already
  compiled into Android and built with desktop FFI reuse in mind, but
  until now not compiled into desktop's build at all. Algorithm
  (confirmed identical between Android and the host `decode_bench.c`):
  decode the slot, subtract every hit found so far out of the sample
  buffer (coherent time-domain subtraction -- a complex-gain estimate
  via a moving-average `samples * conj(reference)` filter, matching
  WSJT-X's own "subtractft8" approach), fully rebuild the decoder's STFT
  over the residual from scratch, redecode, repeat until a pass finds
  nothing new or a budget (`max(2500ms, cycle_ms * 0.75)`, mirroring
  Android's own `DeepDecodeBudget`) or a 16-loop hard cap is hit. New
  `EngineCommand::SetDeepDecodeSic`, additive and separate from the
  existing LDPC/OSD toggle so Balanced's existing behavior is untouched.
  Deliberately did **not** replicate Android's early/late dual-buffer
  decode threading (`LateDecode.java`) -- that solves an Android-specific
  fast-TX-turnaround problem; desktop decodes once per slot, so the loop
  just runs once, inline, inside the existing long-lived decoder thread.
  New regression test (`dsp/decoder.rs`) ports `test_subtract.c`'s own
  calibrated "masked same-frequency signal" fixture (a weak signal 2 Hz
  under a strong one, 10 dB down, overlapping in time) end-to-end through
  the real Rust decode path -- confirmed it's genuinely masked before
  subtraction and genuinely recovered after, on the first run.

- **"MyCall known" a priori (AP) decoding, 2026-11-25.** When a
  candidate's blind decode fails, retries it once assuming the message is
  addressed to us -- payload bits 0..28 (a standard message's call_to
  field) forced to our own callsign's known encoding before BP runs.
  WSJT-X's own documented technique (`a2` in its own UI), worth up to
  ~4 dB of sensitivity specifically on replies addressed to the operator
  -- a different, complementary gain to the SIC work above (SIC helps
  overlapping signals, AP helps a single weak signal that's addressed to
  you). New `message.c` export (`ftx_pack_call_ap_bits`) computes the 29
  known bits from a callsign string alone (no live hash table needed);
  new `decode.c` export (`ft8_decode_ap`) retries a candidate with those
  bits forced, reusing the existing BP/CRC/unpack pipeline unchanged.
  `Decoder::decode_all` only attempts this on candidates that already
  failed blindly, so it costs nothing in the common case -- no new
  Settings toggle needed, unlike Decode Depth. Decoder thread respawns on
  a station-callsign change (`EngineCommand::SetStation`), same
  mechanism Decode Depth/SIC already use.

  **A real sign-convention bug was caught and fixed during testing, not
  shipped.** `ldpc.c`'s own doc comment ("codeword[i] = log(P(0)/P(1))")
  turned out to be stale and wrong -- directly contradicted by
  `ft8_extract_symbol`'s own comment ("log(p(1)/p(0))") one function
  away, which is what the real, working decode path has actually been
  built on. The first implementation trusted the wrong comment, forcing
  the 29 known bits backwards -- confirmed by a synthetic regression test
  (hand-built, genuinely valid codewords via a newly-exposed `encode174`,
  bypassing any audio/waterfall simulation) that even a *zero-corruption*
  codeword failed to decode until the sign was flipped. Found before this
  ever reached a real signal, but a reminder that a decoder's own
  internal comments aren't automatically authoritative -- the function
  that actually produces the real values in production is.

- **Windows MSI/NSIS installers now ship Hamlib, not just dev builds.**
  `build.rs` has copied the arch-correct vendored Hamlib DLLs next to the
  exe since the Windows port landed, but only into `target/<profile>/` --
  that made `cargo run`/`tauri dev` work out of the box while silently
  leaving the actual installed app with no DLL next to it, since the
  Tauri bundler only packages the exe plus whatever's declared under
  `bundle.resources`. Real-world symptom: Hamlib had to be installed
  separately on every Windows machine the built MSI/EXE was deployed to.
  Fixed by also mirroring the DLLs into a stable `hamlib-dist/` dir next
  to `tauri.conf.json`, declared as a Windows-only bundle resource via
  `tauri.windows.conf.json` (`$RESOURCES` on Windows is the exe's own
  directory -- confirmed in `tauri-utils`' own `resource_dir` -- the same
  place `load_hamlib()`'s bare-filename DLL search already checks first).

  **Real-world follow-up, confirmed on an actual Windows install:** the
  first version of this fix declared `resources` as a plain glob array
  (`["hamlib-dist/*"]`), which per Tauri's own docs preserves each
  matched file's relative directory component -- so the DLLs landed in
  `<exe_dir>\hamlib-dist\`, a subfolder, not `<exe_dir>\` itself. Adding
  `hamlib-dist` to `%PATH%` by hand made every rig (not just one) appear
  in the picker, confirming Hamlib was failing to load at all, not just
  missing QMX specifically. Fixed by switching to the *map* form of
  `resources` (`{"hamlib-dist/*": "./"}`), which Tauri explicitly
  documents as flattening instead of preserving structure.

- **Candidate-level decode parallelism.** `Decoder::decode_all`'s
  candidate loop (up to 256 per slot) ran one at a time on the single
  long-lived decoder thread; now it fans out across worker threads via
  `std::thread::scope`, each decoding a contiguous slice of the candidate
  list. Investigation found the C core itself was already fully reentrant
  (a sweep of every file in `ft8_lib` for mutable static state found only
  `static const` tables, plus `osd.c`'s `g_rows` lazy-init singleton,
  which was already a correct atomic CAS/spin-wait design) -- the only
  real shared state was `dsp/hashtable.rs`'s thread-local callsign hash
  table, which candidates within one slot use to cross-reference
  hashed/compound callsigns. Fixed by pointing the thread-local at a
  shared `parking_lot::Mutex<HashTable>` instead of an exclusively-owned
  table, so every worker thread can still resolve a hash another thread
  saved earlier, regardless of which one finishes first. Results from all
  workers are merged in original candidate-index order before the
  existing raw-text dedup runs once over the full list, so output
  (messages, order, which duplicate wins a tie) is byte-for-byte identical
  to the old single-threaded scan -- verified directly by a test that
  decodes the same synthesized 10-signal slot with `workers=1` and
  `workers=4` and asserts the message lists match exactly. This is
  internal to one `decode_all` call: `engine.rs`'s single-decoder-thread-
  per-protocol model and the SIC subtract-and-redecode loop are
  unchanged, but SIC directly benefits since it's where a time budget
  previously got burned one candidate at a time. Worker count is a new
  Settings row ("Decode Threads," Advanced, next to Decode Depth) rather
  than a hardcoded constant, per the user's own explicit request --
  "reserve N CPU cores for other work," defaulting to reserving 1,
  respawning the decoder thread on change (same mechanism Decode Depth/
  Deep SIC/MyCall already use).

- **Clicking a directed reply in the decode list no longer re-answers
  with our own grid.** Real bug, found live: answered a station's CQ
  (grid sent), he didn't reply in time so the operator stopped TX, then
  his signal-report reply decoded a cycle later and showed up in the
  decode list (`to_me`, not a fresh CQ). Clicking it went through the
  plain `Answer` path, which always assumed "breaking into someone
  else's CQ bulletin" and re-sent our own grid instead of recognizing
  his report and replying with one of our own -- the same bug class
  `AnswerQueuedCaller` (caller-queue chips) was already fixed for, just
  reachable through a second entry point that fix didn't cover.
  `AnswerArgs` now carries `extra`/`is_cq` from the clicked message, and
  a non-CQ click uses `answer_caller()`'s content-aware stage jump
  instead of always restarting at Grid. New regression test
  (`engine.rs`) reproduces the exact scenario end to end.

- **Decode Threads (the new candidate-parallelism worker-count setting)
  is covered by Settings backup/export.** No code change needed --
  export/import is architected as a blanket dump of the whole `config`
  key/value table minus an explicit `SENSITIVE_KEYS` exclusion list
  (`settings_backup.rs`), and `decodeThreadsReserved` is a plain flat
  key, same shape as `decodeDeep`/`deepDecodeSic` it was modeled after,
  not in that exclusion list. Added a regression test confirming it
  round-trips and is never accidentally marked sensitive, guarding
  against a future `SENSITIVE_KEYS` edit silently dropping it from
  every export.

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
- **Swiping the app away from Recents now exits it.** Before, RX decode, mic
  capture and the wakelock kept running after the swipe (the RX foreground
  service is a plain started service, and `MainViewModel` is a process-wide
  singleton), until the app was force-stopped. `onTaskRemoved()` now runs the
  same full shutdown as the notification's Exit button (stop TX/RX, disconnect
  the rig, stop the recorder and service). `debug.log` shows
  `RxForegroundService: exiting (task removed from Recents)`.
  (`RxForegroundService`, 7e14191c)

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
- **Log re-import, 2026-11-02.** Both phones' logbook tables (`QSLTable`,
  `QslCallsigns`) were emptied and re-imported from the QRZ.com export
  (54,503 records → 52,948 QSOs) plus the CQRLOG dump (+3) = **52,951**, the
  same count as the desktop. Zero exact duplicates (call + date + minute).
  Motorola flags: LoTW sent 52,902 / rcvd 40,931, eQSL sent 52,937 / rcvd
  30,753, QRZ confirmed 47,336, COUNTRY on every row, STATE on 45,543.
  Pre-wipe databases: `Documents\FT8AF-db-backups\*-20261102-before-log-wipe.db`.
  With the LoTW-copy merge (2026-11-03) the QRZ.com export alone gives
  45,567 QSOs (no exact duplicates, QRZ confirmations unchanged). Backups
  before that re-import: `*-before-lotw-twin-reimport.db`.
- **Logbook table sort/search** run off the main thread with keys computed
  once per record; sorting 52,951 QSOs by RST on the main thread had given
  "FT8AF isn't responding".

## Open items

- In landscape the bottom row of Settings → Radio & Audio can sit behind
  other content (Enabled Bands was hard to reach).
- A non-debug build with a higher version code (e.g. upstream
  0.150.0-dev.1498) can't be replaced by our debug build without
  uninstalling it first, which clears the app's data.
- Other `ToastMessage` notices (e.g. late start) still go only to the debug
  list, which the Compose UI doesn't show.
- The voice-command mic button covers the QSO sheet's close button.
- **Desktop import needs the LoTW-copy merge** (Android `d4f3796d` /
  `d82697f8`), or its count stays ~7,400 above the phones'.

### Future enhancements: logbook import/export

Wanted, not started (2026-11-03). AI5II's setup: QRZ.com is the master log
(it only imports LoTW confirmations and won't delete confirmed QSOs); CQRLOG
on the Quadra exchanges with LoTW, eQSL, HamQTH, ClubLog and HRDLog.net.

- **eQSL upload and download.** Upload each logged QSO to eQSL (ADIF posted
  to eQSL's import page, as the QRZ upload works) with a catch-up "send all
  not sent", and download the eQSL inbox to mark QSOs eQSL-received (like
  the LoTW confirmations download; never adds QSOs). Settings → Logging
  section with login / QTH nickname. Decide first which program uploads
  FT8AF's QSOs, since CQRLOG already does. Offline / site down: use the
  mechanism QRZ, Cloudlog and WRL already have — a per-QSO sent flag
  (`eqsl_sent` exists, schema v25), `QsoAutoSync` sending everything not yet
  sent at app start and when the network returns (15 s debounce), and the
  Logbook header's catch-up sync button for "send all not sent".
- **LoTW report reconciliation.** LoTW holds 82,597 QSO records and 39,300
  QSLs for ~45,500 real QSOs. Read the LoTW report ADIF (never import it into
  the logbook: no TIME_OFF, so the copy merge wouldn't catch it) to count
  duplicate uploads, match each confirmation to a QSO, and use each record's
  upload date (`APP_LoTW_RXQSO`) to judge LoTW-only QSOs such as UT5MB.
- **LoTW confirmations download: match the merged QSOs.** It must keep doing
  only what it does now — set `isLotW_QSL` on a matching QSO, never add one
  or change anything else (user rule, 2026-11-03). But it matches the exact
  HH:MM, and some confirmations carry the LoTW copy's time (NG4C 2017-11-01:
  LoTW 0347, merged QSO 0352). Fall back to the merge's test (call, date,
  band, mode, within 15 min) when no minute matches.
  (`LotwClient`, `UPDATE QSLTable SET isLotW_QSL=1 …`)
- **Import summary counts 30 too many "updated"** (QRZ export: 54,503
  records, summary 45,567 added + 8,966 updated = 54,533; the logbook itself
  is right).
- **Suspect LoTW-only QSOs.** From the QRZ export: 108 LoTW-style records
  confirmed only after our own (re-)upload (e.g. N8KYX, K6HN, sent May 2022,
  matched 2026), and 1,394 uploaded but never matched. Review list, not
  automatic deletion.
- **Optional CQRLOG cleanup** on the Quadra: back up, move each copy's
  confirmations onto its full record, delete only copies that have one
  (never standalone LoTW QSOs), then export a clean ADIF (e.g. for a new
  QRZ.com logbook).
