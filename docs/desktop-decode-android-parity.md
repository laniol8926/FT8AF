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

Added right after: an explicit close "X" (`.modal-close`, top-right of the
sheet header) matching the circular button in Android's own screenshot --
`BandModeSheet` previously only closed via the overlay-click-outside
pattern other modals on this branch use. The same phantom-input quirk above
recurred once more while testing this, this time including a visible focus
ring on the Call CQ button after a single real click -- still zero `ptt=1`
both times, and the X button itself was confirmed working via a screenshot
taken immediately after clicking it, but given this project's TX-safety
history, further interactive verification of this specific flow was
deliberately stopped rather than pushed further near the Call CQ control.
Worth a closer look (not blocking, not reproduced as a code bug) before
relying on scripted clicks near TX-adjacent controls again.

## Done this round, part 8: real per-mode frequencies, real Width/NR/MSG, FT2 decode

Closes three gaps part 6/7 explicitly deferred (per-mode dial frequencies,
Width/NR/MSG as real controls instead of disabled stubs, FT2) plus two
layout bugs found while verifying them live.

- **Per-mode dial frequencies, real now**: switching FT8↔FT4↔FT2 retunes
  within the same band to that mode's actual calling frequency, matching
  Android's `OperationBand.getModeBandFreq`/`MainViewModel.setOperatingMode`
  exactly — same-band-only (never jumps bands), leaves the dial untouched if
  the new mode has no entry for the current band (160m/60m have no FT4
  dial), and refuses outright while actually transmitting rather than
  force-stopping. `bands.rs` gained `FT4_BANDS`/`FT2_BANDS` (mirroring
  Android's `bands.txt` `:FT4`/`:FT2` entries) and `mode_band_freq`/
  `bands_for_mode`/`bands_for_mode_entries`; `engine.rs`'s `switch_mode` was
  rewritten around this instead of its previous force-stop-then-switch
  behavior.
- **Width and NR are real DSP now**, not disabled stubs: `wf.rs` gained
  `WfConfig.display_hz` (2500–5000 Hz, Android's `spectrumWidth` range,
  clamped the same way) — `process_row`'s column count now reads it instead
  of the old fixed `WF_MAX_HZ` — and `WfConfig.denoise`, a display-only
  band-tilt flattening (prefix-sum local-average subtraction, ported from
  the shared `fft_display.c`'s algorithm; confirmed by reading that file
  that it never touches decoding, which runs its own independent FFT).
  New `set_wf_display_hz`/`set_wf_denoise` Tauri commands merge into the
  live config rather than requiring every field on every call. **MSG was
  already fully wired in part 7's commit** (`drawSignalLabels` ran
  unconditionally) — this round just added the `showMessages` toggle gate
  the strip's chip was missing.
- **Width's actual UI, corrected from a live-drag popover to Android's real
  design**: a screenshot of Android's "Spectrum Width" dialog showed a
  centered Cancel/Save modal with a bounded number field (2500–5000 Hz
  hint), not a slider that commits on every drag tick. The first
  implementation this round *was* a live popover slider — replaced with
  `SpectrumWidthDialog` (draft state, `onDisplayHz` only called once on
  Save) once the screenshot came in. The live-slider version is also why an
  earlier debugging session this round chased a phantom "stale Band & Mode
  sheet" bug for a long time: dragging across it fired `set_wf_display_hz`
  (a full `WfProcessor` rebuild) on every intermediate pixel, and on this
  board's CPU (shared with live FFT decode) that was enough to build a
  genuine input-processing backlog — clicks got applied correctly but
  several seconds to tens of seconds late, which looked exactly like stale
  React state until traced with a temporary click-logging overlay (since
  removed). Logged here so the same shape of false alarm isn't rediscovered
  from scratch: screenshots taken soon after a synthetic click on this
  hardware are not reliable evidence of application state — re-check
  against `sqlite3 ~/.local/share/FT8AF/ft8af.sqlite` or wait substantially
  longer before concluding anything moved.
