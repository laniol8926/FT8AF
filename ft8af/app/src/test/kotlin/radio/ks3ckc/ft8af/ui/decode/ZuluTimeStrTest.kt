package radio.ks3ckc.ft8af.ui.decode

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/** Unit tests for [zuluTimeStr], the landscape Decode title row's "HH:MM:SSZ" clock. */
class ZuluTimeStrTest {

    @Test
    fun `formats as zero-padded HH MM SS with a Z suffix`() {
        // 2026-10-26T04:02:39Z
        assertThat(zuluTimeStr(1_792_987_359_000L)).isEqualTo("04:02:39Z")
        assertThat(zuluTimeStr(1_000L)).isEqualTo("00:00:01Z")
    }

    @Test
    fun `wraps at midnight`() {
        assertThat(zuluTimeStr(86_399_000L)).isEqualTo("23:59:59Z")
        assertThat(zuluTimeStr(86_400_000L)).isEqualTo("00:00:00Z")
    }

    @Test
    fun `unknown clock shows a placeholder`() {
        assertThat(zuluTimeStr(0L)).isEqualTo("--:--:--Z")
    }
}
