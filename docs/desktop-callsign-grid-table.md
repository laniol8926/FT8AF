# Desktop callsign → grid cache (`callsign_qth`)

A persistent callsign → Maidenhead-grid lookup table, needed so a future Map
screen can show where any previously-heard station is, not just ones mid-QSO
right now — a station's grid only arrives on specific FT8 message types
during a live exchange. Android already solves this with its own
`CallsignQTH` table (`DatabaseOpr.java`/`MainViewModel.java`); desktop had no
equivalent until now. New schema v7 table: `callsign_qth(callsign TEXT
PRIMARY KEY, grid TEXT NOT NULL, updated_at INTEGER NOT NULL)` — matches
Android's own table shape exactly, no extra columns.

## Resolved: the first-run seed is now narrated by a native OS popup

**The problem** (found while verifying this live): a real, reproducible
WebKitGTK bug on this board means the main window can stop repainting almost
immediately after launch — not the clock, not the sync readout, not the
in-app progress dialog, nothing visually updates again until a real mouse
click forces a repaint. A first-time user watching the ~200MB download +
1.7M-record scan happen behind a silently frozen window would reasonably
conclude the app had hung and kill it. Tried and ruled out as fixes:
`WEBKIT_DISABLE_DMABUF_RENDERER=1` (already set in `run-ft8af.sh` for a
related but distinct freeze — see that script's own comment — does not
prevent this one), `WEBKIT_DISABLE_COMPOSITING_MODE=1`, `GDK_BACKEND=x11`,
`vblank_mode=0`, and combinations of all four — the underlying repaint bug
itself is still unfixed and tracked separately.

**The fix**: sidestep it rather than chase it. `main.rs`'s `setup()` now
triggers the first-run seed itself (not the frontend's mount effect — see
below) and shows a **native GTK message dialog** (`tauri-plugin-dialog`,
backed by `rfd` — a genuinely separate OS window, not webview content) with
"Creating US Grid Table" before the download starts. Native dialogs render
through a completely different pipeline than the webview, so they stay
visible and responsive regardless of the repaint bug — confirmed directly:
`wmctrl -l` lists it as its own top-level window (`Creating US Grid Table`)
alongside the frozen `FT8AF` one, and it persists correctly for as long as
it isn't dismissed. Only the *starting* popup is native; a native
completion/error popup was tried too and then deliberately removed after
live testing showed it was redundant with the in-app completion dialog
(`FccGridSeedCompleteDialog`) — one "it's done" notification is enough once
the operator already knows the app is alive.

`run_fcc_grid_seed` (shared by both the backend auto-trigger and the manual
`import_fcc_grid_seed` command) emits `fcc-grid-progress` throughout and a
new `fcc-grid-done` event (`{ok: FccSeedSummary} | {err: string}`) at the
end regardless of which trigger started it, so the in-app dialog still works
correctly whenever the window *is* repainting — App.tsx now listens for both
passively for its whole lifetime instead of only while a manually-triggered
seed is in flight, which is also what lets a Settings-panel-triggered
manual re-run (`CallsignGridPanel`, "Update now") share the exact same UI
path as the first-run auto-start.

**Verified idempotent**: two back-to-back real runs — first with an empty
table (native popup appears, seed completes, 806,668 rows), second
immediately after with the table already populated (no popup, zero seed
activity in the log, row count unchanged) — confirm `callsign_qth_count() !=
0` correctly skips re-seeding on every subsequent launch.

## Populated four ways

