package radio.ks3ckc.ft8af.ui.components

import com.google.common.truth.Truth.assertThat
import com.k1af.ft8af.ModeProfile
import com.k1af.ft8af.R
import com.k1af.ft8af.database.OperationBand
import org.junit.Test

/**
 * Unit tests for the pure Band & Mode sheet helpers ([formatSlotSeconds], [bandHintRes],
 * [mostUsedBands]). No Android runtime needed: OperationBand.Band is a plain data holder,
 * ModeProfile is an enum, and R.string ids are compile-time ints.
 */
class BandModeSheetLogicTest {

    private fun band(freq: Long, wave: String, modeId: Int, marked: Boolean): OperationBand.Band {
        val b = OperationBand.Band(freq, wave)
        b.mode = modeId
        b.marked = marked
        return b
    }

    @Test
    fun `formatSlotSeconds trims whole and fractional seconds`() {
        assertThat(formatSlotSeconds(15_000)).isEqualTo("15s")
        assertThat(formatSlotSeconds(7_500)).isEqualTo("7.5s")
        assertThat(formatSlotSeconds(3_750)).isEqualTo("3.75s")
    }

    @Test
    fun `formatSlotSeconds matches the shipped mode profiles`() {
        assertThat(formatSlotSeconds(ModeProfile.FT8.slotMillis)).isEqualTo("15s")
        assertThat(formatSlotSeconds(ModeProfile.FT4.slotMillis)).isEqualTo("7.5s")
        assertThat(formatSlotSeconds(ModeProfile.FT2.slotMillis)).isEqualTo("3.75s")
    }

    @Test
    fun `20m hint is time-of-day aware, others are static`() {
        assertThat(bandHintRes("20m", isDaytime = true)).isEqualTo(R.string.band_hint_20m_best)
        assertThat(bandHintRes("20m", isDaytime = false)).isEqualTo(R.string.band_hint_20m)
        assertThat(bandHintRes("40m", isDaytime = true)).isEqualTo(R.string.band_hint_40m)
        assertThat(bandHintRes("17m", isDaytime = false)).isEqualTo(R.string.band_hint_17m)
        assertThat(bandHintRes("15m", isDaytime = true)).isEqualTo(R.string.band_hint_15m)
        assertThat(bandHintRes("10m", isDaytime = false)).isEqualTo(R.string.band_hint_10m)
        assertThat(bandHintRes("80m", isDaytime = true)).isEqualTo(R.string.band_hint_80m)
    }

    @Test
    fun `bandHintRes returns 0 for a band with no hint`() {
        assertThat(bandHintRes("6m", isDaytime = true)).isEqualTo(0)
    }

    @Test
    fun `mostUsedBands keeps display order and the mode's dials`() {
        val ft8 = ModeProfile.FT8.id
        val ft4 = ModeProfile.FT4.id
        val bands = listOf(
            band(7_074_000, "40m", ft8, marked = true),
            band(14_074_000, "20m", ft8, marked = true),
            band(14_080_000, "20m", ft4, marked = true), // wrong mode — must be ignored
            band(21_074_000, "15m", ft8, marked = true),
        )
        val rows = mostUsedBands(bands, ft8)
        assertThat(rows.map { it.waveLength }).containsExactly("20m", "40m", "15m").inOrder()
        val twenty = rows.first { it.waveLength == "20m" }
        assertThat(twenty.freqHz).isEqualTo(14_074_000)
        assertThat(twenty.bandIndex).isEqualTo(1)
    }

    @Test
    fun `mostUsedBands prefers the marked entry over an earlier unmarked one`() {
        val ft8 = ModeProfile.FT8.id
        val bands = listOf(
            band(14_071_000, "20m", ft8, marked = false),
            band(14_074_000, "20m", ft8, marked = true),
        )
        val rows = mostUsedBands(bands, ft8)
        assertThat(rows).hasSize(1)
        assertThat(rows[0].freqHz).isEqualTo(14_074_000)
        assertThat(rows[0].bandIndex).isEqualTo(1)
    }

    @Test
    fun `mostUsedBands falls back to first match when nothing is marked`() {
        val ft8 = ModeProfile.FT8.id
        val bands = listOf(
            band(14_071_000, "20m", ft8, marked = false),
            band(14_090_000, "20m", ft8, marked = false),
        )
        val rows = mostUsedBands(bands, ft8)
        assertThat(rows).hasSize(1)
        assertThat(rows[0].freqHz).isEqualTo(14_071_000)
    }

    /** A cut-down FT8 band plan in bands.txt order, every curated band present. */
    private fun ft8Plan(): List<OperationBand.Band> {
        val ft8 = ModeProfile.FT8.id
        return listOf(
            band(1_840_000, "160m", ft8, marked = true),
            band(3_573_000, "80m", ft8, marked = true),
            band(5_357_000, "60m", ft8, marked = false),
            band(7_074_000, "40m", ft8, marked = true),
            band(10_136_000, "30m", ft8, marked = true),
            band(14_074_000, "20m", ft8, marked = true),
            band(18_100_000, "17m", ft8, marked = true),
            band(21_074_000, "15m", ft8, marked = true),
            band(28_074_000, "10m", ft8, marked = true),
            band(50_313_000, "6m", ft8, marked = true),
        )
    }

    @Test
    fun `mostUsedBands with nothing hidden is the curated list in curated order`() {
        val rows = mostUsedBands(ft8Plan(), ModeProfile.FT8.id) { false }
        assertThat(rows.map { it.waveLength }).containsExactly("20m", "40m", "17m", "15m", "10m").inOrder()
    }

    @Test
    fun `mostUsedBands fills hidden curated bands from the operator's enabled bands`() {
        // An 80/60/40/30/20m rig: everything else hidden (the real-device setting).
        val enabled = setOf("80m", "60m", "40m", "30m", "20m")
        val rows = mostUsedBands(ft8Plan(), ModeProfile.FT8.id) { it !in enabled }
        assertThat(rows.map { it.waveLength }).containsExactly("80m", "60m", "40m", "30m", "20m").inOrder()
        // Unmarked-only band (60m) still gets its first dial.
        assertThat(rows.first { it.waveLength == "60m" }.freqHz).isEqualTo(5_357_000)
    }

    @Test
    fun `mostUsedBands never shows a hidden band, even when there is nothing to fill with`() {
        val enabled = setOf("20m")
        val rows = mostUsedBands(ft8Plan(), ModeProfile.FT8.id) { it !in enabled }
        assertThat(rows.map { it.waveLength }).containsExactly("20m")
    }
}
