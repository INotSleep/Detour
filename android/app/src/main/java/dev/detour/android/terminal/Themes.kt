package dev.detour.android.terminal

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import com.termux.terminal.TerminalEmulator
import com.termux.terminal.TextStyle
import kotlin.math.pow

/**
 * A terminal colour theme, the desktop's (`src/lib/themes.ts`): background, foreground,
 * cursor, selection, then the 16 ANSI colours (normal, bright). Names are the themes' own.
 */
data class TerminalTheme(val id: String, val name: String, val c: List<String>) {
    /** Colour `i` of the list as ARGB. */
    fun argb(i: Int): Int {
        val h = c[i].removePrefix("#")
        val rgb = h.take(6).toLong(16).toInt()
        val a = if (h.length >= 8) h.substring(6, 8).toInt(16) else 255
        return (a shl 24) or (rgb and 0xFFFFFF)
    }

    fun color(i: Int): Color = Color(argb(i))

    val background: Color get() = color(0)
    val foreground: Color get() = color(1)
    val cursor: Color get() = color(2)
    val selection: Color get() = if (c[3].length > 7) color(3) else color(3).copy(alpha = 0.4f)
    val isLight: Boolean get() = luminance(argb(0)) > 0.45

    companion object {
        private fun t(id: String, name: String, base: String, normal: String, bright: String) =
            TerminalTheme(id, name, "$base $normal $bright".split(" ").filter { it.isNotEmpty() })

        val all: List<TerminalTheme> = listOf(
            t("detour-dark", "Detour Dark", "#0b0c0e #d9dbe1 #8d9cff #7a8cff52", "#1b1d22 #ef6461 #4cc38a #e5a54b #7a8cff #c58af9 #56c7d8 #c9ccd5", "#5d616c #ff8582 #6fd8a4 #f2c178 #9eabff #d8acff #7fdcea #f2f3f6"),
            t("detour-light", "Detour Light", "#fbfbfc #17181c #4d5ee8 #4d5ee833", "#17181c #d6453f #2f9e66 #b7791f #4d5ee8 #9a4fd0 #1d8fa3 #8a8d96", "#4f525a #e5534b #38b174 #c98a2a #6573f0 #ad66de #2aa5ba #c9cbd1"),
            t("one-dark", "One Dark", "#282c34 #abb2bf #528bff #3e4451", "#282c34 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #abb2bf", "#5c6370 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #ffffff"),
            t("dracula", "Dracula", "#282a36 #f8f8f2 #f8f8f2 #44475a", "#21222c #ff5555 #50fa7b #f1fa8c #bd93f9 #ff79c6 #8be9fd #f8f8f2", "#6272a4 #ff6e6e #69ff94 #ffffa5 #d6acff #ff92df #a4ffff #ffffff"),
            t("nord", "Nord", "#2e3440 #d8dee9 #d8dee9 #434c5e", "#3b4252 #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #88c0d0 #e5e9f0", "#4c566a #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #8fbcbb #eceff4"),
            t("tokyo-night", "Tokyo Night", "#1a1b26 #c0caf5 #c0caf5 #33467c", "#15161e #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #a9b1d6", "#414868 #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #c0caf5"),
            t("catppuccin-mocha", "Catppuccin Mocha", "#1e1e2e #cdd6f4 #f5e0dc #45475a", "#45475a #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #bac2de", "#585b70 #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #a6adc8"),
            t("gruvbox-dark", "Gruvbox Dark", "#282828 #ebdbb2 #ebdbb2 #504945", "#282828 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #a89984", "#928374 #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2"),
            t("monokai", "Monokai", "#272822 #f8f8f2 #f8f8f0 #49483e", "#272822 #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f8f8f2", "#75715e #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f9f8f5"),
            t("tomorrow-night", "Tomorrow Night", "#1d1f21 #c5c8c6 #c5c8c6 #373b41", "#1d1f21 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #c5c8c6", "#969896 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #ffffff"),
            t("solarized-dark", "Solarized Dark", "#002b36 #839496 #93a1a1 #073642", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#586e75 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
            t("solarized-light", "Solarized Light", "#fdf6e3 #657b83 #586e75 #eee8d5", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#002b36 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
            t("one-light", "One Light", "#fafafa #383a42 #526fff #e5e5e6", "#383a42 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #a0a1a7", "#696c77 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #fafafa"),
            t("github-light", "GitHub Light", "#ffffff #24292f #0969da #b6d7ff", "#24292f #cf222e #116329 #4d2d00 #0969da #8250df #1b7c83 #6e7781", "#57606a #a40e26 #1a7f37 #633c01 #218bff #a475f9 #3192aa #8c959f"),
        )

        fun named(id: String): TerminalTheme? = all.firstOrNull { it.id == id }

        private fun luminance(argb: Int): Double {
            fun lin(v: Int): Double {
                val s = v / 255.0
                return if (s <= 0.03928) s / 12.92 else ((s + 0.055) / 1.055).pow(2.4)
            }
            return 0.2126 * lin((argb shr 16) and 255) + 0.7152 * lin((argb shr 8) and 255) + 0.0722 * lin(argb and 255)
        }
    }
}

/**
 * The theme's colours into the emulator: the 16 ANSI ones, foreground, background, cursor.
 * The view needs redrawing after (`TerminalView.invalidate()`). A program's own colours (OSC 4)
 * and a terminal reset go back to the emulator's defaults, so call it again after a reset.
 */
fun TerminalTheme.applyTo(emulator: TerminalEmulator) {
    val colors = emulator.mColors.mCurrentColors
    for (i in 0 until 16) colors[i] = argb(4 + i)
    colors[TextStyle.COLOR_INDEX_FOREGROUND] = argb(1)
    colors[TextStyle.COLOR_INDEX_BACKGROUND] = argb(0)
    colors[TextStyle.COLOR_INDEX_CURSOR] = argb(2)
}

private lateinit var preferences: SharedPreferences

/** The terminal's look and behaviour on this phone: theme, font size, output highlighting. */
object TerminalSettings {
    /** "system" follows the app (its theme, or the phone's dark or light). */
    const val SYSTEM = "system"
    const val MIN_FONT = 9f
    const val MAX_FONT = 24f

    private var _themeId by mutableStateOf(SYSTEM)
    private var _fontSize by mutableFloatStateOf(13f)
    private var _highlight by mutableStateOf(true)

    var themeId: String
        get() = _themeId
        set(v) { _themeId = v; preferences.edit().putString("terminalTheme", v).apply() }

    /** In sp. */
    var fontSize: Float
        get() = _fontSize
        set(v) { _fontSize = v.coerceIn(MIN_FONT, MAX_FONT); preferences.edit().putFloat("terminalFontSize", _fontSize).apply() }

    var highlight: Boolean
        get() = _highlight
        set(v) { _highlight = v; preferences.edit().putBoolean("terminalHighlight", v).apply() }

    /** Reads the settings; called once at start (`AppAppearance.load` does it too). */
    fun load(context: Context) {
        if (::preferences.isInitialized.not()) preferences = context.applicationContext.getSharedPreferences("settings", Context.MODE_PRIVATE)
        _themeId = preferences.getString("terminalTheme", SYSTEM) ?: SYSTEM
        _fontSize = preferences.getFloat("terminalFontSize", 13f)
        _highlight = preferences.getBoolean("terminalHighlight", true)
    }

    /** The terminal's own theme, else the app's, else Detour Dark or Light as the app is. */
    fun theme(dark: Boolean): TerminalTheme =
        TerminalTheme.named(themeId) ?: AppAppearance.theme ?: TerminalTheme.all[if (dark) 0 else 1]
}

/**
 * The app's look, as on the desktop: the phone's dark or light, or a theme. A theme sets dark
 * or light, the accent colour, and the terminal's colours while the terminal follows the app.
 */
object AppAppearance {
    private var _themeId by mutableStateOf(TerminalSettings.SYSTEM)

    var themeId: String
        get() = _themeId
        set(v) { _themeId = v; preferences.edit().putString("appTheme", v).apply() }

    fun load(context: Context) {
        TerminalSettings.load(context)
        _themeId = preferences.getString("appTheme", TerminalSettings.SYSTEM) ?: TerminalSettings.SYSTEM
    }

    val theme: TerminalTheme? get() = TerminalTheme.named(themeId)
    /** Dark or light as the theme is; null follows the phone. */
    val dark: Boolean? get() = theme?.let { !it.isLight }
    /** The theme's blue. */
    val accent: Color? get() = theme?.color(8)
}
