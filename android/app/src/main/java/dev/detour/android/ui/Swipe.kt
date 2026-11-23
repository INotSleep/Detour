package dev.detour.android.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** A button behind a row, shown by swiping the row to the left. */
data class SwipeAction(val label: String, val icon: ImageVector, val color: Color, val run: () -> Unit)

@Composable
fun deleteAction(run: () -> Unit) = SwipeAction(dev.detour.android.core.t("Delete"), Icons.Outlined.Delete, colors.red, run)

@Composable
fun editAction(run: () -> Unit) = SwipeAction(dev.detour.android.core.t("Edit"), Icons.Outlined.Edit, colors.accent, run)

@Composable
fun closeAction(run: () -> Unit) = SwipeAction(dev.detour.android.core.t("Close"), Icons.Outlined.Close, colors.red, run)

/** The row whose buttons are out: one at a time, as in iOS lists. */
private object OpenRow {
    var id by mutableStateOf<Any?>(null)
}

/**
 * iOS's swipe actions: drag the row left and its buttons show at the right (the first one at the
 * edge); a tap on one runs it, a tap on the row or a drag back hides them. A full swipe runs
 * nothing (iOS `allowsFullSwipe: false`).
 */
@Composable
fun SwipeActions(actions: List<SwipeAction>, content: @Composable () -> Unit) {
    if (actions.isEmpty()) {
        content()
        return
    }
    val density = LocalDensity.current
    val haptic = LocalHapticFeedback.current
    val buttonWidth = with(density) { 74.dp.toPx() }
    val open = buttonWidth * actions.size
    val offset = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val me = remember { Any() }

    fun settle(out: Boolean) = scope.launch {
        if (out) OpenRow.id = me
        else if (OpenRow.id === me) OpenRow.id = null
        offset.animateTo(if (out) -open else 0f, spring(dampingRatio = 0.85f, stiffness = 500f))
    }

    // Another row opened: this one closes.
    LaunchedEffect(OpenRow.id) { if (OpenRow.id !== me && offset.value != 0f) offset.animateTo(0f) }

    Box {
        // The buttons, revealed from the right as the row moves.
        Row(Modifier.matchParentSize(), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.End) {
            val shown = (-offset.value).coerceAtLeast(0f)
            val each = with(density) { (shown / actions.size).toDp() }
            // The first action sits at the edge, as iOS puts the destructive one.
            for (a in actions.reversed()) {
                Box(
                    Modifier.width(each).fillMaxHeight().background(a.color).clickable {
                        settle(false)
                        a.run()
                    },
                    contentAlignment = Alignment.Center,
                ) {
                    if (each > 40.dp) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Icon(a.icon, null, tint = Color.White, modifier = Modifier.size(22.dp))
                            Text(a.label, color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium, maxLines = 1)
                        }
                    }
                }
            }
        }
        Box(
            Modifier
                .offset { IntOffset(offset.value.roundToInt(), 0) }
                .background(colors.cell)
                .draggable(
                    rememberDraggableState { d ->
                        scope.launch {
                            val next = offset.value + d
                            // Past its buttons the row gives, more and more slowly.
                            val bounded = if (next < -open) -open + (next + open) / 3 else next.coerceAtMost(0f)
                            offset.snapTo(bounded)
                        }
                    },
                    Orientation.Horizontal,
                    onDragStarted = { if (OpenRow.id !== me) OpenRow.id = null },
                    onDragStopped = { v ->
                        val out = offset.value < -open / 2 || v < -800f
                        if (out && OpenRow.id !== me) haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                        settle(out && v < 800f)
                    },
                )
        ) {
            content()
            // With the buttons out, a tap on the row only puts them away.
            if (offset.value != 0f) {
                Box(Modifier.matchParentSize().clickable(remember { androidx.compose.foundation.interaction.MutableInteractionSource() }, null) { settle(false) })
            }
        }
    }
}
