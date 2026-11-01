package radio.ks3ckc.ft8af.lotw

import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

/** [LotwClient]: report URL and lotwreport.adi parsing (fork). */
class LotwClientTest {

    private val report = """
        ARRL Logbook of the World Status Report
        <PROGRAMID:4>LoTW
        <APP_LoTW_LASTQSL:19>2026-10-30 14:02:11
        <eoh>
        <APP_LoTW_OWNCALL:5>AI5II
        <CALL:5>K9LZJ
        <BAND:3>40M
        <MODE:4>JT65
        <QSO_DATE:8>20150909
        <TIME_ON:6>213900
        <QSL_RCVD:1>Y
        <eor>
        <CALL:6>va3tps
        <BAND:3>20M
        <MODE:4>MFSK
        <SUBMODE:3>FT4
        <QSO_DATE:8:D>20261027
        <TIME_ON:6>014700
        <QSL_RCVD:1>Y
        <eor>
    """.trimIndent()

    @Test
    fun parsesConfirmations() {
        val r = LotwClient.parseReport(report)
        assertThat(r.lastQsl).isEqualTo("2026-10-30 14:02:11")
        assertThat(r.qsls).hasSize(2)
        assertThat(r.qsls[0]).isEqualTo(
            LotwClient.Qsl("K9LZJ", "20150909", "213900", "40m", "JT65", "JT65"),
        )
        // FT4 arrives as MFSK + SUBMODE; call is upper-cased; typed field (:D) still read.
        assertThat(r.qsls[1].call).isEqualTo("VA3TPS")
        assertThat(r.qsls[1].mode).isEqualTo("FT4")
        assertThat(r.qsls[1].rawMode).isEqualTo("MFSK")
        assertThat(r.qsls[1].qsoDate).isEqualTo("20261027")
    }

    @Test
    fun badLoginPage_isReportedNotParsed() {
        val e = assertThrows(IllegalArgumentException::class.java) {
            LotwClient.parseReport("<html>Username/password incorrect</html>")
        }
        assertThat(e.message).contains("username/password")
    }

    @Test
    fun reportUrl_isIncrementalWhenSinceGiven() {
        val url = LotwClient.buildReportUrl("ai5ii", "p&ss", "2026-10-30")
        assertThat(url).startsWith("https://lotw.arrl.org/lotwuser/lotwreport.adi?login=ai5ii")
        assertThat(url).contains("password=p%26ss")
        assertThat(url).contains("qso_qsl=yes")
        assertThat(url).contains("qso_qslsince=2026-10-30")
        assertThat(LotwClient.buildReportUrl("a", "b", null)).doesNotContain("qslsince")
    }

    @Test
    fun sinceDate_takesTheDayPart() {
        assertThat(LotwClient.sinceDate("2026-10-30 14:02:11")).isEqualTo("2026-10-30")
        assertThat(LotwClient.sinceDate(null)).isNull()
        assertThat(LotwClient.sinceDate("bad")).isNull()
    }
}
