package radio.ks3ckc.ft8af.qrz

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/** [pickLoggedState]: QRZ's state at home, the worked grid's state when away (fork). */
class LoggedStateTest {

    @Test
    fun atHome_usesQrzState() {
        // Border grid: the grid table says AL, QRZ knows the station is in GA.
        assertThat(pickLoggedState("AL", "ga", "EM73jb", "EM73", portable = false)).isEqualTo("GA")
    }

    @Test
    fun awayFromHome_usesWorkedGrid() {
        // QRZ address in Ohio, operating from EM63 (Alabama).
        assertThat(pickLoggedState("AL", "OH", "EN80", "EM63", portable = false)).isEqualTo("AL")
    }

    @Test
    fun pota_alwaysUsesWorkedGrid() {
        assertThat(pickLoggedState("TN", "TN", "EM75", "EM75", portable = true)).isEqualTo("TN")
        assertThat(pickLoggedState("KY", "TN", "EM75", "EM75", portable = true)).isEqualTo("KY")
    }

    @Test
    fun noQrz_usesWorkedGrid() {
        assertThat(pickLoggedState("TX", null, null, "EM12", portable = false)).isEqualTo("TX")
        assertThat(pickLoggedState(null, null, null, "IO91", portable = false)).isNull()
    }

    @Test
    fun noWorkedGrid_trustsQrz() {
        assertThat(pickLoggedState(null, "IN", "EM79", "", portable = false)).isEqualTo("IN")
    }

    @Test
    fun portableSuffixes() {
        assertThat(isPortableCall("K9LZJ/P")).isTrue()
        assertThat(isPortableCall("W1AW/M")).isTrue()
        assertThat(isPortableCall("VE3/K9LZJ")).isFalse()
        assertThat(isPortableCall("K9LZJ")).isFalse()
        assertThat(isPortableCall(null)).isFalse()
    }
}
