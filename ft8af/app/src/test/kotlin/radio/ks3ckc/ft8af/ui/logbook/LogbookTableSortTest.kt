package radio.ks3ckc.ft8af.ui.logbook

import com.google.common.truth.Truth.assertThat
import com.k1af.ft8af.log.QSLCallsignRecord
import org.junit.Test
import radio.ks3ckc.ft8af.ui.components.QsoStatus

/** [sortQsoTable]: the landscape logbook table's tap-a-heading sort (fork). */
class LogbookTableSortTest {

    private fun rec(
        call: String,
        date: String,
        time: String = "120000",
        freq: String = "7.074",
        rstSent: String = "",
        confirmed: Boolean = false,
        worked: Boolean = false,
    ): QSLCallsignRecord {
        val r = QSLCallsignRecord()
        r.setCallsign(call)
        r.lastTime = date
        r.timeOn = time
        r.band = "40m($freq MHz)"
        r.rstSent = rstSent
        r.isLotW_QSL = confirmed
        r.isQSL = worked
        return r
    }

    private val noState: (String) -> String? = { null }

    private fun calls(list: List<QSLCallsignRecord>) = list.map { it.callsign }

    private val log = listOf(
        rec("KW8CW", "20261028", rstSent = "+05", freq = "7.074"),
        rec("AA6MU", "20261027", rstSent = "-21", freq = "14.074", confirmed = true),
        rec("N5PNZ", "20261029", rstSent = "", freq = "10.136", worked = true),
    )

    @Test
    fun dateDefaultIsNewestFirst_andReverses() {
        assertThat(calls(sortQsoTable(log, QSO_COL_DATE, false, noState)))
            .containsExactly("N5PNZ", "KW8CW", "AA6MU").inOrder()
        assertThat(calls(sortQsoTable(log, QSO_COL_DATE, true, noState)))
            .containsExactly("AA6MU", "KW8CW", "N5PNZ").inOrder()
    }

    @Test
    fun callsignSortsAlphabetically() {
        assertThat(calls(sortQsoTable(log, QSO_COL_CALL, true, noState)))
            .containsExactly("AA6MU", "KW8CW", "N5PNZ").inOrder()
    }

    @Test
    fun frequencySortsNumerically() {
        assertThat(calls(sortQsoTable(log, QSO_COL_FREQ, true, noState)))
            .containsExactly("KW8CW", "N5PNZ", "AA6MU").inOrder()
    }

    @Test
    fun reportSortsNumerically_blanksLast() {
        assertThat(calls(sortQsoTable(log, QSO_COL_RST_SENT, true, noState)))
            .containsExactly("AA6MU", "KW8CW", "N5PNZ").inOrder()
        assertThat(calls(sortQsoTable(log, QSO_COL_RST_SENT, false, noState)))
            .containsExactly("KW8CW", "AA6MU", "N5PNZ").inOrder()
    }

    @Test
    fun statusDescendingPutsConfirmedFirst() {
        assertThat(calls(sortQsoTable(log, QSO_COL_STATUS, false, noState)))
            .containsExactly("AA6MU", "N5PNZ", "KW8CW").inOrder()
    }

    @Test
    fun place_nonUsCountryShownAsLogged() {
        assertThat(logbookPlace(null, "", "Germany")).isEqualTo("Germany")
    }

    @Test
    fun place_usUsesGridStateThenLoggedState() {
        assertThat(logbookPlace("VA", "MD", "United States")).isEqualTo("VA")
        assertThat(logbookPlace(null, "DE", "United States")).isEqualTo("DE")
        assertThat(logbookPlace("TX", "", null)).isEqualTo("TX")
    }

    @Test
    fun place_nothingLogged_isNullForLookup() {
        assertThat(logbookPlace(null, "", null)).isNull()
    }

    @Test
    fun qslMarks_receivedBeatsSent() {
        val r = rec("K9LZJ", "20150909")
        r.isLotW_QSL = true
        r.lotwSent = true
        r.eqslSent = true
        assertThat(qslMarks(r)).containsExactly("L" to '✓', "E" to '↑', "P" to '·', "Q" to '·').inOrder()
    }

    @Test
    fun sortByQsl_ranksReceivedOverSent() {
        val none = rec("A1A", "20260101")
        val sent = rec("B1B", "20260101").apply { lotwSent = true; eqslSent = true }
        val rcvd = rec("C1C", "20260101").apply { isLotW_QSL = true; qslRcvd = true }
        val sorted = sortQsoTable(listOf(sent, rcvd, none), QSO_COL_QSL, ascending = false) { null }
        assertThat(sorted.map { it.callsign }).containsExactly("C1C", "B1B", "A1A").inOrder()
    }

    @Test
    fun statusPill_confirmedByLotwOrQrz_elseBlank() {
        val lotw = rec("L1L", "20260101", confirmed = true)
        val qrz = rec("Q1Q", "20260101").apply { qrzQsl = true }
        val eqslOnly = rec("E1E", "20260101").apply { eqslRcvd = true }
        val uploadedOnly = rec("U1U", "20260101", worked = true).apply { syncedQrz = true }
        assertThat(qsoStatusOf(lotw)).isEqualTo(QsoStatus.CONFIRMED)
        assertThat(qsoStatusOf(qrz)).isEqualTo(QsoStatus.CONFIRMED)
        // eQSL / paper show only in the QSL column; an upload alone isn't a confirmation.
        assertThat(qsoStatusOf(eqslOnly)).isNull()
        assertThat(qsoStatusOf(uploadedOnly)).isNull()
    }

    @Test
    fun statusSort_qrzConfirmedRanksWithLotw() {
        val qrz = rec("Q1Q", "20260101").apply { qrzQsl = true }
        val plain = rec("P1P", "20260102")
        val sorted = sortQsoTable(listOf(plain, qrz), QSO_COL_STATUS, ascending = false) { null }
        assertThat(sorted.map { it.callsign }).containsExactly("Q1Q", "P1P").inOrder()
    }
}
