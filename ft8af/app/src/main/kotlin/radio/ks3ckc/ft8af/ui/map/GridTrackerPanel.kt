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

        // Grids heard, then worked / confirmed from the log, as the activity seeds them.
        for (grid in GeneralVariables.callsignAndGrids.values) {
            tracker.upgradeGridMode(grid, GridOsmMapView.GridMode.QSX)
        }
        mainViewModel.databaseOpr.getQsoGridQuery { grids ->
            mapView.post {
                for ((grid, confirmed) in grids) {
                    tracker.upgradeGridMode(
                        grid,
                        if (confirmed) GridOsmMapView.GridMode.QSL else GridOsmMapView.GridMode.QSO,
                    )
                }
            }
        }

        val decodingObserver = Observer<Boolean> { decoding ->
            if (decoding == true) {
                tracker.clearLines()
                tracker.clearMarkers()
            }
        }
        val messagesObserver = Observer<ArrayList<Ft8Message>> {
            val current = mainViewModel.currentMessages ?: return@Observer
            for (msg in ArrayList(current)) {
                val grid = msg.getMaidenheadGrid(mainViewModel.databaseOpr)
                tracker.upgradeGridInfo(
                    grid, msg.messageText,
                    String.format(java.util.Locale.US, "%d dBm , %.1f ms", msg.snr, msg.time_sec),
                )
                tracker.drawLine(msg, mainViewModel.databaseOpr)
                if (msg.checkIsCQ()) tracker.addGridMarker(grid, msg)
            }
            tracker.showInfoWindows()
        }
        val newGridObserver = Observer<String> { grid ->
            tracker.upgradeGridMode(grid, GridOsmMapView.GridMode.QSX)
        }
        mainViewModel.mutableIsDecoding.observe(lifecycleOwner, decodingObserver)
        mainViewModel.mutableFt8MessageList.observe(lifecycleOwner, messagesObserver)
        GeneralVariables.mutableNewGrid.observe(lifecycleOwner, newGridObserver)

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
            GeneralVariables.mutableNewGrid.removeObserver(newGridObserver)
            lifecycleOwner.lifecycle.removeObserver(lifecycleObserver)
            mapView.onPause()
            mapView.onDetach()
        }
    }

    AndroidView(factory = { mapView }, modifier = modifier)
}
