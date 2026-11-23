package dev.detour.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.terminal.TerminalSettings
import dev.detour.android.terminal.TerminalTheme

/** The themes, each shown in its own colours; the first row follows the system (or the app). */
@Composable
fun ThemePickerPage(title: String, selection: String, system: String, back: String, onBack: () -> Unit, select: (String) -> Unit) {
    PushedPage(title, back, onBack) {
        section(key = "themes") {
            ThemeRow(TerminalSettings.SYSTEM, system, null, selection, last = false, select)
            TerminalTheme.all.forEachIndexed { i, th -> ThemeRow(th.id, th.name, th, selection, i == TerminalTheme.all.lastIndex, select) }
        }
    }
}

/** The name of a theme setting: the theme's, or what "system" means there. */
fun themeName(id: String, system: String): String =
    if (id == TerminalSettings.SYSTEM) system else TerminalTheme.named(id)?.name ?: system

@Composable
private fun ThemeRow(id: String, name: String, theme: TerminalTheme?, selection: String, last: Boolean, select: (String) -> Unit) {
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    Cell(last = last, onClick = {
        if (selection != id) haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.Confirm)
        select(id)
    }, inset = 92) {
        Swatch(theme)
        Spacer(Modifier.width(12.dp))
        Text(name, color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
        // The check pops in on the theme picked: seen even past the finger that picked it.
        androidx.compose.animation.AnimatedVisibility(
            selection == id,
            enter = androidx.compose.animation.scaleIn(androidx.compose.animation.core.spring(dampingRatio = 0.5f, stiffness = 600f)) + androidx.compose.animation.fadeIn(),
            exit = androidx.compose.animation.fadeOut(),
        ) { Icon(Icons.Outlined.Check, null, tint = colors.accent, modifier = Modifier.size(20.dp)) }
    }
}

/** The background with a prompt in the theme's own colours. */
@Composable
private fun Swatch(theme: TerminalTheme?) {
    val shape = RoundedCornerShape(6.dp)
    val frame = Modifier.size(64.dp, 32.dp).clip(shape).border(0.5.dp, colors.secondary.copy(alpha = 0.3f), shape)
    if (theme != null) {
        Box(frame.background(theme.background), contentAlignment = Alignment.Center) {
            Text(
                buildAnnotatedString {
                    withStyle(SpanStyle(color = theme.foreground)) { append("$ ") }
                    withStyle(SpanStyle(color = theme.color(8))) { append("~") }
                    withStyle(SpanStyle(color = theme.color(6))) { append(" ok") }
                },
                fontSize = 13.sp, fontFamily = FontFamily.Monospace, maxLines = 1,
            )
        }
    } else {
        Row(frame) {
            Box(Modifier.weight(1f).fillMaxHeight().background(TerminalTheme.all[0].background))
            Box(Modifier.weight(1f).fillMaxHeight().background(TerminalTheme.all[1].background))
        }
    }
}
