package dev.detour.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** The iOS system colours the app is drawn with on iPhone, so both look the same. */
@Immutable
data class Palette(
    val dark: Boolean,
    val accent: Color,
    /** The screen behind grouped lists. */
    val background: Color,
    /** A grouped list's rows. */
    val cell: Color,
    val label: Color,
    val secondary: Color,
    val tertiary: Color,
    val separator: Color,
    /** Buttons and keys on a cell or a bar (iOS's tertiarySystemFill). */
    val fill: Color,
    /** The tab bar and the bars over the keyboard. */
    val bar: Color,
    val red: Color,
    val green: Color,
    val orange: Color,
)

val DarkPalette = Palette(
    dark = true,
    accent = Color(0xFF0A84FF),
    background = Color(0xFF000000),
    cell = Color(0xFF1C1C1E),
    label = Color(0xFFFFFFFF),
    secondary = Color(0x99EBEBF5),
    tertiary = Color(0x4DEBEBF5),
    separator = Color(0xFF38383A),
    fill = Color(0x3D767680),
    bar = Color(0xFF121212),
    red = Color(0xFFFF453A),
    green = Color(0xFF30D158),
    orange = Color(0xFFFF9F0A),
)

val LightPalette = Palette(
    dark = false,
    accent = Color(0xFF007AFF),
    background = Color(0xFFF2F2F7),
    cell = Color(0xFFFFFFFF),
    label = Color(0xFF000000),
    secondary = Color(0x993C3C43),
    tertiary = Color(0x4D3C3C43),
    separator = Color(0xFFC6C6C8),
    fill = Color(0x1F767680),
    bar = Color(0xFFF9F9F9),
    red = Color(0xFFFF3B30),
    green = Color(0xFF34C759),
    orange = Color(0xFFFF9500),
)

val LocalPalette = staticCompositionLocalOf { DarkPalette }

/** The palette in use. */
val colors: Palette @Composable get() = LocalPalette.current

@Composable
fun DetourTheme(dark: Boolean = isSystemInDarkTheme(), accent: Color? = null, content: @Composable () -> Unit) {
    val base = if (dark) DarkPalette else LightPalette
    val p = if (accent != null) base.copy(accent = accent) else base
    // Menus and dialogs on the cards' colour, as iOS draws them (no Material tint).
    val base2 = if (dark) darkColorScheme() else lightColorScheme()
    val scheme = base2.copy(
        primary = p.accent, onPrimary = Color.White, secondary = p.accent, tertiary = p.accent,
        background = p.background, onBackground = p.label,
        surface = p.cell, onSurface = p.label, onSurfaceVariant = p.secondary, surfaceVariant = p.cell,
        surfaceContainerLowest = p.cell, surfaceContainerLow = p.cell, surfaceContainer = p.cell,
        surfaceContainerHigh = p.cell, surfaceContainerHighest = p.cell, surfaceTint = Color.Transparent,
        outline = p.separator, outlineVariant = p.separator, error = p.red,
    )
    CompositionLocalProvider(LocalPalette provides p) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
