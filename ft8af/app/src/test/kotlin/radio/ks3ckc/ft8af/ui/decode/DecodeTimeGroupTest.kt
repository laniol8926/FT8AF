package radio.ks3ckc.ft8af.ui.decode

import com.google.common.truth.Truth.assertThat
import com.k1af.ft8af.FT8Common
import com.k1af.ft8af.Ft8Message
import org.junit.Test

/**
 * Unit-tests the pure [computeTimeGroupDividers] decode-list slot grouper.
 *
 * No Robolectric runner: [computeTimeGroupDividers] only reaches the pure
 * [com.k1af.ft8af.ModeProfile] and [com.k1af.ft8af.ui.WaterfallTimestampGate],
 * and the `Ft8Message(int)` constructor just assigns `signalFormat` (no
 * android.util.Log). Any incidental framework call returns defaults under
 * `unitTests.returnDefaultValues`.
 *
 * The divider marks the first row of each receive slot. It must key on the
 * per-message mode's slot length (FT8 15s, FT4 7.5s, FT2 3.75s) rather than a
 * hard-coded 15s, which previously collapsed 2-4 fast-mode cycles under one
 * divider.
 */
class DecodeTimeGroupTest {

    private fun msg(mode: Int, utcMs: Long) = Ft8Message(mode).apply { utcTime = utcMs }

    @Test
    fun emptyList_returnsEmptyArray() {
        assertThat(computeTimeGroupDividers(emptyList()).toList()).isEmpty()
    }

    @Test
    fun firstMessage_alwaysGetsDivider() {
        val result = computeTimeGroupDividers(listOf(msg(FT8Common.FT8_MODE, 15_000L)))
        assertThat(result.toList()).containsExactly(true)
    }

    @Test
    fun ft8_groupsWithinSlotAndBreaksAcrossSlots() {
        // Two decodes in the same 15s slot, then one in the next slot.
        val messages = listOf(
            msg(FT8Common.FT8_MODE, 30_000L),
            msg(FT8Common.FT8_MODE, 32_000L),
            msg(FT8Common.FT8_MODE, 45_000L),
        )

        val result = computeTimeGroupDividers(messages)

        assertThat(result.toList()).containsExactly(true, false, true).inOrder()
    }

    @Test
    fun ft8_reproducesLegacyDivide15000() {
        // Reproduce the previous inline `utcTime / 15000L` grouping exactly.
        val times = longArrayOf(0L, 14_999L, 15_000L, 29_999L, 30_000L, 30_001L)
        val messages = times.map { msg(FT8Common.FT8_MODE, it) }

        val result = computeTimeGroupDividers(messages)

        var prevSlot = Long.MIN_VALUE
        val expected = times.mapIndexed { i, t ->
            val slot = t / 15_000L
            val divider = i == 0 || slot != prevSlot
            prevSlot = slot
            divider
        }
        assertThat(result.toList()).isEqualTo(expected)
    }

    @Test
    fun ft4_marksEachHalfCycleThatFifteenSecondGridWouldCollapse() {
        // Two FT4 decodes in the SAME 15s window (both `utc/15000 == 0`) but
        // different 7.5s slots (0 and 1). The old /15000 math drew one divider;
        // the mode-aware grid draws two.
        val messages = listOf(
            msg(FT8Common.FT4_MODE, 1_000L),   // 7.5s slot 0
            msg(FT8Common.FT4_MODE, 8_000L),   // 7.5s slot 1
        )

        // Precondition: the legacy 15s grid would have grouped these.
        assertThat(1_000L / 15_000L).isEqualTo(8_000L / 15_000L)

        val result = computeTimeGroupDividers(messages)

        assertThat(result.toList()).containsExactly(true, true).inOrder()
    }

    @Test
    fun ft2_marksEachQuarterCycle() {
        // Four FT2 decodes one 3.75s slot apart each get their own divider,
        // where a 15s grid would have shown a single group.
        val messages = listOf(
            msg(FT8Common.FT2_MODE, 3_750L),
            msg(FT8Common.FT2_MODE, 7_500L),
            msg(FT8Common.FT2_MODE, 11_250L),
            msg(FT8Common.FT2_MODE, 15_000L),
        )

        val result = computeTimeGroupDividers(messages)

        assertThat(result.toList()).containsExactly(true, true, true, true).inOrder()
    }

