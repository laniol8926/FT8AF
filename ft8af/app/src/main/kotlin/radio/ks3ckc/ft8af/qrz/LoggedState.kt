@file:JvmName("LoggedState")

package radio.ks3ckc.ft8af.qrz

/**
 * The STATE to log for a QSO made in the app (fork).
 *
 * QRZ's state is the station's home address: exact, but wrong when the station is
 * operating away from home. So it's used only when QRZ's grid agrees with the grid the
 * station sent (same 4 characters) and the QSO isn't portable: not a POTA/park QSO and
 * no /P, /M, /MM or /R on the call. Otherwise the state comes from the worked grid.
 *
 * @param gridState state looked up from the worked grid (US grids only), or null
 * @param qrzState  QRZ's state for the call, or null
 * @param qrzGrid   QRZ's grid for the call, or null
 * @param workedGrid the grid the station sent / was logged with
 * @param portable  POTA/park QSO or portable suffix
 */
fun pickLoggedState(
    gridState: String?,
    qrzState: String?,
    qrzGrid: String?,
    workedGrid: String?,
    portable: Boolean,
): String? {
    val q = qrzState?.trim()?.uppercase()?.takeIf { it.isNotEmpty() }
    val sameGrid = qrzGrid != null && workedGrid != null &&
        qrzGrid.length >= 4 && workedGrid.length >= 4 &&
        qrzGrid.substring(0, 4).equals(workedGrid.substring(0, 4), ignoreCase = true)
    if (!portable && q != null && (sameGrid || workedGrid.isNullOrBlank())) return q
    return gridState?.takeIf { it.isNotBlank() }
}

/** True for a /P, /M, /MM, /R or /QRP style suffix (operating away from home). */
fun isPortableCall(call: String?): Boolean {
    val parts = call?.uppercase()?.split('/') ?: return false
    return parts.drop(1).any { it in setOf("P", "M", "MM", "R", "AM", "QRP") }
}
