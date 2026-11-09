# Desktop Decode screen: Android UI parity (in progress)

Tracks `feat/decode-waterfall-android-design`'s push to bring the desktop
Decode screen in line with the Android app's layout and data (chips, meters,
columns), beyond the Hunt mode / QMX meters / blank-webview work already on
this branch. Not pushed or PR'd yet — still changing.

## Done this round

- **Call CQ / MORE button**: initially widened (`min-width` + right padding
  on `.pill-cq`) so the MORE badge stopped overlapping the "CQ" text; later
  this round MORE became its own separate `pill-dark` button next to Call
  CQ instead of an overlaid badge at all (see part 4 below) — the widening
  was reverted along with it.
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

## Done this round, part 3: message-logic parity audit + fixes

A direct source comparison against `dev` (fetched fresh, confirmed zero
drift — see the audit at the top of this round) found two real divergences
in application-layer logic built on top of the shared decode core, both
fixed in `src-tauri/src/callsign.rs` (new, with its own unit tests) and
wired into both `engine.rs` (`UiMessage.is_cq`/`to_me`) and `qso.rs`
(`process_rx`'s reply matching):

- **CQ detection**: Android's `Ft8Message.checkIsCQ()` also treats `DE` and
  `QRZ` as CQ-equivalent `call_to` values, not just `CQ`. Desktop only
  checked `"CQ"` — `callsign::is_cq_callsign` now matches all three.
- **"Addressed to me" matching**: Android's `checkIsMyCallsign` does
  substring-containment against the *longest `/`-segment* of the operator's
  own callsign (handles `W1AW` matching a decoded `"W1AW/P"`). Desktop did a
  strict exact match — in **both** the UI highlight and `qso.rs`'s actual
  auto-sequencer reply recognition, so a portable/compound `call_to` could
  go unrecognized mid-QSO. `callsign::is_my_callsign` now matches Android.

Also fixed: a second stale-cache class of bug, separate from the
styles.css-seeding issue earlier on this branch — WebKitGTK's own disk
cache (`~/.local/share/radio.ks3ckc.ft8af.desktop/WebKitCache`) served an
old build's JS bundle after a rebuild. Cleared locally for testing; **not
yet fixed at the root** (would need explicit no-cache headers on the custom
protocol responses) — flagged since a real user updating the app could hit
the same thing.

And a real desktop-only bug: WebKitGTK's window goes solid light-grey and
stops repainting (process stays alive, not a crash) after the window loses
focus, on this board's software/embedded GL driver — a known WebKitGTK
class of issue, not Android-related. Fixed in `linux/run-ft8af.sh` by
setting `WEBKIT_DISABLE_DMABUF_RENDERER=1` before exec'ing the binary;
confirmed via an unfocus/refocus screenshot test.

## Done this round, part 4: decode row rebuilt to match the real Android layout

The first pass at Time/Call/SNR/Δt columns was **wrong** — built from
reading `DecodeRow.kt`'s Kotlin without checking the actual rendered
output. Found real reference screenshots in `screenshots/` (`01_decode.png`
portrait card view, `01b_decode_compact.png` portrait compact view) plus
`docs/landscape-layout.md`'s explicit spec for the layout desktop actually
mirrors (split-view/landscape): the compact row is **Message (30 chars) →
SNR → Freq(Hz) → DT → a status badge**, with the callsign embedded in the
message text, not a separate column — exactly what this branch had before
today. Reverted the Time/Call columns; kept Message/SNR/Freq/Δt.

Replaced the old binary "is this a CQ" dot with a real multi-state status
pill, ported from Android's `resolveQsoStatus`/`StatusPill.kt`:

- **`src/qsoStatus.ts`** (+ tests): `resolveQsoStatus(flags)`, the exact
  priority chain (calling me → POTA → new DXCC → new zone → new state →
  new grid → new prefix → new band → worked → plain CQ), gated by the same
  *default* highlight toggles Android ships (DXCC/zone/band/worked/POTA on,
  state/grid/prefix off — there's no Settings UI yet to change these, so
  the gates are hardcoded for now, ready to wire up when that screen
  exists). `QSO_STATUS_META` carries Android's exact `StatusPill.kt`
  colors (hardcoded hex/rgba, not reusing the app's CSS tokens, since
  several alpha values don't map to any existing one).
- **New Band** needed one more backend piece: `Db::worked_calls_any_band`
  (+ Tauri command, `api.workedCallsAnyBand`) — any-band worked calls,
  mirroring `checkQSLCallsign_OtherBand`, combined with the existing
  current-band `workedCalls` to get "worked elsewhere, not here."
- Two tiers left out rather than faked: **SOTA** (needs a parsed CQ
  "modifier" field the decoder doesn't extract) and **NEW_POTA** (an
  unhunted park specifically, vs. plain POTA — needs a live pota.app spot
  feed, same gap `hunt.ts` already documents for Hunt's `pota_first`).

Also this round: time-group dividers now show `HH:MM:SS UTC · N stations`
**per slot** (was a one-time header for just the newest batch), centered;
the RX/TX gain sliders moved from the Decode chips row to Settings → Audio
(alongside Input/Output device and TX audio offset — functionality kept,
just relocated, not deleted); and the CQ "MORE" badge became a proper
`CqOptionsModal` (a real modal, not a cramped popover, and no longer an
absolutely-positioned badge overlapping the Call CQ pill — it's a plain
separate button next to it now) with Android's actual CQ-variant options
(`screenshots/07_callcq_more.png`, `CqOptionsSheet.kt`): the 8 directional
presets (CQ/DX/NA/EU/SA/AS/OC/AF), a custom 1-4 letter modifier, free text
with Android's exact allowed charset, and a persisted "SAVED" quick-select
chip for the last-sent free text. A preset/custom modifier transmits via
the existing free-text repeat path (`api.freeText`), not a new engine-level
CQ-modifier state machine — it sends and repeats correctly, and Call CQ
still turns into STOP while it's running, but an incoming reply isn't
auto-advanced the way a plain CQ's is; the operator clicks the decoded row
to answer, same as any other station. Documented inline in `App.tsx`.

**Field Day was asked for and is still missing** (Class A-F, transmitter
count, ARRL/RAC section). Investigated: Android's Field Day message isn't
packed by the shared `ft8_lib` `pack77()` both apps otherwise call through
FFI — `FT8Package.generatePack77_fd` is a hand-rolled bit-packer specific
to Android (exact bit layout documented in its own doc comment: two
adjacent c28 callsign fields + R1 + transmitter count + class + section
index + n3/i3, 77 bits total), separate from the standard message path.
Porting it means a bit-exact Rust equivalent of that packer (plus the
86-entry ARRL section table) feeding into the *already-shared*
`encode_tones`/`synth_gfsk` waveform pipeline — feasible, but real
on-air-correctness-critical work (wrong bits either fail to decode
anywhere, harmless, or worse, decode to a *wrong* exchange on someone
else's screen), not something to rush. Not started; needs a deliberate
go-ahead given the stakes.

## Done this round, part 5: CQ modal width, decode sort control, More panel rebuilt

- **CQ Options modal width**: at the default 520px shared `.modal` width, the
  8 directional preset chips (CQ/DX/NA/EU/SA/AS/OC/AF) wrapped after "CQ AS".
  Gave the modal its own `.modal-cq-options` width (640px) rather than
  widening every dialog that reuses `.modal`.
- **Decode-list sort control (Time/Call/SNR/DX)**: `src/decodeSort.ts` (+
  tests) ports Android's `DecodeCollapse.kt` — `nextSortMode` (the same
  Time→Call→SNR→DX→Time cycle) and `collapseByStation` (latest decode per
  callsign, then ordered by the chosen mode; gridless stations sink to the
  bottom in DX mode, same as Android, via `hunt.ts`'s existing
  `maidenheadDistanceKm`). A button in the top bar (after the SWR/PWR chips,
  matching where the operator asked for it — Android's own copy lives in the
  Decode screen's title-bar actions instead) cycles modes and shows
  Android's exact TIME/CALL/SNR/DX labels in amber mono, persisted via
  `decode_sort_mode` config. **Deliberately scoped down from Android**: only
  CALL/SNR/DX collapse-and-reorder the list; the default TIME mode keeps
  this branch's existing full-history, per-slot-grouped view exactly as
  before rather than also switching to Android's always-collapsed
  one-row-per-station list, which would be a bigger, unrequested change to
  already-approved default behavior. Time-group dividers only render in TIME
  mode, matching Android's `showTimeGroupDividers`.
- **More panel (tab bar's "More" toggle) rebuilt on Android logic,
  replacing all of its previous content**: it used to show a desktop-only
  plain-text RX/TX/stage/target/message readout, a manual Tx1-5 stage
  override, and a separate Stop TX button — none with an Android
  equivalent. All of that is gone, replaced by a direct port of Android's
  `OperateControlsDrawer` rows below the Call CQ/Hunt row (which stays on
  TopBar, not duplicated): **Band & Mode** (a card reusing the same
  label+overlaid-`<select>` trick as TopBar's own band picker), **TX period
  (Even/Odd)** (`PeriodTuneDxRow`/`TxPeriodControl` in `TxStrip.kt`, wired
  end-to-end: `EngineCommand::SetTxPeriod` + a `preferred_tx_parity` engine
  field, unit-tested, the `set_tx_period` Tauri command, `api.setTxPeriod`,
  `TxStateEvent.preferred_tx_parity`; `StartCq` seeds the TX-parity lock
  from this preference so picking a period actually restricts which slots a
  new CQ/QSO keys in from, not just cosmetically), **TUNE**, and **TX
  volume** (`TxVolumeRow`, −/slider/+, sharing the same handler as Settings
  → Audio's TX output level slider rather than a second independent one).
  TUNE and Band & Mode now appear in *both* TopBar and this panel, on
  purpose — matching Android, which shows them in both the drawer and the
  screen's title row rather than picking one. The guiding principle this
  round: port Android's actual structure first, even where it duplicates an
  existing desktop control, rather than deciding up front what desktop
  "needs" — some of this may turn out redundant once Settings exists, but
  that's a later call, not one to make while still establishing parity.
  One deliberate difference from Android: there's a third **Auto** state on
  the TX-period control (the prior default — key up at the next free slot,
  in either parity) alongside 1st (even) / 2nd (odd), since Android's
  control always has exactly one of the two selected, which would force
  every CQ to wait for a specific parity by default — a default-behavior
  change nobody asked for.
  **DX (DXpedition/Hound mode)** is shown as a real, visibly disabled button
  in the right spot (not omitted) with an explanatory tooltip: Android's
  `startHoundMode`/`stopHoundMode` is a real Fox/Hound TX sequencer, not a
  UI toggle — same category of real on-air-stakes work as Field Day above,
  left as a documented, visible gap rather than fully omitted or faked.

## Done this round, part 6: real FT4 mode, a shared Band & Mode picker, FT4 timing throughout

**The inline FT8/FT4 toggle-buttons design below was superseded in part 7**:
real Android screenshots showed mode selection belongs inside a proper
"Band & mode" bottom sheet, not separate buttons next to the label. Left
here, append-only, so the reasoning that led to the first (wrong) design is
still on record.

The Band & Mode dropdown only ever listed bands -- there was no way to change
operating mode at all, and the upper-left TopBar picker and the More panel's
Band & Mode card were two independently-built controls (noticed when a
frequency fix to one didn't carry to the other). Both gaps are fixed together:

- **`src/App.tsx`'s `BandModePicker`**: one shared component now backs *both*
  the TopBar title (upper-left) and the
  More panel's Band & Mode card -- same band `<select>` + FT8/FT4 mode
  toggle, a `variant` prop only changing layout (bare title vs. bordered
  labeled card), matching Android's single `bandModeLabel` reused across its
  title row and TxStrip/drawer rows rather than two pickers that can drift
  apart. The mode toggle is a true sibling of the band-select's own overlay
  `<select>` (not nested inside it), since that select is an `inset:0`
  overlay over the whole label and would otherwise swallow clicks meant for
  the toggle buttons.
- **FT4 is real, not a label**: investigated before building anything --
  Android's `ModeProfile.java` already describes FT4 as a fully distinct
  profile (7.5s cycle, 0.048s symbol period, BT=1.0, 105 tones, `isFt8 =
  false`), and the shared C `ft8_lib` already exports `ft4_encode` right next
  to `ft8_encode` (`ft8/encode.h`), with the decoder's `monitor_config_t`
  already taking a raw `ftx_protocol_t` -- desktop's own `Decoder::new` already
  accepted an `is_ft8` bool before this round, just never called with
  `false`. That meant the real work was Rust-side plumbing, not new DSP:
  - **`src-tauri/src/mode.rs`** (new, fully unit-tested): `Mode::{Ft8, Ft4}`,
    ported field-for-field from `ModeProfile` -- `cycle_ms`, `symbol_period`,
    `symbol_bt`, `num_tones`, `is_ft8`, plus derived `audio_ms`/
    `audio_slack_ms`/`tx_latest_ms`/`decode_at_ms` that replace what used to
    be fixed constants in `engine.rs` (`CYCLE_MS`, `TX_SLACK_MS`,
    `TX_LATEST_MS`, `DECODE_AT_MS`). A test pins `Mode::Ft8`'s derived values
    to engine.rs's old constants exactly (15000/12640/2260/13200), so the
    refactor can't silently drift FT8's existing, already-correct timing.
  - **`dsp/encode.rs`**: added `encode_tones_mode`/`generate_signal`/
    `synth_gfsk_for`/`waveform_len_for`, mode-parameterized siblings of the
    existing FT8-only `encode_tones`/`generate_ft8`/`synth_gfsk`/
    `waveform_len` (kept as thin `Mode::Ft8` wrappers of the new ones, with a
    test asserting byte-identical output -- this round must not change a
    single sample of FT8's existing waveform).
  - **`engine.rs`**: `Engine` gained a `mode: Mode` field (persisted via
    `operating_mode` config, same pattern as `dial_hz`); every scheduling
    site that read the old fixed constants now reads `self.mode.*()`
    instead, including the pure, unit-tested gate functions
    (`tx_slot_eligible`, `boundary_tx_ready`, `wf_boundary_row`), which now
    take their cycle/window values as parameters rather than closing over a
    module constant -- new tests exercise them with `Mode::Ft4`'s values to
    confirm the parameterization actually varies behavior, not just FT8's.
    The QMX CAT-tone path (`CatToneTx`) needed real restructuring: its
    `tones` array was fixed at FT8's 79, too small for FT4's 105, so it's
    now sized to the larger mode's tone count (105) with an explicit `len`
    field, plus a `symbol_period` field captured at TX start (`service_cat_tx`
    used to read the module-level FT8 constant directly). `qso.rs` gained a
    real `mode` field (mirroring the existing `band`/`freq_mhz` pattern) so
    logged QSOs record "FT4" instead of a hard-coded "FT8" literal.
    `EngineCommand::SetMode` stops any in-flight TX/QSO/tune first (same
    precaution `DisconnectRig` already took) and respawns the decode-worker
    thread for the new protocol, since a `Decoder` is built for one protocol
    for its whole lifetime (`spawn_decoder_thread`, extracted from what used
    to be inline in `Engine::new` so `SetMode` can call it again).
  - **Frontend**: `api.setMode`, an `OperatingMode` type, `operating_mode`
    config persistence, and the TopBar countdown/progress-bar (`secondsLeft`/
    `pct`), which had hard-coded FT8's 15000ms, now read a small
    `MODE_CYCLE_MS` lookup keyed on the live mode. The waterfall itself
    needed no separate fix: its rows are pushed by the backend's own
    per-mode `wf_boundary_row` cadence, already covered above.
  - **Scope line deliberately held**: per-mode dial frequencies (Android's
    `OperationBand` carries a different FT4 calling frequency per band) were
    *not* added -- the band list stays shared across modes, and the operator
    picks whatever dial frequency they want for either mode (same as desktop
    already allows for custom bands). Adding a real FT4-frequency table is a
    separate, smaller follow-up if it turns out to matter in practice.
  - **FT2** (Android's third mode, double FT4's baud) is *not* ported --
    desktop's FFI layer has no protocol-3 binding at all (`FTX_PROTOCOL_FT4`/
    `_FT8` only), unlike FT4 which the shared library already exported.
    Left as a documented gap, consistent with Field Day/Hound above.

237 Rust lib tests (+11 over the previous round), 125 vitest tests (unchanged
-- no new frontend logic needed its own module). Verified live: mode switch
confirmed via the in-app log ("switched to FT4"), waterfall continuing to
paint correctly post-switch with FT4-cadence row spacing, both pickers
(TopBar and More panel) staying in sync, and zero `ptt=1` events throughout
all testing -- no live transmission was attempted or authorized this round.

## Done this round, part 7: real Band & Mode sheet, a real RX meter, honest Width/NR/MSG gaps

Two corrections/additions, driven by an actual Android screenshot the
operator shared (not a guess from reading Kotlin) and a direct comparison of
the waterfall bottom strip:

- **Band & Mode is a real bottom sheet now**, replacing part 6's inline
  FT8/FT4 toggle buttons. The screenshot showed Android's actual
  `BandModeSheet.kt`: a "Band & mode" sheet titled dialog with a 3-way mode
  tile row (FT8/FT4/FT2, each showing its cycle time -- 15s/7.5s/3.75s) over
  a scrollable band list (name, dial frequency, a checkmark on the current
  selection). `BandModePicker` now just renders the clickable label+chevron
  trigger (still the one shared component behind both TopBar's title and the
  More panel's card); clicking either opens the new `BandModeSheet` modal.
  **FT2 is shown, not omitted** -- it's a real option in Android's sheet --
  but disabled with a tooltip: unlike FT4 (which the shared `ft8_lib` already
  exports `ft4_encode` for), desktop's FFI layer has no FT2 protocol binding
  at all. Android's band rows also carry a short descriptive subtitle
  ("Local, evenings", "Daytime, worldwide") that desktop's `BandInfo` has no
  data for -- left out rather than inventing text that might not match
  Android's actual wording.
- **RX input-level meter, ported for real**: Android's
  `InputLevelIndicator.kt`/`AudioInputLevel.java` classify a peak+RMS reading
  into SILENT/LOW/GOOD/HIGH/CLIPPING and show an RMS-filled bar with a peak
  tick, a percent-of-peak readout, and a color-coded status word ("Low"/
  "Good"/"High"/"Clip"). Desktop's old `AudioBadge` only ever had an RMS
  dBFS number -- `engine.rs` now also computes peak (`peak_linear`) and
  exposes both linear values plus the classification
  (`classify_input_level`, `InputLevelStatus`) on `InputLevelEvent`, using
  Android's exact thresholds (`CLIP_PEAK`, `HIGH_PEAK_DBFS`, `LOW_RMS_DBFS`;
  `SILENCE_DBFS` already matched). The frontend's new `InputLevelIndicator`
  is a close visual port of Android's component. Found and fixed two bugs
  while building this: a copy-paste placeholder (`rms: db` instead of the
  real linear RMS) caught before commit, and a genuinely pre-existing flaky
  test (`preferred_tx_period_seeds_the_lock_on_cq_start`, from the TX-period
  work two rounds ago) that depended on real wall-clock position within the
  TX cycle -- ~15% of real-time instants fall inside FT8's TX window, which
  silently changed `tx_parity`'s expected value. Fixed by pinning
  `clock_offset_ms` to mid-cycle in that test, not by weakening the
  assertion.
- **Width/NR/MSG chips**: added to the waterfall bottom strip in Android's
  order and position, but each disabled with an explanatory tooltip rather
  than omitted or faked -- same treatment as the More panel's DX button.
  Width needs the waterfall's displayed Hz span to become a runtime config
  (today `WF_TOP_HZ`/`WF_MAX_HZ` are fixed constants on both ends). NR needs
  a real noise-reduction DSP stage desktop doesn't have. MSG (labeling
  decoded callsigns directly on the waterfall at their frequency) is lower
  effort than NR -- the backend already carries each decode's `freq_hz` --
  but still needs real canvas work in `drawWaterfallRow` to place text
  per-decode, so it's deferred as its own pass rather than rushed in here.

239 Rust lib tests (+2 over part 6), 125 vitest tests (unchanged). Verified
live against the running app and real decodes: the RX meter reads live
peak/RMS/status correctly (confirmed "Good"/"Low" at different input
levels), the Band & Mode sheet opens and matches the Android screenshot's
layout exactly (mode tiles, band list, checkmark), and FT8↔FT4 switching
through the sheet works. One environment note, not a code issue: screenshots
taken ~1s+ after a click intermittently showed spurious state changes (mode
tiles toggling, the More panel opening) with no further input sent --
zero `ptt=1` events throughout, and an instant (zero-delay) screenshot
confirmed the sheet renders and behaves correctly the moment it opens. This
matches a class of input-delivery quirk already noted earlier on this branch
(xdotool/this test harness), not a React bug.

## Note for when Map work starts

Not started this round -- just a pointer so the prerequisite isn't lost.
Android's QSO-arrow/line rendering on the Map screen depends on a
callsign-to-grid cache, `CallsignQTH(callsign, grid, updateTime)`
(`database/DatabaseOpr.java`): populated on every decode
(`MainViewModel.java` ~line 1447, `addCallsignQTH(msg.getCallsignFrom(),
msg.maidenGrid)`), backfilled from the log at startup
(`fillCallsignQTHFromLog`), and loaded into memory for fast lookup
(`loadCallsignQTHToMemory`) -- because a station's grid only arrives on
specific message types during a QSO, not every decode, so the Map needs this
cache to know where *any* previously-heard station is, not just ones
mid-QSO right now. Desktop has no equivalent table yet; building the Map
screen needs this (or an equivalent) first.

## Build note

`npm run tauri -- build --no-bundle` is the only way to get a real,
launchable binary (plain `cargo build` skips `beforeBuildCommand`, producing
a dev-mode binary missing the `custom-protocol` feature). A full release
build on this board took ~36 minutes on the first build after this round's
dependency churn; expect faster incremental builds after that.
