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
| Title row | band/mode label, "UTC : 04:02:39" on a second line, action icons | band/mode label, Zulu clock "04:02:39Z", **compact Call CQ**, **compact Hunt**, **status line + CAT chip**, action icons, all on one line with tighter padding |
| Under the title row | filter chips | **slot timer bar** (DT / GPS pill, progress, seconds), then the filter chips |
| Decode list | saved compact/expanded choice | **always opens compact** |
| Above the tab bar | slot bar, status line, drawer peek | nothing; the list runs down to the tab bar |
| Tab bar | icon above label, 30 dp spare at the bottom | icon beside label, 8 dp at the bottom, plus a **More controls** entry |
| More controls drawer | peek with Call CQ + Hunt, expands upward | hidden until opened from the tab bar; holds Band & Mode, TX period / Tune / DX and power |

Result on a motorola edge 2022: about 12 compact decode lines in landscape,
up from none.

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

`OperateStatusRow(fillWidth = false)` renders the same pulse dot, state
("Listening", "Transmitting", "Tuning") and CAT chip ("Radio linked", or the
tap-to-reconnect chip when the link drops) inline. The countdown uses the
short form **"Next TX in Xs"** (`tx_next_window_short`); the full "Next
transmit window in Xs" is kept everywhere else.

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
  ~853 dp wide). The clock there uses the Zulu form "04:02:39Z"
  (`zuluTimeStr`) instead of "UTC : 04:02:39", which frees ~60 dp. The
  band/mode label is protected, so if the row still runs short the status
  line shrinks first and the countdown's seconds can be clipped.
- `tx_next_window_short` is English-only, like `tx_next_window` (neither has
  translations yet).
- With the "controls drawer" setting turned off, landscape still uses the
  original always-open strip; this layout only applies with the drawer on.

## Testing

- `TwoLineLabelTest` covers the label split (last space, no space, padding).
- `ZuluTimeStrTest` covers the "HH:MM:SSZ" clock (padding, midnight wrap,
  unknown-clock placeholder).
- Existing `TxStripActionStateTest` covers the CQ/Hunt lock rules the compact
  buttons reuse.
- Layout verified on device in landscape and portrait: Galaxy A32 (Android
  13) and motorola edge 2022 (Android 15).
