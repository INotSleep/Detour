package dev.detour.android.core

import android.content.Context
import java.util.Locale

/**
 * The app's text in the person's language: the iOS app's Localizable.strings, shared (English
 * is the key itself). `%@` there stands for a value, as `%s` does here.
 */
object L10n {
    private var table: Map<String, String> = emptyMap()

    fun load(context: Context) {
        val locale = Locale.getDefault()
        val names = buildList {
            if (locale.language == "zh") add("zh-Hans")
            if (locale.country.isNotEmpty()) add("${locale.language}-${locale.country}")
            add(locale.language)
        }
        val dir = names.firstOrNull { runCatching { context.assets.open("$it.lproj/Localizable.strings").close() }.isSuccess }
        table = dir?.let { parse(context.assets.open("$it.lproj/Localizable.strings").bufferedReader().readText()) } ?: emptyMap()
    }

    private val line = Regex("^\\s*\"((?:[^\"\\\\]|\\\\.)*)\"\\s*=\\s*\"((?:[^\"\\\\]|\\\\.)*)\"\\s*;")

    private fun parse(text: String): Map<String, String> {
        val out = HashMap<String, String>()
        for (l in text.lineSequence()) {
            val m = line.find(l) ?: continue
            out[unescape(m.groupValues[1])] = unescape(m.groupValues[2])
        }
        return out
    }

    /** `\"`, `\\` and `\n` as the .strings format writes them. */
    private fun unescape(s: String): String {
        val out = StringBuilder()
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c == '\\' && i + 1 < s.length) {
                when (val n = s[i + 1]) {
                    'n' -> out.append('\n')
                    't' -> out.append('\t')
                    else -> out.append(n)
                }
                i += 2
            } else {
                out.append(c)
                i++
            }
        }
        return out.toString()
    }

    fun get(key: String): String = table[key] ?: key

    /** With values: `%@` and `%1$@` are filled in order. */
    fun format(key: String, vararg args: Any?): String =
        get(key).replace(Regex("%(\\d+\\$)?@"), "%$1s").let { runCatching { String.format(it, *args) }.getOrDefault(it) }
}

/** The text for `key` in the person's language. */
fun t(key: String): String = L10n.get(key)
fun t(key: String, vararg args: Any?): String = L10n.format(key, *args)
