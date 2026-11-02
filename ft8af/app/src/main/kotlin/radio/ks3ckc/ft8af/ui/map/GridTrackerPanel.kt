package radio.ks3ckc.ft8af.ui.map

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.Observer
import com.k1af.ft8af.GeneralVariables
import com.k1af.ft8af.MainViewModel
import com.k1af.ft8af.Ft8Message
import com.k1af.ft8af.grid_tracker.GridOsmMapView
import org.osmdroid.views.MapView

private const val PREFS = "ft8af_map"
// "tracker" or "classic". A string (not a boolean) so the settings backup, which
// carries string prefs, keeps it; a new key so every install starts on the tracker.
private const val KEY_MAP_STYLE = "mapStyle"

/**
 * Whether the map card shows the grid tracker (the default, fork) rather than the
 * original FT8AF map, which the Tracker button still switches back to.
 */
internal fun loadTrackerMap(context: Context): Boolean =
    runCatching {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(KEY_MAP_STYLE, "tracker") != "classic"
    }.getOrDefault(true)

internal fun saveTrackerMap(context: Context, on: Boolean) {
    context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
        .putString(KEY_MAP_STYLE, if (on) "tracker" else "classic").apply()
}

/**
 * The FT8CN / FT8TW grid tracker map (osmdroid, offline night tiles, lines between
 * stations in QSO, CQ markers, worked/confirmed grids) inside the Map tab's card (fork).
 * Drives [GridOsmMapView] the way GridTrackerMainActivity does, minus its own UI chrome.
 */
@Composable
internal fun GridTrackerPanel(mainViewModel: MainViewModel, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val mapView = remember { MapView(context) }
    val tracker = remember { GridOsmMapView(context, mapView, mainViewModel) }

    DisposableEffect(lifecycleOwner) {
        // One-line boxes: just the message (callsign and grid), no detail lines.
        tracker.setCompactInfo(true)
        tracker.initMap(GeneralVariables.getMyMaidenhead4Grid(), true)
        // Label the QSO lines too, not only CQs (GridOsmMapView defaults QSX labels off;
        // the full-screen tracker turns them on from its own settings bar).
        tracker.setShowQSX(true)

        // No grid shading (unlike the full-screen tracker): with a big log the worked /
        // confirmed / known grids cover the map. Only the current slot's lines and CQs show.

        // The current slot only (the newest one decoded), redrawn whole on every pass so
        // it matches the newest group in the decode list. Main thread only.
        val recentSlots = java.util.TreeMap<Long, List<Ft8Message>>()
        val messagesObserver = Observer<ArrayList<Ft8Message>> {
            val current = mainViewModel.currentMessages ?: return@Observer
            val slotMs = GeneralVariables.currentMode().slotMillis.toLong().coerceAtLeast(1L)
            ArrayList(current).groupBy { it.utcTime / slotMs }.forEach { (slot, msgs) ->
                recentSlots[slot] = msgs
            }
            if (recentSlots.isEmpty()) return@Observer
            val newest = recentSlots.lastKey()
            recentSlots.headMap(newest).clear()

            tracker.hideInfoWindows()
            tracker.clearLines()
            tracker.clearMarkers()
            // Newest message per station pair: one box each.
            val drawn = HashSet<String>()
            for (msg in recentSlots.descendingMap().values.flatten()) {
                val key = "${msg.callsignFrom}|${msg.callsignTo}"
                if (!drawn.add(key)) continue
                // No upgradeGridInfo: it shades each decoded grid and never clears it.
                tracker.drawLine(msg, mainViewModel.databaseOpr)
                if (msg.checkIsCQ()) {
                    tracker.addGridMarker(msg.getMaidenheadGrid(mainViewModel.databaseOpr), msg)
                }
            }
            tracker.showInfoWindows()
        }
        mainViewModel.mutableFt8MessageList.observe(lifecycleOwner, messagesObserver)

        val lifecycleObserver = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> mapView.onResume()
                Lifecycle.Event.ON_PAUSE -> mapView.onPause()
                else -> Unit
            }
        }
        lifecycleOwner.lifecycle.addObserver(lifecycleObserver)
        mapView.onResume()

        onDispose {
            mainViewModel.mutableFt8MessageList.removeObserver(messagesObserver)
            lifecycleOwner.lifecycle.removeObserver(lifecycleObserver)
            mapView.onPause()
            mapView.onDetach()
        }
    }

    AndroidView(factory = { mapView }, modifier = modifier)
}