- **FT2, decode-only** (matches reality everywhere — confirmed against
  ft8af.app, which only ever claims "decodes in milliseconds — now across
  three modes", never TX, and no `ft2_encode` exists anywhere, Android
  included): `Mode::Ft2` added (`mode.rs`: 3750ms cycle, 0.024s symbol
  period, 105 tones reusing FT4's layout, `can_transmit() == false`,
  `ffi_protocol()` returning the `FTX_PROTOCOL_FT2` enum value).  Decode
  works because desktop's FFI links the shared `ft8_lib` monitor/decoder
  directly (`dsp/decoder.rs`, `dsp/ffi.rs`) rather than through Android's
  JNI wrappers (`ft2_decode_jni.cpp`) — `common/monitor.c` already derives
  FT2's slot_time/symbol_period from the protocol enum, so only the third
  constant needed plumbing through; `Decoder::new` now takes the raw
  protocol int instead of an `is_ft8` bool. `engine.rs`'s `start_transmit`
  refuses to key up in FT2 regardless of caller (covers the Call CQ/More/
  Hunt path and the CAT-tone sub-path it falls through to, and the
  click-a-CQ-row-to-answer path, which still shows the refusal as an error
  toast rather than being separately gated in the UI). TopBar greys out
  Call CQ/More/Hunt while in FT2 with an explanatory tooltip; TUNE stays
  enabled (a steady unmodulated carrier, no message encoding — mode-
  independent, and Android's own FT2 screenshot leaves it enabled too).
  `FT2_BANDS` covers the 12 bands desktop already has edge definitions for
  (matching Android's FT2 table exactly for those, including 160m/60m,
  which FT4 lacks but FT2 has); Android's FT2 table also lists
  1.25m/70cm/23cm/13cm/3cm and the QO-100 satellite dials, skipped here —
  no band-edge or transverter/satellite support anywhere else in this
  codebase, FT8 included.
- **Two layout bugs found live, both fixed**: (1) the waterfall's
  Width/NR/MSG strip was being squeezed to a 1-pixel sliver (or hidden
  behind the tab bar entirely on the standalone Waterfall tab) by its own
  canvas wrapper's `height: "100%"` claiming the whole flex column
  regardless of siblings — changed to `flex: 1` so it properly shares the
  column with `WaterfallStatusStrip` (`.decode-waterfall` also needed
  `min-height: 0`, the usual flex-children-can't-shrink-without-it fix,
  already present on `.waterfall-tab` but missing on its Decode-screen
  sibling). The strip is now also rendered on the standalone Waterfall tab,
  which never had it at all before this round. (2) The bottom `status-line`
  footer (raw `EngineEvent::Info`/`Error` text — "switched to FT8",
  "capturing from 'default' @ 44100 Hz", etc.) was removed per direct
  feedback: a carryover from an early desktop-only prototype stage with no
  Android equivalent. `status`/`setStatus` stay wired for Settings' own
  save-confirmation use; only the always-visible footer render is gone.
- **Decode-list time-group dividers now overlap their own rule** instead of
  sitting in a separate row below a `border-top` line — `HH:MM:SS UTC · N
  stations` is centered directly on the bold slot-delimiter line (the line
  drawn edge-to-edge via `::before` at the row's vertical center, the label
  masking it locally with the same panel background), saving one text
  line's height per slot boundary in the decode list.

259 Rust lib tests (+20 over part 7: `mode.rs` FT2 profile/can_transmit/
ffi_protocol, `bands.rs` FT4/FT2 `mode_band_freq` + band-table coverage,
`engine.rs` `switch_mode` retune/leave-untouched/reject-while-transmitting,
`wf.rs` display_hz clamping/column-count and denoise toggle/band-tilt
flattening), 125 vitest tests (unchanged — this round's frontend work was
layout/wiring, not new pure logic modules). Verified live against the
running app and real decodes on 40m: FT8→FT4→FT2 switching retunes the
dial correctly each time (cross-checked against `operating_mode`/`dial_hz`
in sqlite, not just the UI, after the phantom-input lesson above); FT2
decodes real stations with Call CQ/More/Hunt all correctly greyed out; the
Width dialog matches the Android screenshot and the waterfall visibly
redraws at the new span on Save; the status strip is now visible under the
waterfall on both the Decode screen and the standalone Waterfall tab; zero
`ptt=1` events throughout all of this round's testing — no live
transmission was attempted or authorized.

## Done this round, part 9: the empty-decode-list message is Android's real one

Desktop's "no decodes yet" message was leftover copy from an early
desktop-only prototype stage (`"set your callsign and rig in Settings;
decoding starts on its own"`), never updated to match Android once a real
design existed — flagged directly as a leftover worth replacing with
Android's actual version. Android's `EmptyState` composable
(`ui/decode/DecodeScreen.kt`) isn't one static message: it's 10 different
title/body pairs keyed on the active filter chip (`strings_compose.xml`'s
`decode_empty_*` resources), e.g. "No CQ calls" / "No stations are calling
CQ on this band right now." for the CQ Calls filter, "No calls for you" /
"No stations are calling your callsign right now." for For Me, down to the
default (no filter) case, `"No signals decoded"` / `"Waiting for %s signals
to appear..."` with the live operating mode interpolated in. All 10 are
ported verbatim into `App.tsx`'s new `EMPTY_STATE_TEXT` map and `EmptyState`
component, keyed on the existing `Filter` type (already exactly Android's
filter set) and now also threaded `operatingMode` into `DecodeScreen`'s
props for the default message's mode name.

Android's version also draws `EmptyStateWaves`: an animated canvas graphic
(three sine waves drifting in phase over 6s, radial accent glow behind
them). Ported as a **static** inline SVG approximation (radial gradient +
three wave paths) rather than a canvas animation loop — the filter-aware
copy was the actual complaint, and a static graphic is far cheaper to keep
correct than chasing an animation loop's edge cases. New `.decode-empty-*`
CSS reuses the existing `--muted`/`--text-faint` tokens, which already
mirror Android's `TextMuted`/`TextFaint` exactly.

## Done this round, part 10: a real Map, inside the Decode screen's waterfall pane

The Map tab's `ComingSoonScreen` stub is replaced by a working `MapScreen`
in `App.tsx`, built on top of the `callsign_qth` cache from the previous
round (see `desktop-callsign-grid-table.md`) -- the prerequisite the old
"Note for when Map work starts" above used to flag. This went through two
rounds of direct correction before landing; both are recorded below because
the reasoning (not just the final shape) is what keeps the next person from
repeating either mistake.

**Placement**: the Map is not a separate full-width tab screen.
`DecodeScreen` gained a `rightPane: "waterfall" | "map"` prop; the "Map"
bottom tab now renders the *same* `DecodeScreen` the "Decode" tab does
(same message list on the left), with `rightPane="map"` swapping
`MapScreen` in for `WaterfallView` in the right-hand `.decode-waterfall`
pane -- the waterfall actually unmounts (closes) rather than just hiding,
since the two are mutually exclusive in that one pane. "Decode" keeps
`rightPane="waterfall"` as the default.

**Round 1 correction -- wrong Android source entirely.** The first build
ported `ui/map/MapScreen.kt`, FT8AF upstream's own canvas-drawn,
point-marker map (and its support modules `MapViewport.kt`/
`MarkerStyle.kt`/`ConnectionLines.kt`). Directly corrected: this fork
(`origin` = `github.com/laniol8926/FT8AF`) does not use that map. It uses a
different, older module instead -- `com.k1af.ft8af.grid_tracker.
GridOsmMapView` (Java, osmdroid), wrapped by `ui/map/GridTrackerPanel.kt`.
Confirmed directly in that file's own doc comment: "the grid tracker (**the
default, fork**) rather than the original FT8AF map," and
`loadTrackerMap`'s own default is `true`. The commit history
(`git log --oneline -- '*grid_tracker*' '*GridTrackerPanel*'`) is
extensive, active fork-specific work ("Map: the grid tracker is the
default map," "Map tracker panel: no grid shading, only this cycle's lines
and CQs," etc.) -- not legacy dead code that happened to still compile.

**Round 1 correction -- wrong data volume, caused a real crash.** That
first build also bulk-loaded the *entire* `callsign_qth` table (a short-
lived `all_callsign_qth` query/command, since removed) to plot every
previously-heard station as a point. Tested live on this board, it froze
the whole machine -- `mmap failed ... Cannot allocate memory` -- trying to
hold and render ~800k rows (the real FCC-seeded table) at once on a 3.8GB
SBC. Reading the real GridTracker source (see below) showed this was never
how the feature was supposed to work in the first place: the real one
redraws only the current decode slot, every pass, which is inherently tiny
(one FT8/FT4/FT2 cycle's worth of decodes) -- fixing the Android-parity
question and the crash turned out to be the same fix.

**What GridTracker actually does**, read in full
(`GridOsmMapView.java`, 1018 lines; `GridTrackerPanel.kt`, 113 lines;
`Ft8Message.java`'s `getMaidenheadGrid`/`getToMaidenheadGrid`/`inMyCall`):
- **Current slot only.** The message observer groups the live list by slot
  (`utcTime / slotMillis`), keeps only the newest slot, and on every pass
  clears and redraws from scratch -- "no grid shading: with a big log the
  worked/confirmed/known grids cover the map. Only the current slot's
  lines and CQs show" (the fork's own commit message). Deduped by
  `callsignFrom|callsignTo` pair, first occurrence per pass wins.
- **A CQ message** gets a marker at the caller's position -- no line
  (there's no second party yet).
- **Any directed exchange** (grid-reply, report, R+report, RRR, 73, R73)
  gets a **line between the two real stations** -- call_from's position to
  call_to's, not "me" to them. This is a live "who's working whom on this
  frequency right now" view, not a "where have I heard people" view.
- **Grid resolution**, confirmed against the real source rather than taken
  on description alone: call_from's position uses the message's own grid
  when the decode has one (TX1 CQ, TX2 grid-reply -- exactly desktop's
  existing non-empty `UiMessage.grid` case), else a cache lookup. **call_to's
  position is always a cache lookup** (`GeneralVariables.getGridByCallsign`,
  Android's equivalent of `qth_for_call`) -- the recipient's grid is never
  transmitted in any FT8 message type, so this isn't just a fallback for
  the grid-less TX3-5/73/R73 steps the way call_from's is; it's
  unconditional. `getToMaidenheadGrid` short-circuits to `""` for a CQ
  message, which is why CQ messages never draw a line.
- **Line color**: red (`#FF1E27` in Android) if `msg.inMyCall()` (either
  end is the operator's own callsign), else neutral for an overheard
  third-party QSO.

**Ported** (`desktop/src/gridTrackerSlot.ts` + `gridTrackerSlot.test.ts`,
15 tests covering slot isolation, pair dedup/first-wins, CQ-gets-marker-
no-line, directed-gets-line, call_to always via the resolver, call_from
only via the resolver when the message's own grid is empty,
`involvesMyCall` both ends): `currentSlotMessages` (slot filtering) and
`buildGridTrackerSlot` (the marker/line decision above) as plain,
decoupled TS functions -- same `hunt.ts` pattern of not depending on
`ipc.ts`'s `UiMessage` directly. No matching Kotlin test file exists for
this exact logic (it's fork-only code, not something upstream Android unit-
tests), so these tests assert the behavior directly from the real source
read above instead of porting existing cases.

**`qth_for_call` (not a bulk fetch)**: the one piece of backend plumbing
this needed (`db.rs`/`main.rs`/`ipc.ts`) is a single-callsign lookup,
called for both ends of a directed message (call_to always, call_from only
when its own grid is empty), lazily and cached per session in `MapScreen`
(a miss caches as `null` so it isn't re-queried every render) -- scoped to
only the current slot's distinct callsigns, which is inherently small.

**Not ported, by design** (flagging up front this time, not discovered
mid-build): the real offline raster tile basemap
(`ft8af/app/src/main/assets/nightUSGS4Layer.sqlite`, 4.5MB, an MBTiles-
style archive read via osmdroid's `OfflineTileProvider` -- osmdroid-
specific plumbing with no direct web equivalent); the bundled land/country
GeoJSON outline background from the first build's equirect canvas work
(`mapProjection.ts`/`mapViewport.ts`/`worldOutlines.ts`, still correct and
reused) stands in for it instead, which is a reasonable substitute, not a
faithful port of tile rendering. Also deferred: the moving "direction
pulse" animation along each line (`DirectionLineAnimator`, pure visual
flourish); tap-and-hold-to-call / tap-to-navigate marker interactions
(`gotoCqGrid`, the fork's own "press and hold a CQ box to call" commits);
the richer per-marker/per-line info-window content (frequency/band/RST/
comment, country/ITU/CQ-zone flags) -- v1 shows the message text on click,
matching the fork's own "one-line pop-up boxes (the message only)"
simplification for this panel view (the full-screen tracker activity is
richer; this panel deliberately isn't).

**Gray-line terminator**: kept from the first build
(`terminator.ts`, NOAA low-precision sub-solar point, unchanged) -- this
one genuinely is shared with the real fork behavior, since
`GridOsmMapView.initMap` also calls `setGrayLine()`.

**Color mapping is desktop's own, not Android's literal hex**: the "my
call" QSO line reuses `--tx` (red, close to Android's real
`#FF1E27`) instead of introducing a new token; the "overheard third-party
QSO" line uses `--text` (neutral) in place of Android's plain white, which
reads the same on this dark theme. CQ markers are tinted by
`workedCalls`/`workedCallsAnyBand` (`--ok` green if already worked,
`--accent` amber otherwise) rather than Android's black/blue QSL-status
icon tints, which were tuned for Android's own icon system and wouldn't
read on this theme (black would be invisible). The `--status-new` token
added for the first build's "plain decode" marker role is removed --
that role doesn't exist in the GridTracker model.

**Bundled GeoJSON, fixed after a real failure (round 1, still correct)**:
`world_land.json`/`world_countries.json`, copied verbatim from
`ft8af/app/src/main/res/raw/`. First served from `desktop/public/map/` and
`fetch()`ed at runtime -- silently never loaded, because Tauri's CSP
(`tauri.conf.json`) only allow-lists `connect-src` for `ipc:`, not
same-origin static assets. Fixed by moving both files to
`desktop/src/map-data/` and importing them directly (`resolveJsonModule`,
already on in `tsconfig.json`) -- Vite inlines them into the JS bundle at
build time, no runtime fetch, no CSP interaction.

**Testing**: `gridTrackerSlot.test.ts` (15 tests, see above),
`mapProjection.test.ts`, `mapViewport.test.ts`, `terminator.test.ts`,
`worldOutlines.test.ts` (kept from the first build, still correct). 276
Rust tests, 172 vitest tests.

## Done this round, part 11: ADIF import stopped locking up the board

Starting a real ADIF import test (the user's own ~45,600-row log, then a
second ~54,500-record file on top of it) crashed this 3.8GB board twice.
Traced and fixed three separate, real bugs -- not one -- each confirmed via
a live re-test, not just "it compiles":

**1. The whole file was buffered in memory, 3-4 times over.** The Logbook's
"Import ADIF..." used a plain `<input type="file">`, read the whole file
into a JS string via `file.text()`, sent that string through
`invoke("import_adif", { adif: text })` (one JSON IPC payload), and
`adif.rs`'s `parse_adif` parsed it into one fully-materialized
`Vec<AdifRecord>` before any DB work started. Fixed by porting the same
fix already shipped on the Android side for the identical problem
(`com.k1af.ft8af.log.AdifStreamReader`, "Rewrite ImportSharedLogs to
stream ADIF records instead of buffering the whole file," prompted by
"an 87MB real ADIF log" -- almost exactly tonight's 82MB file). Desktop's
version adapts rather than transliterates: `parse_adif` was already a
byte-offset, declared-length scanner (not Android's delimiter/regex-split
one), so the new `AdifStreamReader` in `adif.rs` just makes that same scan
pull-based over a `Read`, draining consumed bytes as it goes -- peak memory
is ~one record's text, not the file. `parse_adif(text: &str)` is now a
thin wrapper over the same reader (a `Cursor`), so the two paths can't
drift apart. The frontend now gets a file **path** via a native picker
(`pick_adif_file`, `tauri-plugin-dialog`) instead of file contents --
content never crosses IPC at all. New tests: `adif.rs` gained
`AdifStreamReader`-specific cases (tiny-chunk tag/multibyte-value
splitting, a 50,000-record peak-buffer-stays-bounded check,
streamed-vs-whole-string agreement); `db.rs` gained a real-temp-file
streaming-import test.

**2. The file picker (and the import itself) deadlocked the main thread.**
`pick_adif_file` was a plain `fn`, which Tauri dispatches on the main
thread by default -- and `blocking_pick_file()`'s own doc comment is
explicit that it "should *NOT* be used when running on the main thread"
(its own example is `async fn`). Confirmed live: no dialog ever appeared,
no error, just a permanent hang. Fixed by making both `pick_adif_file` and
`import_adif` `async fn` (matching the plugin's documented pattern) --
also fixes a second, related problem: a large import would otherwise have
frozen the whole window for however long it took, with no way for the
progress dialog to repaint.

**3. One transaction for the whole file meant a kill lost everything.**
The import ran inside a single `BEGIN`/`COMMIT` spanning the entire file --
built two rounds ago specifically to avoid a fsync-per-row's own multi-
minute hang, but it meant a kill before the one final `COMMIT` (which is
exactly what happened live, twice, trying to recover from the other two
bugs before they were diagnosed) rolled back *the entire import*, 45,588
rows included. The user: "I think in Android we limited it to 200" --
checked the real mechanism (`DatabaseOpr.beginImportBatch`/
`commitImportBatch`/`endImportBatch`, `ImportSharedLogs.IMPORT_BATCH`):
Android actually commits every **500** records, not 200 (close
recollection, not the exact fork constant), "so a crash or kill loses at
most the open batch; re-importing skips what's already in." Ported
exactly: both `import_adif_with_progress` and
`import_adif_file_with_progress` now commit-and-reopen every
`Db::IMPORT_BATCH` (500) records instead of wrapping the whole file in one
transaction. New test: a 1,200-record fixture crossing two batch
boundaries, confirming nothing is dropped or double-counted at a boundary.

**Checked but not changed**: the dedup/duplicate-matching logic
(`find_duplicate_locked`/`find_lotw_twin_locked`) was independently
verified against the *current* `DatabaseOpr.checkIsQSL`/`matchLotwTwin`
(not a historical commit) at the user's request and found to already be an
exact, up-to-date port -- minute-level `TIME_ON` match, band/mode
excluded, 15-minute LoTW-twin window, nearest-wins tiebreak, and the
LoTW-twin RST_SENT-never-clobbers fix are all already there. No gap.

**A fourth bug, found after the first three were fixed and re-verified**:
with all three import fixes above confirmed correct (the Rust process's
own RSS stayed in the tens-to-low-hundreds of MB through a real import),
the board still neared a full lockup twice more -- confirmed via
`ps aux --sort=-rss` to be `WebKitWebProcess` ballooning to 2-2.7GB RSS,
**not** the Rust backend, and on one occurrence with **no import running
at all** -- just the Logbook screen holding 45,588 rows plus normal
interaction. Root cause, found by reading `LogScreen`'s own `refresh()`
in `App.tsx`: it fetched **every** matching row on every refresh --
`api.searchLog(search, filter, LOG_FETCH_ALL, 0)` with
`LOG_FETCH_ALL = Number.MAX_SAFE_INTEGER`, the table's own comment
explaining this was a *deliberate* earlier fix (a capped page only sorted
that arbitrary window, not the whole matching log, when the user picked a
non-default sort column). The user's recollection -- "we had to limit the
number of logbook rows that were displaying because Android tried to load
the entire logbook" -- pointed at the right class of problem; the exact
fix landed on desktop's own existing (and already-tested) infrastructure
instead of porting an Android mechanism: `search_qsos` (`db.rs`) already
takes `limit`/`offset` and already does `ORDER BY id DESC LIMIT ?2 OFFSET
?3` in SQL (newest-first, the table's own default sort) -- only the
*caller* was passing an unbounded limit. Fixed by capping the fetch
(`LOG_FETCH_LIMIT`, 300 -- the user's own call after confirming the fix
worked, down from an initial 2000) and showing "N of 45,588" via the
existing `qsoCountLabel` (already used for an active search/filter,
extended to also cover a truncated fetch) rather than silently hiding the
rest. Sorting by a non-default column still only sorts within the fetched
page, same known, visible (via the same count label) limitation as
before -- true whole-log server-side sorting for every column (including
the derived Status/QSL/State columns, which would need their scoring
logic replicated in SQL) is a possible follow-up, not done here.

**Verified live, not just unit-tested**: re-ran the exact scenario that
crashed the board twice -- importing `ai5iicqrlog.adi` (32MB) against the
already-45,588-row log from the 82MB file -- with the capped Logbook view
active. `WebKitWebProcess` stayed at 230-280MB RSS throughout (vs. the
2-2.7GB it reached before this fix), available memory held at ~2.6-2.7GB
for the full ~4-minute import, and the row count ticked up incrementally
during the run (proof the part-11 batch-commit fix above is also visibly
working, not just correct in principle) to 45,624+ with zero memory
pressure.

**Testing**: 282 Rust tests (`adif.rs` +4, `db.rs` +2 net vs. part 10's
count), 172 vitest tests (no new test for the fetch cap itself -- it's a
one-line change to an existing, already-tested `api.searchLog` call; the
underlying `search_qsos` pagination was already covered by
`search_qsos_matches_predicate_and_paginates`).

## Done this round, part 12: Logbook display parity audit

The user asked for a direct side-by-side comparison (`~/Pictures/android.png`
vs `~/Pictures/desktop.png`, a real device screenshot vs. this build) with
the explicit goal "they should be identical." Found and fixed two real
divergences, confirmed against Android's actual current source (not
guessed from the screenshot):

- **Status column was missing the upload-sync chips.** Android's Status
  cell shows the CONF pill *and* a separate "QRZ"/"CL" chip when the row's
  been uploaded to QRZ.com or Cloudlog/Wavelog/Nextlog
  (`syncChipLabels`/`SyncChip` in `LogbookScreen.kt`) -- a different signal
  from confirmation (`qsoStatusOf`, CONF only for LoTW/QRZ.com
  confirmation). Desktop's `qsoStatusLabel` already matched the CONF logic
  exactly, but a previous round (2026-11-03) deliberately left the sync
  chips out, reasoning the QRZ tag "read as a confirmation when it only
  meant uploaded." The user's direct call tonight was to match Android
  exactly instead, so they're back: new `qsoSyncLabels(r)` in `logbook.ts`
  (`synced_cloudlog` -> "CL", `synced_qrz` -> "QRZ", in that order,
  matching Android's fixed order -- no WRL chip, desktop has no World Radio
  League upload tracking to port that from), rendered in `App.tsx`'s
  Status cell next to the CONF pill, styled as a small bordered chip
  (`.sync-chip` in `styles.css`) using the exact same Signal-color
  alphas as Android's `SyncChip` composable. Not visually confirmed live
  with real data -- no row in the current database has either flag set
  (desktop has no live Cloudlog/QRZ upload-sync feature yet to ever set
  them, a separate, already-documented gap), and directly editing the
  production database to manufacture a test case was (correctly) blocked
  as a destructive action. Covered by new unit tests instead
  (`qsoSyncLabels` x5 in `logbook.test.ts`).
- **QSL "sent" mark was the wrong color.** Android's `QslMarksText` uses
  `StatusConfirmed` (green) for a received mark, `Signal` (cyan) for a
  sent-only mark, `TextFaint` (dim) for neither. Desktop's `.qsl-mark.sent`
  was `var(--accent)` (amber) -- confirmed live after the fix, now
  `var(--signal)` (cyan), matching exactly.
- **Column header text**: "Date"/"Time"/"Call" -> "QSO\nDate"/"QSO\nTime"/
  "Callsign", matching Android's actual string resources
  (`strings_compose.xml`'s `log_col_qso_date`/`log_col_qso_time`/
  `log_col_callsign`) exactly -- the other 8 header labels already matched.

**Confirmed already matching, not changed**: the sorted-column header
highlight color (`AccentSoft`/`Accent` in Android, `--accent-soft`/
`--accent` on desktop -- the android.png screenshot's compression made
this look like a different grey-brown color at a glance, but the source
confirms both use the same amber-family token).

**Round 2, after "the display looks nowhere like the android display"**:
the first pass above was real but too shallow -- logic/data fixes without
touching the table's actual visual structure, which had never been styled
to match Android's grid-like look in the first place (confirmed: no base
`table`/`th`/`td` rule existed at all; only a few specific exceptions like
`.call-cell`). Read Android's `QsoTableHeader`/`QsoTableRow`/`QsoTableCell`
in full and ported the visual structure itself, not just colors:

- **Per-cell borders.** Android draws a `0.5dp` border around *every* cell,
  header and data alike (`QsoTableCell`'s own `.border(0.5.dp, Border)`) --
  that's what gives the grid-like, boxed-header look, not a header-only
  style. Desktop had no cell borders at all. Added `.qso-table th, .qso-table
  td { border: 1px solid var(--border); }`.
- **Monospace font.** Android uses `GeistMonoFamily` ("Geist Mono") for
  the whole table, "kept for tabular data columns... whose numeric/column
  alignment relies on Geist Mono's fixed advance widths." Desktop already
  had a `.mono` utility class with the same font stack (used elsewhere,
  e.g. the waterfall) but the Logbook table never used it. Applied directly
  to `.qso-table`.
- **Column widths.** Android weights each column (`QsoTableWeights =
  [1.02, 0.58, 0.95, 0.62, 0.75, 0.68, 0.75, 0.55, 0.55, 0.62, 0.98]`,
  `LogbookScreen.kt`); desktop's columns were all equal width. Ported the
  exact same weights as a new `QSO_TABLE_WEIGHTS` export (`logbook.ts`),
  applied via a `<colgroup>` of percentage widths (`table-layout: fixed`
  for them to take effect) instead of letting the browser auto-size by
  content.
- **Per-column text colors.** Android's `QsoTableRow` colors each cell
  individually (Grid = `Signal`/cyan, Call = `Accent`/bold -- already had
  this one, State/place = `TextMuted`). Desktop only had the Call-cell
  color; added `.grid-cell` (signal) and a `muted` class on the State cell.
- **Sorted-header color, a real bug, not just a shade difference.**
  Desktop's `.qso-table th.sorted` used `var(--yellow)` -- Android's
  `QsoTableHeader` uses `Accent`/`AccentSoft` for this, a role `--yellow`
  (Android's own `StatusWarn`) never actually plays. Fixed to
  `var(--accent)`/`var(--accent-soft)`.
- **Row height** bumped from 26px to 32px (`ROW_HEIGHT` in `App.tsx`,
  kept in sync with the CSS as the row-virtualization code requires) to
  fit the real padding (7px vertical, matching Android's `7.dp`) instead
  of a tight single-line squeeze -- this is what reads as Android's
  "noticeably taller rows" from a straight screenshot comparison, it just
  needed the real padding Android actually uses, not an arbitrary bump.
- **The missing Stats / Recent / Awards row.** Added the three-way tab row
  itself (`.log-subtabs`/`.log-subtab`, a segmented control matching the
  screenshot) -- "Recent" is the only functional one (today's table);
  Stats and Awards show an honest `ComingSoonScreen` placeholder rather
  than fake content, since those are real, separate, much larger bodies of
  work (VUCC/WAS/WAC award tracking, a "Best DX" card), not a display fix.
  This was flagged as out-of-scope-for-tonight in round 1 of this parity
  pass; the user's follow-up made clear the *tab row itself* (even empty)
  was still a visible, expected part of "identical," so it's back as a
  real but honestly-labeled part of the screen.

**Still genuinely out of scope** (separate features, not display): Stats'
and Awards' actual content (as above); a real per-band filter/indicator to
match Android's "· All bands" subtitle (would need new filtering logic,
not just a label); the search box's magnifying-glass/clear-button icons
(cosmetic, deferred, not requested again after round 1).

**Testing**: 177 vitest tests (+5 net: `qsoSyncLabels`, 5 cases). No Rust
changes this round -- everything in both rounds was TS/CSS. Verified live
both rounds, not just by reading the diff; memory stayed flat (~2.6GB
available) through both rebuild-and-relaunch cycles.

## Done this round, part 13: Logbook top bar, three-dot actions menu, and a real scroll bug

**Top bar restructure.** The user's own screenshot comparison also showed
Android's Logbook screen does *not* share Decode's full "Call CQ / Hunt /
Tune / More" operate bar -- confirmed in `FT8AFApp.kt`: when
`statusOnTop` (`isLandscape && controlsDrawerEnabled && !actionsOnDecodeTitle`)
holds, `LogbookScreen.kt`'s own `TopBar` call uses a `titleContent` slot
("Logbook" title + "{count} QSOs · All bands" subtitle in one row, `tight =
true`) with just the RX/TX/radio-status chips in `actions` -- the full
operate-bar buttons live in a separate collapsible drawer not shown on this
screen at all. The user confirmed this live, running the real Android app on
their phone: switching to Decode restores the full bar, confirming the
behavior is scoped to Logbook only. Ported as a new `LogbookTopBar`
component in `App.tsx` (fetches its own `count` via `api.logCount()` since
it's a sibling of `LogScreen`, not a parent), reusing two pieces extracted
out of the existing `TopBar`: `RxTxRadioChips` (the RX/TX dot + radio-link
pill) and `SyncRow` (the clock-sync badge + cycle bar). Swapped in at the
`tab === "log"` branch of the top-level render.

**Three-dot actions menu.** Matching `QsoActionsMenu` in `LogbookScreen.kt`
exactly: a "⋮" (MoreVert) button opening Edit/Delete, replacing the old lone
"✕" delete button -- and critically, Android's `QsoTableRow`'s cells have
**no** `onClick` at all (confirmed: every `QsoTableCell(...)` call omits it),
so Edit is reachable *only* through this menu, never by clicking a row.
Ported the same way: removed the per-cell/per-row click-to-edit, added
`QsoActionsMenu` with Edit (Accent-colored) and Delete (red) items.

The dropdown itself renders through a React portal into `document.body`
(`createPortal`), positioned from the trigger button's
`getBoundingClientRect()`, rather than as a normal absolutely-positioned
child of the table cell -- the table's scroll container clips any
descendant positioned relative to an ancestor inside it, so a plain
absolute-positioned dropdown would get cut off for any row not right at the
bottom of the visible area.

**A real, pre-existing scroll bug, found verifying the above.** The user
reported live: "the log display is not scrollable. I have a touch screen
display and should be able to scroll down the list with either my mouse or
using the touch screen." Root cause: `.decode-list` only gets
`flex: 1; overflow: auto; min-height: 0` when nested inside `.decode-detail`
(`styles.css`) -- that's Decode's split-pane wrapper. The Logbook screen's
`.decode-list` div sits directly inside `.content.decode`
(`overflow: hidden; display: flex; flex-direction: column`) with no
`.decode-detail` wrapper around it, so it never grew to fill the remaining
space or became its own scroll container -- the ancestor's `overflow: hidden`
just clipped it outright, silently, with no scrollbar and nothing to scroll
by any input method. Fixed with a second, direct rule (`.log-table-list`,
added as an extra class alongside `.decode-list` on that div) giving it the
same `flex: 1; overflow: auto; min-height: 0` independent of the
`.decode-detail` nesting.

Verifying that fix live (mouse-wheel scroll via `xdotool`) surfaced one more
real issue: the native overlay scrollbar's hoverable hit zone is wider than
its thin painted thumb, and it was sitting directly on top of the actions
column's "⋮" trigger at the table's right edge -- clicking the trigger
there was landing on the scrollbar instead (confirmed: it jumped the scroll
position, and the screenshot taken immediately after briefly caught a
mid-repaint blank frame, not an app crash). Fixed by reserving a
`padding-right: 14px` gutter on `.log-table-list`, so the table's own
content (and the "⋮" column) stops short of the real edge and the overlay
scrollbar floats over blank space instead of over a clickable column.

**Testing**: 177 vitest tests (unchanged count -- all three changes are
layout/interaction, not logic covered by new pure-function tests). Verified
live end to end after a real rebuild+relaunch: Logbook's top bar now shows
the lean title/subtitle/status row (confirmed against the user's live
Android phone session); mouse-wheel scroll moves the table through all
45,622 rows; the "⋮" menu opens without hitting the scrollbar, Edit opens
the real edit dialog pre-filled, Cancel and Delete both still work.

## Done this round, part 14: real Stats/Awards, a correct column sort, QRZ sync flags, and three real Decode-row bugs found by live screenshot comparison

**Logbook Stats/Awards tabs are real now**, replacing the honest
`ComingSoonScreen` placeholders: big stat cards (Total QSOs/DXCC/CQ
zones/ITU zones), a Best DX card, Worked All Continents, a band donut
chart, award progress bars, a grid-coverage heatmap, and a signal-trend
sparkline (synthesized on both platforms -- Android's own `SignalSparkline`
doc comment says the same; ported faithfully, not something desktop is
faking that Android has real), plus six award cards on Awards (DXCC
Mixed/WAS/WAZ/WAC/VUCC/IOTA). Backend: `dxcc.rs` now keeps ITU zone and
continent from `cty.dat` (previously discarded); `util.rs` gained
`maidenhead_to_latlon`/`haversine_km` (ported from the frontend's own
`hunt.ts` math) so Best DX can be computed entirely in Rust over the whole
log, never shipping tens of thousands of rows to the frontend to reduce
(the same mistake `LOG_FETCH_LIMIT` exists to prevent); `db.rs` gained
`worked_itu_zones`/`worked_continents`/`worked_states_all`/
`grid_squares_worked`/`worked_grid_fields`/`band_counts`/`total_qsos`/
`best_dx`, aggregated by a new `Db::log_stats`. Verified live against the
real 45,622-row log -- which caught a real bug: `worked_states_all`
resolves via `us_states::worked_state_for`, which also recognizes "DC", so
the WAS award showed "54 / 50" worked states. Added `wasStatesWorked`
(`logbook.ts`) to filter to the real 50 WAS states before counting,
matching Android's own `workedStates` reducer's filtering step, which this
port had initially missed.

**A column sort now runs in SQL before `LIMIT` truncates the result**
(`db.rs`'s new `qso_sort_order_by`/`Db::search_qsos`), instead of the
frontend re-sorting whatever page had already been fetched. Confirmed as a
real bug, user-reported: with `LOG_FETCH_LIMIT` capping the fetch to 300
rows, sorting by any column could only ever reorder the newest 300 QSOs --
a callsign that was alphabetically first across the whole 45,622-row log
but logged years before the most recent 300 could never surface. Sorting
in SQLite (bounded Rust-side memory, never loaded into the frontend)
before `LIMIT` applies fixes this without reintroducing the
WebKitWebProcess memory blowup `LOG_FETCH_LIMIT` exists to prevent.
`logbook.ts`'s old `sortQsoTable` (a pure client-side sort of one page) is
removed along with its now-superseded tests; equivalent coverage now
lives in `db.rs`.

**ADIF import actually reads QRZ/Cloudlog upload-sync status now.**
`adif_record_to_qso` hardcoded `synced_cloudlog`/`synced_qrz` to `false`
on every imported record regardless of what the file said -- a real log
exported from Android (which writes the standard
`QRZCOM_QSO_UPLOAD_STATUS=Y`, or carries a QRZ.com logbook export's own
`APP_QRZLOG_LOGID`, and its own `APP_FT8AF_SYNCED_CLOUDLOG=Y` extension
field for Cloudlog/Wavelog/Nextlog) silently lost that information on
import, so the Logbook's "QRZ"/"CL" sync chips (`qsoSyncLabels`) could
never appear for any imported row. Ported the exact field-reading logic
from Android's `QSLRecord.java`. Also fixed `merge_matched_qso` (the
duplicate-match path a re-import takes) to backfill these two flags onto
an already-logged QSO the same way it already did for
confirmed/qsl_sent/lotw/eqsl/qrz_qsl -- without this, a QSO logged before
this fix existed would stay stuck at `false` forever, even after
re-importing a file that now parses the field correctly.

**Worked-before now uses the live operating mode.** `db.rs`'s
`worked_calls`/`worked_states` already scope "worked" to
station+band+mode correctly, but the frontend
(`refreshWorkedCalls`/`refreshWorkedCallsAnyBand`/`refreshWorkedStates`)
passed the literal string `"FT8"` regardless of `operatingMode`, and never
re-fetched on a mode change -- so operating in FT4 or FT2 still queried
(and highlighted) as if on FT8. The stale comment blaming this on "desktop
only ever logs FT8" no longer matched `qso.rs`: `QsoEngine.mode` is set
live from the real operating mode (same mechanism as `band`/`freq_mhz`),
not hardcoded. Fixed to pass `operatingMode` and added it to each effect's
dependency array.

**Three real bugs in the Decode row, found by a close live screenshot
comparison the user asked for, then confirmed against a real Android
screenshot they sent:**

- **Freq wasn't zero-padded and DT could render a signed zero.** Android's
  `getFreq_hz()` always zero-pads to 4 digits (`"%04.0f"` -> "0523", not
  "523") so the Freq column's digits line up regardless of how many digits
  the offset has; desktop showed the bare rounded number. `formatDecodeDt`
  explicitly renders a value that rounds to zero unsigned ("0.0", never
  "+0.0"/"-0.0" -- a signed zero is noise when scanning a column for real
  outliers); desktop's inline `sign >= 0 ? "+" : ""` always added a sign.
  Ported both into a new `decodeRow.ts` (from Android's `DecodeDt.kt` and
  `Ft8Message.java`'s `getFreq_hz`), with tests -- including one documenting
  that Java's and JS's `Math.round` both round a negative `.5` tie toward
  +Infinity, not away from zero, so `-0.05` renders unsigned "0.0" on both
  platforms; that's Android's real behavior, not an edge case to "fix".
- **A column-header row Android never had.** Desktop showed a persistent
  "MESSAGE / SNR / FREQ / ΔT" label row above the Decode list -- a previous
  round added it without verifying against a real screenshot. Confirmed
  against a real Android screenshot the user sent, and against
  `DecodeScreen.kt`/`DecodeRow.kt`'s actual source (no such composable
  exists there): Android's compact list goes straight from the filter
  chips to the per-cycle time divider, with no persistent header above it.
  Removed.
- **A real flexbox alignment bug**, found investigating the header issue:
  `.decode-row-snr`/`.decode-row-freq`/`.decode-row-dt` never had
  `flex: none`/`min-width: 0`, so their fixed `width`/`text-align: right`
  could be silently overridden by flexbox's default item sizing --
  confirmed live, values drifted left of where they should right-align
  relative to the (also-just-fixed) header labels. `.decode-row-msg` had
  already needed the identical fix earlier this round for its own
  `width: 30ch` (a long message was pushing every column after it out of
  alignment, since each row's message is a different length) -- same root
  cause, same fix, applied to all four cells consistently now.

**The Decode screen's message panel was widened** (`.decode-detail`/
`.decode-waterfall`, 50/50 -> 55/45): at an even split the message panel's
own row content (30ch message + SNR/FREQ/ΔT + a status pill like "NEW
BAND") is wider than the panel, clipping the pill's last letter --
confirmed live ("cuts off the D from New Band").

**The Logbook table's sticky header needed two attempts.**
`position: sticky` on a `<th>` turned out to be unreliable in WebKitGTK
regardless of `border-collapse` mode -- confirmed live, twice, with two
different fixes attempted (first just adding sticky positioning, then
switching `border-collapse: collapse` to `separate`): the header text and
the row scrolling underneath it painted on top of each other instead of
the header cleanly covering it. Replaced with the standard, browser-quirk-
free fix: the header is now its own small `<table>` (sharing the same
`<colgroup>` so columns still line up) rendered outside the scrolling
container -- the same pattern `.decode-row-header` used before this
round's Decode-row cleanup removed that element.

**Testing**: 195 vitest tests, 309 cargo tests. Verified live end to end
after real rebuild+relaunch cycles throughout -- the Stats/Awards tabs,
the sticky header (confirmed clean with a zoomed-in pixel crop after each
attempt), the full-log sort, the Decode row fixes (compared pixel-by-pixel
against both a desktop screenshot and a real Android screenshot the user
sent), and the panel-width fix.

**The QRZ/Cloudlog sync-flag fix's end-to-end verification turned into its
own investigation.** Re-importing a real ~54,500-record QRZ.com export
into the live 45,622-row log, to confirm `synced_qrz` gets backfilled via
`merge_matched_qso` and the "QRZ" chip appears, ran for many minutes
against the real log with `synced_qrz` staying at 0 the whole time. The
user asked to check the dedupe logic itself for the same "taking too long"
class of bug Android had and fixed (`ImportSharedLogs.java` commit
`319829f7`, an ANR on a real ~53,000-QSO log, fixed by
`DatabaseOpr#yieldImportBatch` calling SQLite's `yieldIfContendedSafely()`
after nearly every record). That surfaced two real bugs, one performance
and one correctness -- in that order, since the performance one was found
first:

- **`self.conn`'s Mutex was locked once before the import loop and held
  for the *entire* import**, including across every `COMMIT;BEGIN` batch
  boundary (the SQL-level transaction cycling never released the Rust-
  level guard) -- every other backend command needing the same connection
  (`log_count`, `search_qsos`, `worked_calls`, ...) blocked for the whole
  import, not one batch. Confirmed live: the Logbook screen showed a
  stuck/stale QSO count throughout. The indexes needed for fast duplicate
  matching were already ported and confirmed present and in use
  (`qso_log_import_dedup_idx`/`qso_log_lotw_twin_idx`, schema v7, verified
  with `EXPLAIN QUERY PLAN`), so this wasn't a second missing index. Fixed
  by dropping and re-acquiring the lock after every record (cheap for
  `parking_lot`'s `Mutex` when uncontended; its eventual-fairness
  guarantee hands the lock directly to a genuinely waiting thread, the
  closest equivalent available to `yieldIfContendedSafely()`), proven with
  a real concurrency test racing a background thread's `total_qsos()`
  against an in-flight import. Also switched every hot-path statement the
  import loop calls per record to `prepare_cached` (rusqlite's `Connection`
  has no implicit statement cache the way Android's `SQLiteDatabase` does,
  so the same query text was being re-parsed and re-planned from scratch
  on every one of 54,554 records).
- **The real bug, found by actually reading the imported rows back**:
  `insert_qso_locked`'s `INSERT` and `update_flags_locked`'s `UPDATE`
  statements both omitted the `synced_cloudlog`/`synced_qrz` columns
  entirely. `adif_record_to_qso` correctly parsed
  `QRZCOM_QSO_UPLOAD_STATUS`/`APP_FT8AF_SYNCED_CLOUDLOG` into the
  in-memory `QsoRecord`, and `merge_matched_qso` correctly carried both
  flags into a merged record (its own unit test proved that) -- but
  neither write path ever bound either value to a SQL parameter, so both
  were silently dropped before ever reaching the database, on *every*
  import, not just this one. The two earlier commits fixing the parsing
  and the merge logic were real but incomplete: neither actually closed
  the loop to disk. This is exactly why the first verification attempt
  could never have succeeded no matter how long it ran, and why it needed
  a fix beyond the performance one. Fixed both write paths, with new tests
  that read the row back from the database afterward (not just checking
  the in-memory merge result, which is the gap that let it through the
  first time): one for a fresh insert, one for the existing-row backfill
  via the duplicate-match path.

## Done this round, part 15: decoder suspended during import, and a stale-cache "New DXCC on US stations" bug

**The decoder now suspends itself during a bulk ADIF import**, freeing up
CPU on this board instead of fighting the always-on FT8 decode engine for
it. Confirmed live, same code and data: the full 54,554-record QRZ file
took 10+ minutes inside the running app but 34s in an isolated benchmark
with the decoder idle -- the gap was CPU contention, not a remaining logic
bug in the import path itself (that path's own correctness bugs -- the
mutex-yield and `synced_qrz`/`synced_cloudlog` write-path fixes -- are part
14, above, and are unaffected by this). `engine.rs` gained
`EngineCommand::SetDecodeSuspended(bool)` and the pure, tested
`decode_dispatch_due(decoding, suspended, ...)` gate (extracted from what
was previously an inline condition in the run loop, following the same
pattern as `waterfall_row_due`/`boundary_tx_ready`). This is deliberately
*not* the same thing as the existing `StopDecode`: it only skips handing a
slot to the decoder thread -- audio capture, the live waterfall, UDP, and
`self.decoding` itself are all untouched, so nothing about the running app
visibly changes except the CPU the decoder would have spent. `main.rs`'s
`import_adif` command sends `SetDecodeSuspended(true)` before the import
and resumes it via a `DecodeSuspendGuard` (`Drop`), so it resumes even on
an error or a caught panic.

**A real production-database round-trip proved both fixes together,
cleanly, for the first time.** Wiped the real log to 0 rows (backed up
first), rebuilt with both fixes in place, and imported the full QRZ file
(54,554 records) followed immediately by a full CQRLOG export of the same
station's history (54,515 records) through the running app's own Import
ADIF UI -- not a side-channel script, per standing project practice. QRZ
import: 45,588 added / 8,966 updated (intra-file duplicates) / 0 skipped,
~85s, with the sync clock and TX countdown visibly still ticking the whole
time -- the live-responsiveness proof the isolated benchmark alone
couldn't give. CQRLOG import: 36 added / 54,479 updated / 0 skipped --
almost the entire second file matched and merged into the first import's
own records rather than duplicating, a strong end-to-end confirmation that
cross-source dedup (two different exporters, different field order/casing)
works correctly. Final count (45,624) exactly matched the original
pre-wipe log, with `synced_qrz=1` correctly set on the 45,588 QRZ-sourced
rows and correctly absent on the 36 CQRLOG-only ones.

**That same test run surfaced a real, separate bug**: every single Decode
screen row showed "NEW DXCC" after the import finished, including obvious
already-logged US stations. Not a DXCC-logic bug -- a stale-cache one.
`workedDxccPrefixes`/`workedGrids`/`workedPrefixes`/`workedCqZones`
(`App.tsx`) are each fetched exactly once, on mount (`useEffect(fn, [])`),
and otherwise only refreshed by the live `qso_completed` engine event fired
per logged QSO; `workedCalls`/`workedCallsAnyBand`/`workedStates` at least
also refresh on a band/mode change. A bulk ADIF import goes through none
of that -- it's a single Tauri command writing straight to the database,
with no `qso_completed` event per record or otherwise -- so after any
import, every one of these "worked" sets silently keeps whatever it was
when the app launched until the operator logs one live QSO or changes
band/mode. On a freshly-launched app with an empty log (exactly this
test's setup), that means every entity/grid/prefix/zone looks "new" to the
Decode screen regardless of what the import just added. Fixed by having
`startImportAdif` call the same seven refresh functions the
`qso_completed` handler already calls, right after a successful import.

## Build note

`npm run tauri -- build --no-bundle` is the only way to get a real,
launchable binary (plain `cargo build` skips `beforeBuildCommand`, producing
a dev-mode binary missing the `custom-protocol` feature). A full release
build on this board took ~36 minutes on the first build after this round's
dependency churn; expect faster incremental builds after that.

## Done this round, part 16: the real GridTracker tile basemap (phase 1 of an Android Map-screen port)

The long-standing gap noted below (several rounds: "GridTracker's real
look on Android is actual offline satellite tile imagery, not a flat
vector map") is now closed. This also turned out to be phase 1 of a much
larger piece of work: a fresh look at Android's current `dev` branch
found its Map screen has grown into a ~5,000-line feature (a tile-based
"Tracker" mode, a vector "Azimuthal" radar projection, and a full PSK
Reporter network overlay) -- far more than the single tile-basemap item
below ever covered. The user chose to sequence all of it, starting here;
PSK Reporter, the Azimuthal projection, and remaining polish (state
borders/labels, connection lines, signal-reach card) are tracked as a
roadmap in the current plan file, to be designed in detail each in turn.

**The previous "approved plan" referenced below no longer existed** --
the plan-file slug had been overwritten by an unrelated later session
(the ADIF-import streaming fix). Its one surviving claim worth
correcting: it said "standard Web Mercator XYZ numbering." That's wrong.
Extracting and visually inspecting real tiles from the bundled archive
this round (z=0's single tile shows an undistorted world -- a
normal-sized Greenland, a uniform Antarctic strip -- which real Web
Mercator's high-latitude stretching would never produce) confirms the
tile set is plain **equirectangular**. The key formula itself
(`key = z*4^z + x*2^z + y`) was already right, re-derived independently
and confirmed by extracting specific z=2 tiles and checking their content
against known geography: (0,0) is the Canadian Arctic archipelago, (0,1)
is North America (confirms +y = south), (1,0) is Greenland (confirms +x
= east), (3,3) is Antarctica (the far SE corner).

Because the tiles are equirectangular, desktop's existing
`mapProjection.ts`/`mapViewport.ts` placed them directly -- no Mercator
projection module was needed after all. New `desktop/src-tauri/src/
map_tiles.rs` embeds the already-bundled `resources/nightUSGS4Layer.sqlite`
(4.6MB) via `include_bytes!`, writes it to the app data dir once on first
use (rusqlite needs a real file path; its `serialize`/`deserialize`
feature turned out to need unsafe FFI buffer-copy code to populate from
embedded bytes, so this is both simpler and matches the project's
existing `include_str!`/write-once-to-disk resource pattern), and serves
tiles over a new `map_tile(z,x,y)` command. That command returns a raw
`tauri::ipc::Response` rather than a base64 `data:` URL -- confirmed this
needs zero new Cargo dependencies (base64 was only ever a transitive
dependency) and that `createImageBitmap(Blob)` decodes the raw bytes
without navigating a URL at all, bypassing the page's `img-src` CSP
entirely rather than needing it loosened. `MapScreen`'s draw loop now
layers decoded tile bitmaps over the existing vector land/country
outlines (which stay as the permanent placeholder for anything not yet
loaded, not a special-cased fallback), with an in-flight-promise cache so
a pan/zoom drag's rapid redraws don't fire duplicate tile fetches.

## Done this round, part 17: Map header row -- layer chips, gray line toggle, a real PSK Reporter overlay (phase 2 of the Map-screen port)

Android's Map header has five things: Decodes/PSK spots/Worked layer
chips, a Tracker toggle, and a settings gear. The user asked for the
full row with real logic, not placeholders, but explicitly deferred
**Tracker to last** -- it switches between two *different*
marker-rendering systems on Android (`GridOsmMapView`'s own native
overlays vs. the app's `drawStationLayers` system with layer chips/PSK/
connection lines), and desktop only has the first kind (phase 1's tiles
+ `gridTrackerSlot.ts`'s markers) -- building a second rendering mode
just to give Tracker something to toggle to is real, separate work,
tracked in the current plan file's roadmap. No settings gear either:
its only desktop-applicable item (gray line) already gets its own
direct chip, and a dropdown with one real entry plus two N/A ones
(Projection, Grid tracker full screen -- neither applies without
phases 3/Tracker) wasn't worth adding.

**Decodes/Worked layer chips**: `isStationVisible(isWorked, decodesOn,
workedOn)` (ported from `MapRedesign.kt`, now in `gridTrackerSlot.ts`)
partitions every CQ marker by worked status rather than being
independently additive -- a worked station's visibility is controlled
only by the Worked chip, an unworked one only by Decodes. Defaults
match Android exactly: Decodes on, Worked off.

**Gray line toggle**: the terminator draw block (already correct,
phase-1-untouched) is now wrapped in `if (grayLineEnabled)`, persisted
via `get_config`/`set_config` -- the same generic `"true"`/`"false"`
string round-trip `wf_denoise`/`location_service_enabled` already use,
no new Rust needed.

**PSK Reporter spot overlay -- a real new network client, not a
stub.** New `desktop/src-tauri/src/psk_reporter.rs` ports
`PskReporterClient.kt`, adapted rather than transliterated:

- Confirmed via the real fixture (`ft8af/app/src/test/resources/
  pskreporter/reception-report.xml` on `origin/dev`) that the root
  element is `<receptionReports>` with `<activeReceiver .../>` siblings
  to ignore, and that an attribute present but empty (`receiverLocator=
  ""`) counts as missing, same as an absent one.
- **A real bug caught and fixed before it ever ran**: the first draft
  always plotted the *sender's* callsign/grid as a spot's identity.
  That's backwards for the default "who heard me" direction
  (`by_receiver=false`, query `senderCallsign=<me>`) -- in every
  returned row *we* are the sender, so the station worth plotting is
  the *receiver* (who actually heard us). Fixed by threading
  `by_receiver` into `parse_spots` and picking the correct side; caught
  by writing the test against the real fixture's values, not by
  inspection.
- **Band resolution reuses existing Rust code, not a new table**:
  rather than port Android's `Band` enum/range-table to TypeScript,
  each spot's band is resolved server-side via the already-existing
  `bands::band_for_freq_hz` (the same table `db.rs`'s `worked_calls`/
  `worked_states` use) -- `pskFilter.ts` on the frontend just does a
  plain string compare against the resolved band name.
- **Lat/lon stays a frontend concern**, same decision as phase 1's
  tiles: spots carry a raw `grid` string; the frontend's existing
  `gridToLatLon` (`hunt.ts`, already robust against malformed input)
  converts it, no second maidenhead implementation in Rust.
- **Rate-limit back-off is checked before cooldown, unconditionally**
  (a 429/503 response's 15-minute lockout can't be bypassed by
  `force`; the 290s cooldown can). Extracted a pure `fetch_gate`
  function (mirroring `engine.rs`'s own `decode_dispatch_due`/
  `boundary_tx_ready` pattern) instead of porting Android's injectable-
  clock test seam -- plain `i64` millis need no mocking to unit-test.
- `quick-xml` promoted from a transitive dependency (pulled in via
  `plist`, Tauri's own macOS bundling tooling) to a direct one --
  confirmed via `cargo add --dry-run` this resolves to the exact
  already-locked `0.39.4`, no new version, no new sub-dependency tree.
- The command returns `{ spots, error }` in one IPC round trip rather
  than mirroring Android's separate nullable-return + `lastError`
  property read, since that split only exists there because it's two
  in-process reads -- over IPC it'd be an extra call for no reason.

**Verified live**, including against real PSK Reporter data: toggling
each chip visibly changes the map (Decodes/Worked correctly partition
which markers show, confirmed by a worked-station marker appearing
only once the Worked chip was turned on; Gray line removes/restores the
terminator). Enabling PSK spots correctly reveals the Filter chip and
runs a fetch with no error -- cross-checked with a direct `curl` to
`retrieve.pskreporter.info` for the same callsign/window, confirming
the real response is genuinely empty (this station has no reception
reports in the window queried), not a silent failure. The filter
sheet's Time/Mode/Direction chips render and update state correctly,
triggering a refetch on change.

## Done this round, part 18: Tracker toggle (phase 3 of the Map-screen port, roadmap complete for now)

Last item from the user's own sequencing ("leave tracker until last b[ut]
we will also implement it"). Confirmed before writing any code that
Android's Tracker toggle is a data-source swap, not a second renderer:
Tracker on (`GridTrackerPanel`/`GridOsmMapView`, already desktop's whole
Map screen since phase 1) shows only the current decode slot, cleared
every cycle; Tracker off (`MapScreen.kt`'s own `stations` builder) keeps
one marker per callsign ever decoded, with lines only for two specific
things via `ConnectionLines.kt` -- a solid line to the live TX target,
dashed lines from anyone calling the operator -- not general QSO-exchange
lines between arbitrary pairs. This meant phase 3 needed no Azimuthal
projection and no `drawStationLayers`-style rewrite, simplifying it well
below this file's own earlier "two full rendering systems" framing.

New `desktop/src/stationAccumulator.ts` (`buildAccumulatedStations`) and
`desktop/src/connectionLines.ts` (`buildConnectionLines`, port of
`ConnectionLines.kt`) are both pure, feeding the exact same
`drawCqMarker`/`isStationVisible` drawing phases 1-2 already built --
only the data source changes on toggle. Confirmed first (not assumed)
that `App.tsx`'s decoded-message state is newest-first
(`[...fresh, ...prev]`), so "first occurrence per callsign" is already
"latest wins," no reversal needed; and that the live TX target
(`txState.status.target`, already real frontend state from
`QsoStatus.target` in `qso.rs`/`engine.rs`) simply wasn't threaded past
`App.tsx` to `MapScreen` yet -- added as a plain prop, no backend changes
this phase at all.

One pre-existing limitation knowingly inherited, not introduced:
`MapScreen`'s `messages` prop is already the Decode screen's filtered
list (whatever chip is active, `All` by default), not a guaranteed-
unfiltered history -- Tracker-on already lived with this; Tracker-off
does too.

Verified live against real decode traffic: Tracker on behaves exactly as
before ("N CQ · N QSO this cycle", clearing every ~15s). Toggling off
immediately switches the counter to "N stations tracked" and the count
was observed growing across real cycles without resetting (25 -> 50
tracked stations over two checks), the definitive proof the accumulator
persists rather than re-deriving from the current slot. Toggling back on
correctly reverted to slot-scoped behavior.

This closes the three phases the user asked to sequence (tile basemap,
header chips/PSK Reporter, Tracker). Azimuthal projection and the
remaining polish items (state borders/labels, marker SNR/selection
styling, a richer selected-station card) are still on the roadmap above,
not yet requested to start.
