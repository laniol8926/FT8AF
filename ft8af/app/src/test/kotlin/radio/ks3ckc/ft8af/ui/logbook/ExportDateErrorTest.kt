package radio.ks3ckc.ft8af.ui.logbook

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/** [exportDateError]: the export sheet only runs with blank or full YYYYMMDD dates (fork). */
class ExportDateErrorTest {

    @Test
    fun blankOrFullDates_ok() {
        assertThat(exportDateError("", "")).isNull()
        assertThat(exportDateError("20261101", "20261101")).isNull()
        assertThat(exportDateError("20261027", "")).isNull()
    }

    @Test
    fun partialDate_rejected() {
        // "2026101" would match 20261028 as a string prefix comparison.
        assertThat(exportDateError("2026101", "")).isNotNull()
        assertThat(exportDateError("", "202611")).isNotNull()
    }

    @Test
    fun impossibleDate_rejected() {
        assertThat(exportDateError("20261332", "")).isNotNull()
        assertThat(exportDateError("20260230", "")).isNotNull()
    }

    @Test
    fun reversedRange_rejected() {
        assertThat(exportDateError("20261102", "20261101")).isNotNull()
    }
}
