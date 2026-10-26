package radio.ks3ckc.ft8af.ui.components

import androidx.annotation.StringRes
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInParent
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import radio.ks3ckc.ft8af.theme.*
import radio.ks3ckc.ft8af.ui.motion.MotionTokens
import radio.ks3ckc.ft8af.ui.motion.rememberHaptics
import com.k1af.ft8af.R

enum class FT8AFTab(@StringRes val labelRes: Int) {
    DECODE(R.string.tab_decode),
    MAP(R.string.tab_map),
    WATERFALL(R.string.tab_waterfall),
    POTA(R.string.tab_pota),
    LOG(R.string.tab_logbook),
    SETTINGS(R.string.tab_settings),
}

@Composable
fun TabBar(
    activeTab: FT8AFTab,
    onTabSelected: (FT8AFTab) -> Unit,
    modifier: Modifier = Modifier,
    // Landscape: an extra "More controls" entry after the tabs that opens/closes the operate
    // controls drawer, replacing the drawer's own handle strip there. Null hides it.
    onToggleMoreControls: (() -> Unit)? = null,
    moreControlsOpen: Boolean = false,
    // Landscape: icon beside its label instead of above it, and much less padding (the 30dp
    // under the labels is spare room for the hidden navigation bar's swipe area), so the bar
    // takes roughly half its portrait height on the short screen.
    compact: Boolean = false,
) {
    val itemPadding = if (compact) 4.dp else 8.dp
    val haptics = rememberHaptics()
    val tabBounds = remember { mutableStateMapOf<FT8AFTab, Pair<Float, Float>>() } // center x, width (px)

    LaunchedEffect(activeTab) {
        haptics.tick()
    }

    val activeBounds = tabBounds[activeTab]
    val targetCenter = activeBounds?.first ?: 0f
    val targetWidth = activeBounds?.second ?: 0f

    val animatedCenter by animateFloatAsState(
        targetValue = targetCenter,
        animationSpec = MotionTokens.SpringSmooth,
        label = "tab-indicator-center"
    )
    val animatedWidth by animateFloatAsState(
        targetValue = targetWidth,
        animationSpec = MotionTokens.SpringSmooth,
        label = "tab-indicator-width"
    )

    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(
                Brush.verticalGradient(
                    listOf(Color.Transparent, BgApp.copy(alpha = 0.95f)),
                    startY = 0f,
                    endY = 40f,
                )
            )
            .padding(horizontal = 12.dp)
            .padding(top = if (compact) 4.dp else 8.dp, bottom = if (compact) 8.dp else 30.dp)
            // drawBehind sits AFTER both padding modifiers so its draw scope matches the inner
            // content area — the same coordinate space the tab Columns report from
            // [LayoutCoordinates.positionInParent]. This keeps the pill horizontally aligned
            // with the active tab's icon regardless of padding values.
            .drawBehind {
                if (animatedWidth <= 0f) return@drawBehind
                // Compact items put the label beside the icon, so the pill is wider to cover both.
                val pillWidthPx = animatedWidth * if (compact) 0.8f else 0.56f
                // The Row's inner content height = icon (~24dp) + 4dp gap + label, but for the
                // pill we want a halo around the icon only. The icon is at the top of each
                // Column with vertical padding 8dp, so center the pill on roughly the icon's
                // vertical center. (Compact: the icon is centred in a single-line item.)
                val pillHeightPx = 28.dp.toPx()
                val iconCenterY = itemPadding.toPx() + 12.dp.toPx() // item padding + half of a 24dp icon
                val pillTop = iconCenterY - pillHeightPx / 2f
                val cx = animatedCenter
                val left = cx - pillWidthPx / 2f
                drawRoundRect(
                    color = Accent.copy(alpha = 0.14f),
                    topLeft = Offset(left, pillTop),
                    size = Size(pillWidthPx, pillHeightPx),
                    cornerRadius = CornerRadius(pillHeightPx / 2f, pillHeightPx / 2f),
                )
                // Tiny accent dot just above the pill.
                drawCircle(
                    color = Accent,
                    radius = 1.5.dp.toPx(),
                    center = Offset(cx, pillTop - 4.dp.toPx()),
                )
            },
        horizontalArrangement = Arrangement.SpaceEvenly,
    ) {
        for (tab in FT8AFTab.entries) {
            val isActive = tab == activeTab

            val color by animateColorAsState(
                targetValue = if (isActive) Accent else TextFaint,
                animationSpec = tween(MotionTokens.DurMed, easing = MotionTokens.EasingStandard),
                label = "tab-color"
            )
            val strokeWidth = if (isActive) 1.8f else 1.5f

            // Bounce animation when this tab becomes active
            val bounceScale = remember { Animatable(1f) }
            LaunchedEffect(isActive) {
                if (isActive) {
                    bounceScale.snapTo(1f)
                    bounceScale.animateTo(1.18f, tween(140, easing = MotionTokens.EasingEmphasizedDecel))
                    bounceScale.animateTo(1.12f, MotionTokens.SpringSmooth)
                } else {
                    bounceScale.animateTo(1f, MotionTokens.SpringSmooth)
                }
            }

            TabItem(
                compact = compact,
                modifier = Modifier
                    .weight(1f)
                    .onGloballyPositioned { coords ->
                        val cx = coords.positionInParent().x + coords.size.width / 2f
                        tabBounds[tab] = cx to coords.size.width.toFloat()
                    }
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) { onTabSelected(tab) }
                    .padding(vertical = itemPadding, horizontal = 4.dp),
            ) {
                Box(
                    modifier = Modifier.graphicsLayer {
                        scaleX = bounceScale.value
                        scaleY = bounceScale.value
                    }
                ) {
                    when (tab) {
                        FT8AFTab.DECODE -> FT8AFIcons.Decode(color = color, strokeWidth = strokeWidth)
                        FT8AFTab.MAP -> FT8AFIcons.Globe(color = color, strokeWidth = strokeWidth)
                        FT8AFTab.WATERFALL -> FT8AFIcons.Waterfall(color = color, strokeWidth = strokeWidth)
                        FT8AFTab.POTA -> FT8AFIcons.Tree(color = color, strokeWidth = strokeWidth)
                        FT8AFTab.LOG -> FT8AFIcons.Book(color = color, strokeWidth = strokeWidth)
                        FT8AFTab.SETTINGS -> FT8AFIcons.Cog(color = color, strokeWidth = strokeWidth)
                    }
                }
                Text(
                    text = stringResource(tab.labelRes),
                    color = color,
                    fontSize = 10.5.sp,
                    fontWeight = if (isActive) FontWeight.SemiBold else FontWeight.Medium,
                    letterSpacing = 0.02.sp,
                    maxLines = 1,
                    softWrap = false,
                )
            }
        }

        if (onToggleMoreControls != null) {
            // Not a tab (it opens the drawer over the current screen rather than switching
            // screens), so it gets no selection pill; it turns amber while the drawer is open.
            val color = if (moreControlsOpen) Accent else TextFaint
            TabItem(
                compact = compact,
                modifier = Modifier
                    .weight(1f)
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = onToggleMoreControls,
                    )
                    .padding(vertical = itemPadding, horizontal = 4.dp),
            ) {
                FT8AFIcons.Sliders(color = color, strokeWidth = if (moreControlsOpen) 1.8f else 1.5f)
                Text(
                    text = stringResource(R.string.controls_drawer_more),
                    color = color,
                    fontSize = 10.5.sp,
                    fontWeight = if (moreControlsOpen) FontWeight.SemiBold else FontWeight.Medium,
                    letterSpacing = 0.02.sp,
                    maxLines = 1,
                    softWrap = false,
                )
            }
        }
    }
}

/** One tab-bar entry: icon above label, or ([compact]) icon beside label on one line. */
@Composable
private fun TabItem(
    compact: Boolean,
    modifier: Modifier,
    content: @Composable () -> Unit,
) {
    if (compact) {
        Row(
            modifier = modifier,
            horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) { content() }
    } else {
        Column(
            modifier = modifier,
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) { content() }
    }
}
