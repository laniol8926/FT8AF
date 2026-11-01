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
private const val KEY_TRACKER = "trackerMap"

/** Whether the map card shows the grid tracker instead of the Compose map (fork). */
internal fun loadTrackerMap(context: Context): Boolean =
    runCatching { context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY_TRACKER, false) }
        .getOrDefault(false)

internal fun saveTrackerMap(context: Context, on: Boolean) {
    context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY_TRACKER, on).apply()
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
        tracker.initMap(GeneralVariables.getMyMaidenhead4Grid(), true)
        // Label the QSO lines too, not only CQs (GridOsmMapView defaults QSX labels off;
        // the full-screen tracker turns them on from its own settings bar).
        tracker.setShowQSX(true)

        // No grid shading (unlike the full-screen tracker): with a big log the worked /
        // confirmed / known grids cover the map. Only this cycle's lines and CQs show.

        val decodingObserver = Observer<Boolean> { decoding ->
            if (decoding == true) {
                tracker.hideInfoWindows()
                tracker.clearLines()
                tracker.clearMarkers()
            }
        }
        val messagesObserver = Observer<ArrayList<Ft8Message>> {
            val current = mainViewModel.currentMessages ?: return@Observer
            // The waterfall label list can still carry the previous slot's decodes into
            // this one (a deep pass merges onto it); draw only the newest slot, so last
            // slot's CQ bubbles don't reappear after the clear.
            val slotMs = GeneralVariables.currentMode().slotMillis.toLong().coerceAtLeast(1L)
            val messages = ArrayList(current)
            val newestSlot = messages.maxOfOrNull { it.utcTime / slotMs } ?: return@Observer
            for (msg in messages.filter { it.utcTime / slotMs == newestSlot }) {
                // No upgradeGridInfo: it shades each decoded grid and never clears it.
                tracker.drawLine(msg, mainViewModel.databaseOpr)
                if (msg.checkIsCQ()) {
                    tracker.addGridMarker(msg.getMaidenheadGrid(mainViewModel.databaseOpr), msg)
                }
            }
            tracker.showInfoWindows()
        }
        mainViewModel.mutableIsDecoding.observe(lifecycleOwner, decodingObserver)
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
            mainViewModel.mutableIsDecoding.removeObserver(decodingObserver)
            mainViewModel.mutableFt8MessageList.removeObserver(messagesObserver)
            lifecycleOwner.lifecycle.removeObserver(lifecycleObserver)
            mapView.onPause()
            mapView.onDetach()
        }
    }

    AndroidView(factory = { mapView }, modifier = modifier)
}