1. **First-run FCC/Census bulk seed** (new module `src-tauri/src/fcc_grid.rs`).
   The FCC's amateur bulk dump (`l_amat.zip`,
   `data.fcc.gov/download/pub/uls/complete/l_amat.zip`) has name/address/ZIP
   per callsign but **no lat/lon** — amateur licenses carry no filed antenna
   coordinates, unlike commercial/repeater licenses. Fix: cross-reference
   the ZIP against a bundled Census Bureau ZCTA Gazetteer file (ZIP → lat/lon
   centroid, `resources/zcta_gazetteer.txt`, trimmed to `GEOID|INTPTLAT|
   INTPTLONG` from the real 2026 national file, confirmed against
   census.gov's own record-layout docs), then convert with the
   already-existing `util::latlon_to_maidenhead(lat, lon, 4)`. ZCTA-centroid
   accuracy is well inside a grid4 cell (~1°×2°).

   Real-file field positions, confirmed by actually downloading and
   inspecting `l_amat.zip` (not guessed from FCC's own definitions PDF,
   which blocks automated fetches): `EN.dat` is pipe-delimited with Call
   Sign at field 4, ZIP at field 18 (5 or 9 digits, ZIP+4 with **no**
   hyphen — truncated to the first 5 for the Gazetteer lookup); `HD.dat` has
   Call Sign at field 4, License Status at field 5 (`"A"` = Active; also
   seen: `E`xpired, `C`ancelled, `T`erminated). Of 1,700,423 historical
   records in each file, 822,153 are Active as of this writing.

   **Memory shape matters**: `EN.dat`/`HD.dat` are ~220MB/~240MB
   uncompressed — and this board's own `/tmp` is **tmpfs** (RAM-backed), so
   even "download to a temp file" doesn't avoid holding the data in RAM
   unless that temp file lives on real disk storage. `download_and_seed`
   therefore takes a `work_dir` parameter (the Tauri command passes the
   app's real data directory, not `std::env::temp_dir()`), and both `.dat`
   files are streamed line-by-line straight out of the zip (`zip` crate's
   `ZipFile` is a `Read`, wrapped in a `BufReader`) rather than loaded into a
   `String`. Peak RSS is bounded by the active-callsign `HashSet` (~822k
   short strings) and the small bundled Gazetteer map, not by the file
   sizes. The downloaded zip is deleted after parsing either way (success or
   error).

2. **+ 3. Live decode** (`engine.rs::publish_decodes`). Any decode whose
   `DecodedMessage.grid` is non-empty gets recorded — that field is *already*
   only populated when `dsp/decoder.rs`'s `looks_like_grid` passes (4 chars,
   `A-R`/`A-R`/digit/digit), which structurally can never be true for a
   signal report, `RRR`, or `73`. No extra message-type gating needed; the
   existing grid-detection *is* the gate, matching Android's `checkFun1`
   exactly. Not restricted to CQs or replies-to-us — any heard station's
   grid is worth recording for the Map.

   **Correctness-checked for directional CQs** (e.g. `"CQ POTA K1ABC
   FN42"`): confirmed by reading `ft8_lib/ft8/message.c`'s `unpack28` that
   the directional token ("POTA", "DX", etc.) is decoded entirely into
   `call_to` (a compound string, "CQ POTA") via its own reserved n28 range
   (1003..=532443, the "CQ ABCD" case) — `call_from` and the grid come from
   two completely separate, independently-decoded fields and are never at
   risk. Proven, not just reasoned about: `tests/roundtrip.rs`'s
   `directional_cq_modifier_does_not_leak_into_call_from_or_grid` hand-builds
   a real 77-bit payload for exactly this case (since `pack77` itself can't
   *encode* a directional CQ — `message.c`'s `pack28` has a literal
   `// TODO: ... not implemented` for it) and decodes it through the actual
   C function.

4. **ADIF import backfill** (`db.rs::import_adif_with_progress`'s existing
   loop, which already parses `GRIDSQUARE` per record and already holds the
   transaction lock — one extra guarded call per record, same transaction,
   no new locking).

## Write priority

Two tiers, mirroring Android's own `addCallsignQTH` (unconditional replace)
vs. `fillCallsignQTHFromLog` (`INSERT OR IGNORE`):

- **Live decode always wins** (`Db::upsert_callsign_qth_live`, `INSERT OR
  REPLACE`) — directly heard over the air just now is the most authoritative
  source there is.
- **ADIF import and the FCC/Census seed only fill gaps**
  (`upsert_callsign_qth_if_absent_locked`, `INSERT OR IGNORE`) — neither may
  ever clobber a live-heard grid. This also means, structurally, that
  seeding order never matters: run the FCC import before or after months of
  live decoding and logging, and nothing already-known is ever downgraded.

## Tauri commands / frontend

`callsign_qth_count` (Settings panel's row count) and `import_fcc_grid_seed`
(manual re-run command, `catch_unwind`-wrapped like `import_adif` since it
also parses external file content this app doesn't control) in `main.rs`;
both call into the shared `run_fcc_grid_seed`, which the `setup()` auto-
trigger thread also calls directly (not through the command) so the native
popup can wrap it. `fcc-grid-progress`/`fcc-grid-done` events drive
`App.tsx`'s `FccGridSeedDialog`/`FccGridSeedCompleteDialog`/
`FccGridSeedErrorDialog` (the same structural shape as the existing
ADIF-import dialogs) via one passive listener active for the component's
whole lifetime — no imperative per-trigger listener setup. `CallsignGridPanel`
in Settings (same local-state/`useEffect(refresh, [])` pattern as the
existing `CustomBandsPanel`) shows the live row count and an "Update now"
button for re-running the seed later, since the FCC data changes weekly.

## Testing

287 Rust tests (`cargo test --lib` + the `main.rs` binary's own tests +
`tests/roundtrip.rs`; includes the new `fcc_grid.rs` module's parser/
Gazetteer tests against synthetic fixtures and real downloaded-data spot
checks, the `callsign_qth` schema-v7 migration test, REPLACE-vs-IGNORE
priority-ordering tests, and the directional-CQ protocol-level regression
test above), 125 vitest tests. Verified live against the real FCC data
repeatedly (806,625 / 806,661 / 806,667 / 806,668 / 806,669 / 806,670 rows
seeded across six separate runs, 822,153 active licenses found every time)
— first via direct sqlite queries (before the native-popup fix made the
live UI itself trustworthy to watch), then visually once the fix landed,
including the two-run idempotency check described above.

## Branch note

Started from `dev` (schema/resource-bundling patterns like `dxcc.rs`/
`us_states.rs` were cross-referenced from `feat/decode-waterfall-android-
design` without depending on it), then rebased onto
`feat/decode-waterfall-android-design`'s tip once it became clear this
needed to be tested with that branch's UI, not plain `dev`'s.
