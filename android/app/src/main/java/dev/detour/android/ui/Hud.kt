package dev.detour.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

/** A short note that something was done ("Copied"), or why it wasn't, at the top of the screen for a moment. */
object Hud {
    var text by mutableStateOf<String?>(null)
        private set
    var shown by mutableIntStateOf(0)
        private set
    var failed by mutableStateOf(false)
        private set

    fun show(text: String) {
        this.text = text
        failed = false
        shown++
    }

    fun error(text: String) {
        this.text = text
        failed = true
        shown++
    }
}

@Composable
fun HudView() {
    val haptic = LocalHapticFeedback.current
    var visible by androidx.compose.runtime.remember { mutableStateOf(false) }
    LaunchedEffect(Hud.shown) {
        if (Hud.shown == 0) return@LaunchedEffect
        haptic.performHapticFeedback(if (Hud.failed) HapticFeedbackType.Reject else HapticFeedbackType.Confirm)
        visible = true
        delay(if (Hud.failed) 3000 else 1400)
        visible = false
    }
    Box(Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).padding(top = 8.dp), contentAlignment = Alignment.TopCenter) {
        AnimatedVisibility(visible, enter = fadeIn() + scaleIn(initialScale = 0.85f), exit = fadeOut() + scaleOut(targetScale = 0.9f)) {
            Row(
                Modifier.shadow(10.dp, CircleShape).clip(CircleShape).background(colors.cell).padding(horizontal = 18.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (Hud.failed) Icon(Icons.Outlined.ErrorOutline, null, tint = colors.red, modifier = Modifier.size(18.dp))
                else Icon(Icons.Outlined.Check, null, tint = colors.green, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(Hud.text ?: "", color = colors.label, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, modifier = Modifier.widthIn(max = 300.dp))
            }
        }
    }
}
