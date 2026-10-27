# Landscape layout for the Decode screen

## Problem

On a phone in landscape the screen is only ~384–410 dp tall. The portrait
layout stacked, top to bottom: the Decode title row, the filter chips, the
decode list, the slot timer bar, the status line ("Listening / Next transmit
window"), the operate-controls drawer peek (handle + Call CQ + Hunt, ~130 dp)
and the tab bar (~80 dp). Measured on a Galaxy A32 (384 dp tall in
landscape), that left the decode list **no rows at all**, the filter chips
cut in half, and the decode empty-state graphic drawn over the title row.
Expanding "More controls" made it worse: the sheet grew taller than the
screen and pushed its own handle off the top, leaving nothing on screen to
collapse it with (only the Back button worked).

Landscape has width to spare, so the fix moves things *sideways* onto rows
that already exist rather than adding a side column. (A fixed right-hand
controls panel was tried first and rejected: it took half the width.)

## The landscape layout

Applies when the device is in landscape, the operate-controls drawer setting
is on (the default) and the Decode tab is showing. Portrait is unchanged, and
so are the other tabs except where noted.

| Area | Portrait | Landscape (Decode tab) |
|---|---|---|
| Title row | band/mode label, "UTC : 04:02:39" (and "• N decoded" outside TIME sort) on a second line, action icons | band/mode label, **compact Call CQ**, **compact Hunt**, **RX / "TX in 9s" + stacked CAT chip**, **TUNE**, **Even/Odd**, action icons, all on one line with tighter padding; no clock or decoded count (the slot bar and the waterfall strip show time) |
| Under the title row | filter chips | **slot timer bar** (DT / GPS pill, progress, seconds), then the filter chips |
| Decode list | saved compact/expanded choice | **always opens compact**, 30-character message column, in the left panel of a **split view** with the map or waterfall on the right |
| Above the tab bar | slot bar, status line, drawer peek | nothing; the list runs down to the tab bar |
| Tab bar | icon above label, 30 dp spare at the bottom | icon beside label, 8 dp at the bottom, plus a **More controls** entry |
| More controls drawer | peek with Call CQ + Hunt, expands upward | hidden until opened from the tab bar; holds Band & Mode, TX period / Tune / DX and power |

Result on a motorola edge 2022: about 12 compact decode lines in landscape,
up from none.

### Split view: decode list + map or waterfall

In landscape the **Decode, Map and Waterfall tabs all show the Decode
screen**, split below the filter chips (`DecodeScreen(landscapeSidePanel = …)`):

- **Left:** the decode list, `DecodeListPanelWidth` (480 dp) wide, enough for a
  compact row's 30-character message plus its columns and badge.
- **Right:** the rest of the width. The **Map** tab shows the map there, the
  **Waterfall** tab the waterfall, and the **Decode** tab whichever of the two
  was picked last (`sidePanelTab` in `FT8AFApp`). The pick is saved
  (`GeneralVariables.landscapeSidePanel`, DB key `landscapePanel`: 0 =
  waterfall, the default, 1 = map) and restored once the config has loaded,
  so the panel survives an app restart. Only picks made in the split view are
  saved; in portrait Map and Waterfall are full screen and don't change it.
  The tab bar highlights the tab that was tapped.

All three tabs land on the same `DecodeScreen` branch in `FT8AFApp`
(`screenTab`), so the decode list keeps its rows and scroll position while
the right panel switches. The QSO panel uses `screenTab` too, so it docks
as on Decode rather than floating as on the full-screen Waterfall. POTA,
Logbook and Settings stay full screen.

In the panel:

- **Waterfall** (`WaterfallScreen(showSpectrum = false)`): no columnar
  spectrum strip, just the frequency ruler, the waterfall and its bottom
  strip. The tap-cursor timeout now runs outside the spectrum's update so it
  still clears without the strip.
- **Map** (`MapScreen(compactHeader = true)`): no "Map" title and no
  "N decoded · N worked this session" line; the Decodes / PSK spots / Worked
  chips sit on the settings button's row, and both are 30 dp tall
  (`MapCompactButtonHeight`) instead of 44 dp.

The full-screen Map and Waterfall (portrait, or with the drawer off) are
unchanged.

### 30-character message column

The compact row's message slot is `COMPACT_MESSAGE_CHARS` (30) characters of
its own mono font, measured with a `TextMeasurer`, so SNR / Hz / DT and the
badge follow right after the message instead of sitting at the far right of
a wide row; spare width goes to the end of the row. On a row too narrow for
that (portrait) the slot shrinks as before. A CALLING chip shares the slot,
so its row stays aligned with the others.

### Compact Call CQ (`CompactCallCqButton`, `TxStrip.kt`)

A ~70 dp button with the same logic as the full `CallCqButton`:

- tap calls CQ; once a CQ/QSO is running it becomes the red **Stop** button;
- dimmed and locked while Hunt is armed (`TxStripActionState.cqDisabled`);
- the attached **MORE ▾** segment, or a long-press, opens the CQ options;
- the queued CQ variant (FREE, FD, POTA, …) shows after "CQ".

The label is stacked on two lines by `twoLineLabel()`, which splits the
*translated* label at its last space ("Call CQ" → "Call" / "CQ"); a label
with no space stays on one line.

### Compact Hunt (`CompactHuntButton`, `TxStrip.kt`)

A plain on/off button labelled "Hunt": no icon, no option chip.

- Tap turns Hunt on and the background turns green (`StatusConfirmed`, dark
  text); tap again turns it off and the normal surface colour returns.
- Same lock rule as the Hunt tile: dimmed and locked while a CQ/QSO is running.
- **Long-press opens the hunt options** (priority picker). Without the chip
  this is the only way to reach them in landscape, since Hunt is no longer
  in the drawer there.
- Exposes its on/off state to TalkBack via `toggleableState`.

### Status line on the title row

`OperateStatusRow(fillWidth = false)` renders the same pulse dot and CAT chip
("Radio linked", or the tap-to-reconnect chip when the link drops) inline,
with the state abbreviated (`TxStatusVisuals.shortLabelRes`): **RX** while
listening, **TX** while transmitting, **TUNE** while tuning. Under RX the
countdown reads **"TX in Xs"** (`tx_next_window_short`). Portrait and the
drawer keep "Listening / Next transmit window in Xs".

Inline, the status part (dot + state + countdown) has a fixed width,
`InlineStatusWidth` (84 dp), so the chip and buttons after it don't shift
when the state changes (on a real device a tap once landed on the wrong spot
after such a shift). It is also the row's **only flexible item**: it is
weighted, so the fixed-width Call CQ, Hunt, CAT chip, TUNE and Even/Odd are
measured first and keep their full width, and if the row is short the status
is what gives way.

The landscape title row carries no clock and no "• N decoded" count. The
count (shown outside the TIME sort) and the clock squeezed the status off the
row on a narrower phone with the radio linked; the slot bar and the
waterfall's strip still show the time. With the drawer setting off there are
no controls on the row, and the clock and count show as before.

The CAT chip is stacked on two lines there (`CatStatusChip(stacked = true)`:
"Radio" / "linked", or "Radio not" / "linked", split by `twoLineLabel`) with
tighter padding, about half its single-line width. Tapping it still
reconnects.

### TUNE and Even/Odd on the title row

- **TUNE** (`CompactTuneButton`) is the drawer's `SecondaryButton` TUNE at
  the title row's 36 dp height and a fixed 56 dp width: tap starts the tune
  carrier, it turns amber with its countdown ("TUNE 7s", wrapping to two
  lines), tap again stops it. It is enabled by the same rule as the drawer's
  TUNE, now shared as `tuneButtonEnabled()`: always while tuning (to stop),
  otherwise only when no CQ/QSO is running and nothing is transmitting.
- **Even/Odd** (`CompactTxPeriodButton`, 56 dp) replaces the drawer's
  two-segment TX PERIOD control on the title row. It reads **Even** or
  **Odd** for the selected slot (`tx_period_even_short` /
  `tx_period_odd_short`), and a tap switches to the other slot
  (`otherTxSlot()`) through the same action as the segmented control, which
  also drops an in-progress QSO, as switching slots there does.

Both remain in the More controls drawer as well.

After Even/Odd come two readouts at the same height (`CompactSwrChip`,
`CompactPowerChip`, 44 dp each). Each shows a small label over its value:

- **SWR**, colored green up to 2.0, amber up to 3.0 and red above. It appears
  once a radio has reported SWR (the QMX's `SW;` meter while keyed) and keeps
  the last reading.
- **PWR**, the operator identity power ("4.7W"). It's kept current by the
  QMX's `PC;` meter, otherwise it's what was set in Settings. It's hidden
  when no power is set.

The QSO panel header shows the same two values ("SWR 1.2  4.7W"). See
[qmx-transmit.md](qmx-transmit.md).

`ClickableTopBarTitle` keeps the band/mode label at full width whenever it
has trailing content, so the trailing group (clock, buttons, status) is what
gives way on a narrow screen, not the label (which was being squeezed to
"40…").

### Slot timer bar under the title row

`DecodeScreen(landscapeSlotBar = …)` places the `SlotTimerBar` between the
title row and the filter chips, and `FT8AFApp` drops it from above the tab
bar. `TopBar(tight = true)` trims the title row's vertical padding and
centres the title on the action icons so no blank band opens up between the
row and the bar.

### More controls from the tab bar

In landscape the drawer's collapsed peek height is 0, so nothing of it shows
until it is opened. `TabBar(onToggleMoreControls = …)` adds a **More
controls** entry (sliders icon, `FT8AFIcons.Sliders`) after the tabs; it
turns amber while the drawer is open and tapping it again closes it. The
open drawer's own handle and a tap outside it still close it too.

The drawer itself (`OperateControlsDrawer`):

- leaves out the Call CQ / Hunt row while those are on the title row
  (`showPrimaryActions = false`);
- is capped at the space above the tab bar minus `DrawerTopMargin`, so its
  handle can never leave the screen;
- scrolls its rows when they don't fit. The content's natural height is
  measured as the rows' viewport plus `ScrollState.maxValue`, which stays
  constant whatever height the sheet is clipped to; scrolling is only
  enabled while the drawer is open, and it scrolls back to the top on close.

On other tabs in landscape there is no title row to hold Call CQ and Hunt, so
the drawer keeps them there.

### Compact tab bar

`TabBar(compact = true)` lays each entry out as icon-beside-label with 4 dp
item padding and 8 dp below (the portrait 30 dp is spare room for the
navigation bar, which the app hides; a swipe up from the bottom edge still
reveals it). The selection pill widens to cover icon and label. This roughly
halves the bar's height.

### Compact view by default in landscape

`DecodeScreen` keeps two compact/expanded states. Portrait uses the saved
choice (`GeneralVariables.simpleCallItemMode`, DB key `msgMode`). Landscape
always opens compact; the ≡ toggle still switches it there for the moment,
without changing the saved portrait choice.

### Empty-state clipping

`EmptyState` in `DecodeScreen` is now `clipToBounds()`. When the list area
was short (landscape, especially with the QSO panel open) its waves graphic
overflowed upward and drew over the title row, hiding the Call CQ / Hunt
buttons.

## Known limits

- The title row is tight on narrower landscape screens (e.g. a Galaxy A32,
  ~853 dp wide). With the radio linked everything fits there; if a row ever
  runs short, the status (RX / "TX in Xs") is what shrinks, never the
  buttons or the band/mode label.
- `tx_next_window_short` and the RX / TX / TUNE short states are
  English-only, like `tx_next_window` (none have translations yet).
- With the "controls drawer" setting turned off, landscape still uses the
  original always-open strip; this layout only applies with the drawer on.

## Testing

- `TwoLineLabelTest` covers the label split (last space, no space, padding).
- `TxStatusVisualsTest` covers each state's short label (RX / TX / TUNE).
- `TuneChipLabelTest.tuneButtonEnabled_onlyWhenIdleOrAlreadyTuning` covers the
  shared TUNE enable rule; `OtherTxSlotTest` covers the Even/Odd toggle.
- `tx_period_even_short` / `tx_period_odd_short` are English-only for now,
  like the other strings in this area.
- Existing `TxStripActionStateTest` covers the CQ/Hunt lock rules the compact
  buttons reuse.
- Layout verified on device in landscape and portrait: Galaxy A32 (Android
  13) and motorola edge 2022 (Android 15).
