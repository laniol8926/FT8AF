package radio.ks3ckc.ft8af.ui.components

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * Unit tests for [twoLineLabel] — how the landscape compact Call CQ button stacks its
 * (translated) label onto two lines.
 */
class TwoLineLabelTest {

    @Test
    fun `splits at the last space`() {
        assertThat(twoLineLabel("Call CQ")).isEqualTo("Call" to "CQ")
        assertThat(twoLineLabel("Lancer un CQ")).isEqualTo("Lancer un" to "CQ")
    }

    @Test
    fun `a label with no space stays on one line`() {
        assertThat(twoLineLabel("CQ呼出")).isEqualTo("CQ呼出" to null)
    }

    @Test
    fun `surrounding whitespace is ignored`() {
        assertThat(twoLineLabel(" Call CQ ")).isEqualTo("Call" to "CQ")
        assertThat(twoLineLabel(" CQ")).isEqualTo("CQ" to null)
    }
}
