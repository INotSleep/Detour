package dev.detour.android.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** A text field as a row of a form section. */
@Composable
fun FieldCell(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    last: Boolean = false,
    secure: Boolean = false,
    mono: Boolean = false,
    plain: Boolean = false,
    number: Boolean = false,
    singleLine: Boolean = true,
    modifier: Modifier = Modifier,
    ime: ImeAction = ImeAction.Next,
    onDone: () -> Unit = {},
) {
    Column {
        BasicTextField(
            value, onChange,
            modifier = modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 16.dp, vertical = 11.dp),
            singleLine = singleLine,
            textStyle = TextStyle(color = colors.label, fontSize = 17.sp, fontFamily = if (mono) FontFamily.Monospace else null),
            cursorBrush = SolidColor(colors.accent),
            visualTransformation = if (secure) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(
                keyboardType = when { secure -> KeyboardType.Password; number -> KeyboardType.Number; plain -> KeyboardType.Uri; else -> KeyboardType.Text },
                capitalization = if (plain || secure || mono) KeyboardCapitalization.None else KeyboardCapitalization.Sentences,
                autoCorrectEnabled = !(plain || secure || mono),
                imeAction = ime,
            ),
            keyboardActions = KeyboardActions(onAny = { onDone() }),
            decorationBox = { inner ->
                Box(contentAlignment = Alignment.CenterStart) {
                    if (value.isEmpty()) Text(placeholder, color = colors.tertiary, fontSize = 17.sp)
                    inner()
                }
            },
        )
        if (!last) HorizontalDivider(Modifier.padding(start = 16.dp), thickness = 0.5.dp, color = colors.separator)
    }
}

/** A row with a title and a switch. */
@Composable
fun ToggleCell(title: String, on: Boolean, last: Boolean = false, onChange: (Boolean) -> Unit) {
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val change = { v: Boolean ->
        haptic.performHapticFeedback(if (v) androidx.compose.ui.hapticfeedback.HapticFeedbackType.ToggleOn else androidx.compose.ui.hapticfeedback.HapticFeedbackType.ToggleOff)
        onChange(v)
    }
    Cell(last = last, onClick = { change(!on) }, padding = 6.dp) {
        Text(title, color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
        IosSwitch(on, change)
    }
}

/**
 * iOS's switch, 51 by 31: Material's is bigger and wants a 48dp touch area, which made a row with
 * one taller than the rest. Its row takes the taps too.
 */
@Composable
fun IosSwitch(on: Boolean, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val pos by androidx.compose.animation.core.animateFloatAsState(if (on) 1f else 0f, androidx.compose.animation.core.spring(dampingRatio = 0.8f, stiffness = 700f), label = "switch")
    val track = androidx.compose.ui.graphics.lerp(if (colors.dark) Color(0xFF39393D) else Color(0xFFE9E9EA), colors.green, pos)
    Box(
        modifier.width(51.dp).height(31.dp).clip(RoundedCornerShape(16.dp)).background(track)
            .clickable(remember { MutableInteractionSource() }, null) { onChange(!on) }
            .padding(2.dp),
    ) {
        Box(
            Modifier.offset(x = 20.dp * pos).size(27.dp)
                .shadow(2.dp, androidx.compose.foundation.shape.CircleShape)
                .background(Color.White, androidx.compose.foundation.shape.CircleShape)
        )
    }
}

/** A row with a title and its value on the right. */
@Composable
fun ValueCell(title: String, value: String, last: Boolean = false, onClick: (() -> Unit)? = null) {
    Cell(last = last, onClick = onClick) {
        Text(title, color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
        Text(value, color = colors.secondary, fontSize = 17.sp, maxLines = 1)
    }
}

/** iOS's segmented control: the chosen segment on a raised thumb that slides. */
@Composable
fun <T> Segmented(options: List<Pair<T, String>>, selection: T, modifier: Modifier = Modifier, onSelect: (T) -> Unit) {
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val index = options.indexOfFirst { it.first == selection }.coerceAtLeast(0)
    val pos by animateFloatAsState(index.toFloat(), label = "segment")
    BoxWithConstraints(modifier.fillMaxWidth().height(32.dp).clip(RoundedCornerShape(9.dp)).background(colors.fill)) {
        val w = maxWidth / options.size
        Box(
            Modifier.offset(x = w * pos).width(w).fillMaxHeight().padding(2.dp)
                .shadow(2.dp, RoundedCornerShape(7.dp)).clip(RoundedCornerShape(7.dp))
                .background(if (colors.dark) Color(0xFF636366) else Color.White)
        )
        Row(Modifier.fillMaxWidth().fillMaxHeight()) {
            options.forEach { (value, title) ->
                Box(
                    Modifier.weight(1f).fillMaxHeight().clickable(remember { MutableInteractionSource() }, null) {
                        if (value != selection) haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.SegmentTick)
                        onSelect(value)
                    },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(title, color = colors.label, fontSize = 13.sp, fontWeight = if (value == selection) FontWeight.SemiBold else FontWeight.Medium, textAlign = TextAlign.Center, maxLines = 1)
                }
            }
        }
    }
}
