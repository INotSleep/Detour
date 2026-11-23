package dev.detour.android.keys

import dev.detour.android.terminal.TerminalSession

/** What keys send, as xterm does: named keys, function keys and their modified forms. */
object KeyCodes {
    /** Named keys a combination or a button can be, in the order the pickers show them. */
    val named = listOf("esc", "tab", "enter", "backspace", "delete", "insert", "space", "up", "down", "left", "right", "home", "end", "pageup", "pagedown") +
        (1..24).map { "f$it" }

    private const val ESC = "\u001b"

    /**
     * The bytes of `key` (a named key or one character) with modifiers. `appCursor`: the program
     * asked for application cursor keys (arrows as `Esc O A`).
     */
    fun bytes(key: String, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false, appCursor: Boolean = false): ByteArray? {
        val k = if (key.length == 1) key else key.lowercase()
        // xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
        val m = 1 + (if (shift) 1 else 0) + (if (alt) 2 else 0) + (if (ctrl) 4 else 0)
        fun s(text: String) = text.toByteArray()
        fun csi(final: String) = if (m == 1) s("$ESC[$final") else s("$ESC[1;$m$final")
        fun tilde(n: Int) = if (m == 1) s("$ESC[$n~") else s("$ESC[$n;$m~")
        val altPrefix = if (alt) ESC else ""
        val arrows = mapOf("up" to "A", "down" to "B", "right" to "C", "left" to "D", "home" to "H", "end" to "F")
        arrows[k]?.let { f ->
            return if (m == 1) s(ESC + (if (appCursor) "O" else "[") + f) else csi(f)
        }
        when (k) {
            "pageup" -> return tilde(5)
            "pagedown" -> return tilde(6)
            "insert" -> return tilde(2)
            "delete" -> return tilde(3)
            "tab" -> return if (shift && !ctrl && !alt) s("$ESC[Z") else s(altPrefix + "\t")
            "esc" -> return s(altPrefix + ESC)
            "enter" -> return s(altPrefix + "\r")
            "backspace" -> return s(altPrefix) + byteArrayOf(if (ctrl) 0x08 else 0x7f)
            "space" -> return s(altPrefix) + byteArrayOf(if (ctrl) 0x00 else 0x20)
        }
        if (k.startsWith("f")) {
            val n = k.drop(1).toIntOrNull()
            if (n != null && n in 1..24) {
                if (n <= 4) {
                    val final = listOf("P", "Q", "R", "S")[n - 1]
                    return if (m == 1) s("${ESC}O$final") else s("$ESC[1;$m$final")
                }
                // F5 is 15, then gaps where the old terminals' keys were.
                val codes = listOf(15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34, 42, 43, 44, 45)
                return tilde(codes[n - 5])
            }
        }
        if (k.codePointCount(0, k.length) != 1) return null
        var out = s(if (shift) k.uppercase() else k)
        if (ctrl) {
            if (out.size != 1) return null
            out = byteArrayOf(TerminalSession.control(out[0]) ?: return null)
        }
        if (alt) out = s(ESC) + out
        return out
    }

    /** How a named key or a character is shown on a small key. */
    fun title(key: String): String = when (val k = if (key.length == 1) key else key.lowercase()) {
        "esc" -> "Esc"
        "tab" -> "Tab"
        "enter" -> "⏎"
        "backspace" -> "⌫"
        "delete" -> "Del"
        "insert" -> "Ins"
        "space" -> "␣"
        "up" -> "↑"
        "down" -> "↓"
        "left" -> "←"
        "right" -> "→"
        "home" -> "Home"
        "end" -> "End"
        "pageup" -> "PgUp"
        "pagedown" -> "PgDn"
        else -> if (k.startsWith("f") && k.drop(1).toIntOrNull() != null) k.uppercase() else key.uppercase()
    }
}
