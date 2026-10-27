package radio.ks3ckc.ft8af.ui.components

import com.google.common.truth.Truth.assertThat
import com.k1af.ft8af.ft8transmit.MeterProtectionController
import org.junit.Test

/** The landscape SWR chip: colour bands and the ratio text it shows. */
class SwrChipTest {

    @Test
    fun bands_splitAtTwoAndThree() {
        assertThat(swrBand(1.0f)).isEqualTo(SwrBand.GOOD)
        assertThat(swrBand(2.0f)).isEqualTo(SwrBand.GOOD)
        assertThat(swrBand(2.1f)).isEqualTo(SwrBand.CAUTION)
        assertThat(swrBand(3.0f)).isEqualTo(SwrBand.CAUTION)
        assertThat(swrBand(3.1f)).isEqualTo(SwrBand.BAD)
    }

    @Test
    fun ratioText_hasOneDecimalAndCapsAtNinePointNine() {
        assertThat(MeterProtectionController.formatSwrRatio(1.30f)).isEqualTo("1.3")
        assertThat(MeterProtectionController.formatSwrRatio(1.0f)).isEqualTo("1.0")
        assertThat(MeterProtectionController.formatSwrRatio(12f)).isEqualTo("9.9+")
    }

    @Test
    fun powerText_isNotRounded() {
        assertThat(MeterProtectionController.formatPowerWatts(4.5f)).isEqualTo("4.5W")
        assertThat(MeterProtectionController.formatPowerWatts(0.8f)).isEqualTo("0.8W")
        assertThat(MeterProtectionController.formatPowerWatts(12.4f)).isEqualTo("12.4W")
        assertThat(MeterProtectionController.formatPowerWatts(5f)).isEqualTo("5W")
    }
}
