package radio.ks3ckc.ft8af.ui.components

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/** Unit tests for [otherTxSlot], the landscape Even/Odd button's toggle. */
class OtherTxSlotTest {

    @Test
    fun `even switches to odd and odd back to even`() {
        assertThat(otherTxSlot(0)).isEqualTo(1)
        assertThat(otherTxSlot(1)).isEqualTo(0)
    }
}