    @Test
    fun modeChange_forcesDividerEvenWhenSlotIndexCoincides() {
        // FT8 utc=15000 -> slot index 1; FT4 utc=7500 -> slot index 1 too, but the
        // slot lengths differ so they must not be grouped together.
        val messages = listOf(
            msg(FT8Common.FT8_MODE, 15_000L),
            msg(FT8Common.FT4_MODE, 7_500L),
        )

        val result = computeTimeGroupDividers(messages)

        assertThat(result.toList()).containsExactly(true, true).inOrder()
    }

    // ---- per-slot decode counts (one on every divider) ----------------------

    @Test
    fun countRowsPerSlot_givesEverySlotItsOwnCount() {
        val messages = listOf(
            msg(FT8Common.FT8_MODE, 30_000L),
            msg(FT8Common.FT8_MODE, 31_000L),
            msg(FT8Common.FT8_MODE, 32_000L),
            msg(FT8Common.FT8_MODE, 45_000L),
            msg(FT8Common.FT8_MODE, 60_000L),
            msg(FT8Common.FT8_MODE, 61_000L),
        )

        val counts = countRowsPerSlot(messages)

        assertThat(counts[decodeSlotKey(messages[0])]).isEqualTo(3)
        assertThat(counts[decodeSlotKey(messages[3])]).isEqualTo(1)
        assertThat(counts[decodeSlotKey(messages[4])]).isEqualTo(2)
        assertThat(counts).hasSize(3)
    }

    @Test
    fun countRowsPerSlot_matchesTheRowsDrawnUnderEachDivider() {
        // The real-device bug: dividers counted every decode, but the rows are one
        // per station at its latest decode. K1ABC heard again at 30s moved its row
        // out of the 15s slot, and a callsign-less decode had no row at all, yet
        // both still counted toward the 15s slot.
        val decodes = listOf(
            station("K1ABC", 15_000L),
            station("K2DEF", 16_000L),
            station(null, 17_000L),
            station("K1ABC", 30_000L),
        )
        val rows = collapseByStation(decodes, DecodeSortMode.LAST_HEARD)
        val dividers = computeTimeGroupDividers(rows)

        val counts = countRowsPerSlot(rows)

        // Each divider's count is exactly the rows up to the next divider.
        var i = 0
        while (i < rows.size) {
            assertThat(dividers[i]).isTrue()
            var end = i + 1
            while (end < rows.size && !dividers[end]) end++
            assertThat(counts[decodeSlotKey(rows[i])]).isEqualTo(end - i)
            i = end
        }
        assertThat(counts[decodeSlotKey(decodes[0])]).isEqualTo(1)
        assertThat(counts[decodeSlotKey(decodes[3])]).isEqualTo(1)
    }

    private fun station(call: String?, utcMs: Long) =
        msg(FT8Common.FT8_MODE, utcMs).apply { callsignFrom = call }

    @Test
    fun countRowsPerSlot_keysByModeLikeTheDividers() {
        // FT8 utc=15000 and FT4 utc=7500 share slot index 1 but are different
        // slots (different lengths), matching modeChange_forcesDividerEvenWhenSlotIndexCoincides.
        val ft8 = msg(FT8Common.FT8_MODE, 15_000L)
        val ft4 = msg(FT8Common.FT4_MODE, 7_500L)

        val counts = countRowsPerSlot(listOf(ft8, ft4))

        assertThat(decodeSlotKey(ft8)).isNotEqualTo(decodeSlotKey(ft4))
        assertThat(counts[decodeSlotKey(ft8)]).isEqualTo(1)
        assertThat(counts[decodeSlotKey(ft4)]).isEqualTo(1)
    }

    @Test
    fun countRowsPerSlot_emptyListHasNoCounts() {
        assertThat(countRowsPerSlot(emptyList())).isEmpty()
    }
}
