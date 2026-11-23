package dev.detour.android.keys

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** A button of the key bar or of the keys keyboard. */
data class BarKey(
    val kind: Kind,
    val label: String = "",
    val value: String = "",
    val id: String = UUID.randomUUID().toString(),
) {
    enum class Kind(val raw: String) {
        Esc("esc"), Tab("tab"), Ctrl("ctrl"), Alt("alt"), Up("up"), Down("down"), Left("left"), Right("right"),
        Home("home"), End("end"), PageUp("pageUp"), PageDown("pageDown"), Enter("enter"), Backspace("backspace"),
        /** Types `value` (`\n` in it is Enter). */
        Text("text"),
        /** A combination: `ctrl+c`, `alt+b`, `shift+tab`, `ctrl+alt+left`. */
        Combo("combo"),
        /** A named key (`f5`, `insert`, `space`, see `KeyCodes.named`) or one character (`|`). */
        Key("key"),
        /** A snippet: `label` is its name, `value` its command, typed and run with Enter. */
        Snippet("snippet");

        companion object { fun of(raw: String?) = entries.firstOrNull { it.raw == raw } }
    }

    data class Combo(val ctrl: Boolean, val alt: Boolean, val shift: Boolean, val key: String)

    /** Modifiers and the key of a combination. */
    val comboParts: Combo?
        get() {
            if (kind != Kind.Combo) return null
            val parts = value.split("+").map { it.trim() }
            // "ctrl++" is Ctrl and the plus key.
            val key = if (value.endsWith("++")) "+" else parts.lastOrNull() ?: ""
            val mods = parts.dropLast(1).map { it.lowercase() }.toSet()
            if (key.isEmpty()) return null
            return Combo("ctrl" in mods, "alt" in mods || "meta" in mods, "shift" in mods, key)
        }

    /** What the button shows, short enough for a small square key. */
    val title: String
        get() {
            if (label.isNotEmpty()) return label
            return when (kind) {
                Kind.Esc -> "Esc"
                Kind.Tab -> "Tab"
                Kind.Ctrl -> "Ctrl"
                Kind.Alt -> "Alt"
                Kind.Up -> "↑"
                Kind.Down -> "↓"
                Kind.Left -> "←"
                Kind.Right -> "→"
                Kind.Home -> "Home"
                Kind.End -> "End"
                Kind.PageUp -> "PgUp"
                Kind.PageDown -> "PgDn"
                Kind.Enter -> "⏎"
                Kind.Backspace -> "⌫"
                Kind.Text, Kind.Snippet -> value
                Kind.Key -> KeyCodes.title(value)
                Kind.Combo -> comboParts?.let { c ->
                    (if (c.ctrl) "^" else "") + (if (c.alt) "⌥" else "") + (if (c.shift) "⇧" else "") + KeyCodes.title(c.key)
                } ?: value
            }
        }

    /** The bytes it sends; null for Ctrl and Alt, which apply to the next key. */
    fun bytes(appCursor: Boolean = false): ByteArray? {
        val named = mapOf(
            Kind.Esc to "esc", Kind.Tab to "tab", Kind.Up to "up", Kind.Down to "down", Kind.Left to "left", Kind.Right to "right",
            Kind.Home to "home", Kind.End to "end", Kind.PageUp to "pageup", Kind.PageDown to "pagedown", Kind.Enter to "enter", Kind.Backspace to "backspace",
        )
        named[kind]?.let { return KeyCodes.bytes(it, appCursor = appCursor) }
        return when (kind) {
            Kind.Text -> value.replace("\\n", "\r").replace("\n", "\r").toByteArray()
            Kind.Snippet -> (value.trim('\n', '\r').replace("\r\n", "\r").replace("\n", "\r") + "\r").toByteArray()
            Kind.Key -> KeyCodes.bytes(value, appCursor = appCursor)
            Kind.Combo -> comboParts?.let { KeyCodes.bytes(it.key, it.ctrl, it.alt, it.shift, appCursor) }
            else -> null
        }
    }

    fun json(): JSONObject = JSONObject().put("kind", kind.raw).put("label", label).put("value", value).put("id", id)

    companion object {
        fun from(o: JSONObject): BarKey? = Kind.of(o.optString("kind"))?.let {
            BarKey(it, o.optString("label"), o.optString("value"), o.optString("id").ifEmpty { UUID.randomUUID().toString() })
        }

        /** Whether a combination's text is one Detour can send. */
        fun combo(text: String): ByteArray? = BarKey(Kind.Combo, value = text).bytes()
    }
}

/** The bar's keys and whether it shows, kept on this device. */
object KeyBarStore {
    private lateinit var context: Context
    var keys by mutableStateOf(defaults)
        private set
    var hidden by mutableStateOf(false)
        private set

    fun load(context: Context) {
        this.context = context
        val text = context.getSharedPreferences("keys", Context.MODE_PRIVATE).getString("keyBar", null) ?: return
        runCatching {
            val o = JSONObject(text)
            val a = o.optJSONArray("keys") ?: JSONArray()
            keys = (0 until a.length()).mapNotNull { BarKey.from(a.getJSONObject(it)) }
            hidden = o.optBoolean("hidden")
        }
    }

    fun set(keys: List<BarKey>) { this.keys = keys; save() }
    fun hide(on: Boolean) { hidden = on; save() }
    fun reset() { keys = defaults; hidden = false; save() }

    private fun save() {
        val o = JSONObject().put("keys", JSONArray(keys.map { it.json() })).put("hidden", hidden)
        context.getSharedPreferences("keys", Context.MODE_PRIVATE).edit().putString("keyBar", o.toString()).apply()
    }

    val defaults: List<BarKey>
        get() = listOf(
            BarKey(BarKey.Kind.Esc), BarKey(BarKey.Kind.Ctrl), BarKey(BarKey.Kind.Tab), BarKey(BarKey.Kind.Left), BarKey(BarKey.Kind.Down), BarKey(BarKey.Kind.Up), BarKey(BarKey.Kind.Right),
            BarKey(BarKey.Kind.Text, value = "|"), BarKey(BarKey.Kind.Text, value = "~"), BarKey(BarKey.Kind.Text, value = "/"), BarKey(BarKey.Kind.Text, value = "-"),
            BarKey(BarKey.Kind.Alt), BarKey(BarKey.Kind.Home), BarKey(BarKey.Kind.End), BarKey(BarKey.Kind.PageUp), BarKey(BarKey.Kind.PageDown),
            BarKey(BarKey.Kind.Combo, value = "ctrl+c"), BarKey(BarKey.Kind.Combo, value = "ctrl+d"), BarKey(BarKey.Kind.Combo, value = "ctrl+z"), BarKey(BarKey.Kind.Combo, value = "ctrl+r"),
        )
}
