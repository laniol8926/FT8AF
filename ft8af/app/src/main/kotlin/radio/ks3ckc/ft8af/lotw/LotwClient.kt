package radio.ks3ckc.ft8af.lotw

import android.database.sqlite.SQLiteDatabase
import com.k1af.ft8af.GeneralVariables
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.util.Locale

/**
 * LoTW confirmations download (fork). Fetches the operator's QSL'd QSOs from the ARRL
 * lotwreport.adi endpoint with their LoTW website username/password and marks the
 * matching local QSOs `isLotW_QSL = 1`.
 *
 * Download only: uploading to LoTW needs every QSO signed with the operator's TQSL
 * certificate, which a username/password can't do.
 */
object LotwClient {
    const val CONFIG_USERNAME = "lotwUsername"
    const val CONFIG_PASSWORD = "lotwPassword"

    /** Last QSL received date seen (LoTW's APP_LoTW_LASTQSL), so later downloads are incremental. */
    const val CONFIG_LAST_QSL = "lotwLastQsl"

    private const val REPORT_URL = "https://lotw.arrl.org/lotwuser/lotwreport.adi"

    /** One confirmed QSO from the report. */
    data class Qsl(
        val call: String,
        val qsoDate: String,
        val timeOn: String,
        val band: String,
        val mode: String,
        // MODE as sent (MFSK for FT4), for logs that stored that instead of the submode.
        val rawMode: String = mode,
    )

    data class Report(val qsls: List<Qsl>, val lastQsl: String?)

    data class Result(val inReport: Int, val newlyConfirmed: Int, val error: String?)

    /** Query string for the report: confirmed QSOs only, received since [since] (yyyy-MM-dd) if given. */
    internal fun buildReportUrl(username: String, password: String, since: String?): String {
        fun enc(s: String) = URLEncoder.encode(s, "UTF-8")
        val sb = StringBuilder(REPORT_URL)
            .append("?login=").append(enc(username))
            .append("&password=").append(enc(password))
            .append("&qso_query=1&qso_qsl=yes&qso_qsldetail=no")
        if (!since.isNullOrBlank()) sb.append("&qso_qslsince=").append(enc(since))
        return sb.toString()
    }

    private val FIELD = Regex("<([A-Za-z0-9_]+)(?::(\\d+))?(?::[A-Za-z])?>", RegexOption.IGNORE_CASE)

    /**
     * Parse the report ADIF. Throws [IllegalArgumentException] when the text is not an
     * ADIF report (LoTW answers a bad login with an HTML page, not an error code).
     */
    internal fun parseReport(text: String): Report {
        val eoh = text.indexOf("<eoh>", ignoreCase = true)
        if (eoh < 0) {
            throw IllegalArgumentException(
                if (text.contains("password", ignoreCase = true)) "LoTW refused the username/password"
                else "LoTW did not return a report",
            )
        }
        val header = parseFields(text.substring(0, eoh))
        val qsls = ArrayList<Qsl>()
        for (chunk in text.substring(eoh + 5).split(Regex("<eor>", RegexOption.IGNORE_CASE))) {
            val f = parseFields(chunk)
            val call = f["CALL"] ?: continue
            if (f["QSL_RCVD"]?.equals("Y", ignoreCase = true) == false) continue
            qsls += Qsl(
                call = call.uppercase(Locale.US),
                qsoDate = f["QSO_DATE"].orEmpty(),
                timeOn = f["TIME_ON"].orEmpty(),
                band = f["BAND"].orEmpty().lowercase(Locale.US),
                // FT4 comes as MODE=MFSK SUBMODE=FT4; the local log stores FT4.
                mode = (f["SUBMODE"]?.takeIf { it.isNotBlank() } ?: f["MODE"].orEmpty()).uppercase(Locale.US),
                rawMode = f["MODE"].orEmpty().uppercase(Locale.US),
            )
        }
        return Report(qsls, header["APP_LOTW_LASTQSL"])
    }

    private fun parseFields(text: String): Map<String, String> {
        val out = HashMap<String, String>()
        for (m in FIELD.findAll(text)) {
            val len = m.groupValues[2].toIntOrNull() ?: continue
            val start = m.range.last + 1
            if (start + len > text.length) continue
            out[m.groupValues[1].uppercase(Locale.US)] = text.substring(start, start + len).trim()
        }
        return out
    }

    /** "2026-10-27 13:05:00" -> "2026-10-27", the form qso_qslsince takes. */
    internal fun sinceDate(lastQsl: String?): String? =
        lastQsl?.trim()?.takeIf { it.length >= 10 }?.substring(0, 10)

    /**
     * Mark local QSOs confirmed. A QSO matches on call, date, band, mode and the HH:MM of
     * TIME_ON (logs differ on whether they keep seconds). Returns how many rows changed.
     */
    internal fun applyConfirmations(db: SQLiteDatabase, qsls: List<Qsl>): Int {
        var changed = 0
        db.beginTransaction()
        try {
            val stmt = db.compileStatement(
                "UPDATE QSLTable SET isLotW_QSL=1 WHERE isLotW_QSL=0 AND upper(call)=? " +
                    "AND qso_date=? AND lower(band)=? AND upper(mode) IN (?,?) AND substr(time_on,1,4)=?",
            )
            for (q in qsls) {
                if (q.qsoDate.isEmpty() || q.timeOn.length < 4) continue
                stmt.clearBindings()
                stmt.bindString(1, q.call)
                stmt.bindString(2, q.qsoDate)
                stmt.bindString(3, q.band)
                stmt.bindString(4, q.mode)
                stmt.bindString(5, q.rawMode)
                stmt.bindString(6, q.timeOn.substring(0, 4))
                changed += stmt.executeUpdateDelete()
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
        return changed
    }

    /**
     * Download and apply. [fullRefresh] ignores the saved last-QSL date and fetches every
     * confirmation. Blocking: call off the main thread.
     */
    fun downloadConfirmations(
        db: SQLiteDatabase,
        username: String,
        password: String,
        lastQsl: String?,
        fullRefresh: Boolean,
        saveLastQsl: (String) -> Unit,
    ): Result {
        if (username.isBlank() || password.isBlank()) return Result(0, 0, "Enter your LoTW username and password")
        return try {
            val url = buildReportUrl(username.trim(), password, if (fullRefresh) null else sinceDate(lastQsl))
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = 20_000
            conn.readTimeout = 120_000
            val text = try {
                if (conn.responseCode != 200) {
                    return Result(0, 0, "LoTW answered HTTP ${conn.responseCode}")
                }
                conn.inputStream.use { String(it.readBytes(), StandardCharsets.UTF_8) }
            } finally {
                conn.disconnect()
            }
            val report = parseReport(text)
            val changed = applyConfirmations(db, report.qsls)
            report.lastQsl?.let(saveLastQsl)
            // Never log the URL: it carries the password.
            GeneralVariables.fileLog(
                "LoTW: ${report.qsls.size} confirmations in report, $changed newly marked, last QSL ${report.lastQsl}",
            )
            Result(report.qsls.size, changed, null)
        } catch (e: IllegalArgumentException) {
            Result(0, 0, e.message)
        } catch (e: Exception) {
            GeneralVariables.fileLog("LoTW: download failed: ${e.javaClass.simpleName}")
            Result(0, 0, "Download failed: ${e.javaClass.simpleName}")
        }
    }
}
