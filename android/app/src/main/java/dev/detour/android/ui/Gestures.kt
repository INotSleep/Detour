package dev.detour.android.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput

@OptIn(ExperimentalFoundationApi::class)
fun Modifier.combinedClick(interaction: MutableInteractionSource, onClick: (() -> Unit)?, onLongClick: (() -> Unit)?): Modifier =
    combinedClickable(interactionSource = interaction, indication = null, onClick = { onClick?.invoke() }, onLongClick = onLongClick)

/** Takes every touch, so what is under it gets none. */
fun Modifier.blockTouches(): Modifier = this.then(
    Modifier.pointerInput(Unit) {
        awaitPointerEventScope {
            while (true) {
                awaitPointerEvent(androidx.compose.ui.input.pointer.PointerEventPass.Initial).changes.forEach { it.consume() }
            }
        }
    }
)
