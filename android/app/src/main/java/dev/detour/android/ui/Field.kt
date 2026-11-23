package dev.detour.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** A rounded text field with its placeholder inside, as iOS draws one. */
@Composable
fun Field(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    secure: Boolean = false,
    mono: Boolean = false,
    plain: Boolean = false,
    number: Boolean = false,
    singleLine: Boolean = true,
    ime: ImeAction = ImeAction.Done,
    onDone: () -> Unit = {},
    fill: Color? = null,
) {
    BasicTextField(
        value, onChange,
        modifier = modifier.fillMaxWidth(),
        singleLine = singleLine,
        textStyle = TextStyle(color = colors.label, fontSize = 17.sp, fontFamily = if (mono) FontFamily.Monospace else null),
        cursorBrush = SolidColor(colors.accent),
        visualTransformation = if (secure) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = when { secure -> KeyboardType.Password; number -> KeyboardType.Number; plain -> KeyboardType.Ascii; else -> KeyboardType.Text },
            capitalization = if (plain || secure) KeyboardCapitalization.None else KeyboardCapitalization.Sentences,
            autoCorrectEnabled = !(plain || secure || mono),
            imeAction = ime,
        ),
        keyboardActions = KeyboardActions(onAny = { onDone() }),
        decorationBox = { inner ->
            Box(
                Modifier.clip(RoundedCornerShape(10.dp)).background(fill ?: colors.cell).heightIn(min = 44.dp).padding(horizontal = 14.dp, vertical = 11.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (value.isEmpty()) Text(placeholder, color = colors.tertiary, fontSize = 17.sp)
                inner()
            }
        },
    )
}
